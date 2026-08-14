//! Takes a query string, hands it to the active `QueryDriver`, and
//! normalizes the result for the TUI to render. No database-specific logic
//! lives here. Implements `Session`: `submit_query` is the synchronous
//! command a `Screen` calls, which spawns the actual driver IO and reports
//! back through an internal channel that `tick()` drains. `tick()` also
//! fires a background `QueryDriver::ping()` every `PING_INTERVAL`, so
//! `alive()` reflects a dropped connection without the user having to run a
//! query into it first.

use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use tradar_connector_api::Session;
use tradar_core::action::{Action, Component};
use tradar_core::storage::SavedConnection;

use crate::components::query_screen::QueryScreenComponent;
use crate::query_driver::{QueryDriver, QueryResult, SchemaInfo};

/// Outcomes are drained at most this many at a time per `tick()` call, so a
/// connector that somehow floods this channel can never starve rendering.
/// In practice at most one outcome is ever in flight per `QueryEngine`
/// (there is no query pipelining), but the bound is kept for consistency
/// with the rest of the `Session::tick()` contract.
const MAX_DRAIN_PER_TICK: usize = 64;

/// How often `tick()` fires a background `QueryDriver::ping()`. The TUI
/// otherwise has no way to notice a connection dropped except a query
/// failing -- see "Trạng thái connection" in docs/backlog.md. Fifteen
/// seconds is often enough to catch a drop within the time it'd take to
/// notice by hand, and rare enough not to matter against a database with
/// connection logging or a low idle-connection limit.
const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

pub enum QueryOutcome {
    Completed { result: QueryResult },
    Failed { error: String },
}

struct TaggedOutcome {
    epoch: u64,
    outcome: QueryOutcome,
}

pub struct QueryEngine {
    driver: Arc<dyn QueryDriver>,
    connection: SavedConnection,
    schema: Result<Vec<SchemaInfo>, String>,
    history: Vec<String>,
    epoch: u64,
    pending: bool,
    /// The in-flight query's task, kept so it can be aborted. `None`
    /// whenever nothing is running.
    running: Option<tokio::task::JoinHandle<()>>,
    last_outcome: Option<QueryOutcome>,
    outcome_tx: UnboundedSender<TaggedOutcome>,
    outcome_rx: UnboundedReceiver<TaggedOutcome>,
    /// Whether the last background ping succeeded. Starts `true`: the
    /// connect that produced this engine already proved the backend was
    /// reachable, so there is nothing to doubt until a ping actually fails.
    alive: bool,
    /// `tokio::time::Instant` rather than `std::time::Instant` so a paused
    /// clock in tests advances it -- see the ping tests below.
    last_ping: tokio::time::Instant,
    ping_in_flight: bool,
    ping_tx: UnboundedSender<bool>,
    ping_rx: UnboundedReceiver<bool>,
}

impl QueryEngine {
    pub fn new(
        driver: Arc<dyn QueryDriver>,
        connection: SavedConnection,
        schema: Result<Vec<SchemaInfo>, String>,
    ) -> Self {
        let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
        let (ping_tx, ping_rx) = mpsc::unbounded_channel();
        Self {
            driver,
            connection,
            schema,
            history: Vec::new(),
            epoch: 0,
            pending: false,
            running: None,
            last_outcome: None,
            outcome_tx,
            outcome_rx,
            alive: true,
            last_ping: tokio::time::Instant::now(),
            ping_in_flight: false,
            ping_tx,
            ping_rx,
        }
    }

    pub fn connection(&self) -> &SavedConnection {
        &self.connection
    }

    pub fn schema(&self) -> &Result<Vec<SchemaInfo>, String> {
        &self.schema
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Whether the most recent background ping succeeded -- see
    /// `PING_INTERVAL`. Not query-side: an editor left idle for minutes with
    /// a dropped connection still shows this without the user having to run
    /// anything.
    pub fn alive(&self) -> bool {
        self.alive
    }

    /// Fires a `ping()` in the background, unless one is already in flight.
    /// Guarded by `ping_in_flight` rather than firing unconditionally on
    /// every interval tick, so a slow or hanging backend can't pile up
    /// concurrent pings.
    fn fire_ping(&mut self) {
        self.ping_in_flight = true;
        self.last_ping = tokio::time::Instant::now();
        let driver = Arc::clone(&self.driver);
        let tx = self.ping_tx.clone();
        tokio::spawn(async move {
            let alive = driver.ping().await.is_ok();
            let _ = tx.send(alive);
        });
    }

    /// The statements in `text`, per this driver's own rules -- see
    /// `QueryDriver::split_statements`.
    pub fn split_statements(&self, text: &str) -> Vec<crate::query_driver::Statement> {
        self.driver.split_statements(text)
    }

    /// The driver's own completion vocabulary -- see
    /// `QueryDriver::keywords`.
    pub fn keywords(&self) -> &'static [&'static str] {
        self.driver.keywords()
    }

    pub fn export_curl(&self, query: &str) -> Option<String> {
        self.driver.export_curl(query)
    }

    /// The table `query` reads from, when this driver can say -- see
    /// `QueryDriver::edit_source`.
    pub fn edit_source(&self, query: &str) -> Option<String> {
        self.driver.edit_source(query)
    }

    /// This driver's statement for `edit` -- see `QueryDriver::edit_sql`.
    pub fn edit_sql(&self, edit: &crate::query_driver::RowEdit) -> Option<String> {
        self.driver.edit_sql(edit)
    }

    /// Spawns the actual query execution and returns immediately -- the
    /// synchronous command a `Screen` calls from `handle_key_event`. Bumps
    /// an internal epoch so a reply from a superseded call (unreachable
    /// today since queries don't pipeline, but kept for the same reason as
    /// `MAX_DRAIN_PER_TICK`) is dropped instead of overwriting a newer one.
    pub fn submit_query(&mut self, query: String) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        self.history.push(query.clone());

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let outcome = match driver.execute(&query).await {
                Ok(result) => QueryOutcome::Completed { result },
                Err(e) => QueryOutcome::Failed {
                    error: e.to_string(),
                },
            };
            let _ = tx.send(TaggedOutcome { epoch, outcome });
        }));
    }

    /// Runs `statements` one after another, reporting the last result --
    /// or the first failure, since carrying on after an error would apply
    /// half a script and hide which half.
    pub fn submit_all(&mut self, statements: Vec<String>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        for statement in &statements {
            self.history.push(statement.clone());
        }

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let total = statements.len();
            let mut last = None;
            let mut failure = None;
            for (index, statement) in statements.into_iter().enumerate() {
                match driver.execute(&statement).await {
                    Ok(result) => last = Some(result),
                    Err(e) => {
                        failure = Some(format!("statement {} of {total} failed: {e}", index + 1));
                        break;
                    }
                }
            }
            let outcome = match (failure, last) {
                (Some(error), _) => QueryOutcome::Failed { error },
                (None, Some(result)) => QueryOutcome::Completed { result },
                (None, None) => QueryOutcome::Failed {
                    error: "nothing to run".to_string(),
                },
            };
            let _ = tx.send(TaggedOutcome { epoch, outcome });
        }));
    }

    /// Abandons the running query, if any. Aborting the task drops the
    /// driver future, which is what closes the statement on the backend for
    /// a driver that supports it; the epoch bump means a reply that was
    /// already in flight is ignored rather than landing after the fact.
    pub fn cancel(&mut self) -> bool {
        let Some(handle) = self.running.take() else {
            return false;
        };
        handle.abort();
        self.epoch += 1;
        self.pending = false;
        self.last_outcome = None;
        true
    }

    /// The most recent outcome for the in-flight query, if `tick()` picked
    /// one up since the last call. Consumes it -- calling twice in a row
    /// without an intervening `tick()` returns `None` the second time.
    pub fn take_outcome(&mut self) -> Option<QueryOutcome> {
        self.last_outcome.take()
    }
}

impl Session for QueryEngine {
    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            match self.outcome_rx.try_recv() {
                Ok(tagged) if tagged.epoch == self.epoch => {
                    self.pending = false;
                    self.running = None;
                    self.last_outcome = Some(tagged.outcome);
                    changed = true;
                }
                Ok(_stale) => {}
                Err(_) => break,
            }
        }

        // At most one ping is ever in flight, so a single non-blocking
        // check is enough -- unlike the outcome channel above, there's
        // nothing to drain in a loop.
        if let Ok(alive) = self.ping_rx.try_recv() {
            self.ping_in_flight = false;
            if alive != self.alive {
                self.alive = alive;
                changed = true;
            }
        }
        if !self.ping_in_flight && self.last_ping.elapsed() >= PING_INTERVAL {
            self.fire_ping();
        }

        changed
    }

    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        restore: Option<&str>,
    ) -> Box<dyn Component> {
        let mut screen = QueryScreenComponent::new(*self, action_tx);
        if let Some(text) = restore {
            screen.query_editor.set_text(text);
        }
        Box::new(screen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeDriver {
        result: QueryResult,
    }

    #[async_trait]
    impl QueryDriver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
    }

    struct FailingDriver;

    #[async_trait]
    impl QueryDriver for FailingDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Err(anyhow::anyhow!("syntax error"))
        }
    }

    fn connection() -> SavedConnection {
        SavedConnection {
            name: "test".to_string(),
            driver: "sqlite".to_string(),
            target: "test.db".to_string(),
        }
    }

    fn engine(driver: Arc<dyn QueryDriver>) -> QueryEngine {
        QueryEngine::new(driver, connection(), Ok(Vec::new()))
    }

    /// Ticks `engine` until it stops being pending, yielding to let the
    /// spawned query-execution task run. Bounded so a real bug (the outcome
    /// never arriving) fails the test instead of hanging it.
    async fn tick_until_settled(engine: &mut QueryEngine) {
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            engine.tick();
            if !engine.is_pending() {
                return;
            }
        }
        panic!("engine is still pending after 10,000 ticks");
    }

    #[tokio::test]
    async fn submit_query_reports_completed_once_the_task_settles() {
        let result = QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        };
        let mut engine = engine(Arc::new(FakeDriver {
            result: result.clone(),
        }));

        engine.submit_query("SELECT id FROM users".to_string());
        assert!(engine.is_pending());
        tick_until_settled(&mut engine).await;

        match engine.take_outcome() {
            Some(QueryOutcome::Completed { result: got }) => assert_eq!(got, result),
            _ => panic!("expected a Completed outcome"),
        }
    }

    #[tokio::test]
    async fn submit_query_reports_failed_when_the_driver_errors() {
        let mut engine = engine(Arc::new(FailingDriver));

        engine.submit_query("SELECT 1".to_string());
        tick_until_settled(&mut engine).await;

        match engine.take_outcome() {
            Some(QueryOutcome::Failed { error }) => assert_eq!(error, "syntax error"),
            _ => panic!("expected a Failed outcome"),
        }
    }

    #[tokio::test]
    async fn take_outcome_only_returns_a_fresh_outcome_once() {
        let mut engine = engine(Arc::new(FakeDriver {
            result: QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
        }));

        engine.submit_query("SELECT 1".to_string());
        tick_until_settled(&mut engine).await;

        assert!(engine.take_outcome().is_some());
        assert!(
            engine.take_outcome().is_none(),
            "a second take without an intervening tick must find nothing new"
        );
    }

    /// A driver that never finishes, so a cancel has something real to
    /// interrupt.
    struct HangingDriver;

    #[async_trait]
    impl QueryDriver for HangingDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            std::future::pending::<()>().await;
            unreachable!("this driver never completes")
        }
    }

    #[tokio::test]
    async fn cancel_stops_waiting_on_a_query_that_never_finishes() {
        let mut engine = engine(Arc::new(HangingDriver));
        engine.submit_query("SELECT pg_sleep(3600)".to_string());
        assert!(engine.is_pending());

        let cancelled = engine.cancel();

        assert!(cancelled);
        assert!(!engine.is_pending(), "nothing is running any more");
        for _ in 0..100 {
            tokio::task::yield_now().await;
            engine.tick();
        }
        assert!(engine.take_outcome().is_none(), "no late result may land");
    }

    #[tokio::test]
    async fn cancelling_with_nothing_running_is_a_no_op() {
        let mut engine = engine(Arc::new(HangingDriver));

        assert!(!engine.cancel());
    }

    #[tokio::test]
    async fn a_result_in_flight_when_cancel_lands_is_discarded() {
        let mut engine = engine(Arc::new(FakeDriver {
            result: QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
        }));
        engine.submit_query("SELECT 1".to_string());

        // Cancel before draining: the task may well have completed and
        // queued its outcome already, and that reply must not resurface.
        engine.cancel();
        for _ in 0..100 {
            tokio::task::yield_now().await;
            engine.tick();
        }

        assert!(engine.take_outcome().is_none());
        assert!(!engine.is_pending());
    }

    #[tokio::test]
    async fn submit_query_appends_to_history() {
        let mut engine = engine(Arc::new(FakeDriver {
            result: QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
        }));

        engine.submit_query("SELECT 1".to_string());
        tick_until_settled(&mut engine).await;
        engine.submit_query("SELECT 2".to_string());
        tick_until_settled(&mut engine).await;

        assert_eq!(engine.history(), &["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn connection_and_schema_return_what_was_passed_to_new() {
        let engine = QueryEngine::new(
            Arc::new(FailingDriver),
            connection(),
            Err("scan failed".to_string()),
        );

        assert_eq!(engine.connection(), &connection());
        assert_eq!(engine.schema(), &Err("scan failed".to_string()));
    }

    #[test]
    fn export_curl_defaults_to_none_when_the_driver_does_not_support_it() {
        let engine = engine(Arc::new(FailingDriver));

        assert_eq!(engine.export_curl("SELECT 1"), None);
    }

    #[test]
    fn a_fresh_engine_is_assumed_alive() {
        let engine = engine(Arc::new(FailingDriver));

        assert!(
            engine.alive(),
            "connect() already proved reachability, no ping has run yet"
        );
    }

    /// A driver whose `ping()` outcome is controlled from outside, to
    /// exercise the alive/dead transition without a real backend.
    struct PingableDriver {
        alive: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl QueryDriver for PingableDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(QueryResult::Affected { rows: 0 })
        }
        async fn ping(&self) -> anyhow::Result<()> {
            if self.alive.load(std::sync::atomic::Ordering::SeqCst) {
                Ok(())
            } else {
                anyhow::bail!("connection reset by peer")
            }
        }
    }

    /// Drains `engine`'s ping outcome, yielding to let the spawned ping
    /// task actually run. Bounded so a real bug (the outcome never
    /// arriving) fails the test instead of hanging it.
    async fn tick_until_ping_settles(engine: &mut QueryEngine) {
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            if !engine.ping_in_flight {
                engine.tick();
                return;
            }
            engine.tick();
        }
        panic!("ping never settled after 10,000 ticks");
    }

    #[tokio::test(start_paused = true)]
    async fn no_ping_fires_before_the_interval_elapses() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut engine = engine(Arc::new(PingableDriver {
            alive: flag.clone(),
        }));

        for _ in 0..50 {
            tokio::task::yield_now().await;
            engine.tick();
        }

        assert!(
            engine.alive(),
            "the backend is failing, but no ping should have run yet"
        );
        assert!(!engine.ping_in_flight);
    }

    #[tokio::test(start_paused = true)]
    async fn a_background_ping_flips_alive_to_false_once_the_backend_drops() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut engine = engine(Arc::new(PingableDriver {
            alive: flag.clone(),
        }));
        assert!(engine.alive());

        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        tokio::time::advance(PING_INTERVAL + std::time::Duration::from_millis(1)).await;
        engine.tick(); // fires the ping
        tick_until_ping_settles(&mut engine).await;

        assert!(
            !engine.alive(),
            "the periodic ping should have noticed the drop"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn alive_recovers_once_a_later_ping_succeeds_again() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut engine = engine(Arc::new(PingableDriver {
            alive: flag.clone(),
        }));
        tokio::time::advance(PING_INTERVAL + std::time::Duration::from_millis(1)).await;
        engine.tick();
        tick_until_ping_settles(&mut engine).await;
        assert!(!engine.alive());

        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        tokio::time::advance(PING_INTERVAL + std::time::Duration::from_millis(1)).await;
        engine.tick();
        tick_until_ping_settles(&mut engine).await;

        assert!(engine.alive());
    }

    #[tokio::test(start_paused = true)]
    async fn a_ping_in_flight_is_not_duplicated_by_a_later_tick() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut engine = engine(Arc::new(PingableDriver {
            alive: flag.clone(),
        }));
        tokio::time::advance(PING_INTERVAL + std::time::Duration::from_millis(1)).await;

        engine.tick();
        assert!(engine.ping_in_flight);
        // Time has not moved past this point, so a second tick must not
        // fire a second ping on top of the one already in flight.
        engine.tick();

        tick_until_ping_settles(&mut engine).await;
        assert!(engine.alive());
    }
}
