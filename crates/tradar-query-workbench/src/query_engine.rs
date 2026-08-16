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

use tradar_connector_spi::Session;
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

/// How often the results pane's running-query spinner glyph advances --
/// shared with `components::results`'s own spinner-frame calculation so
/// `tick()` knows the coarsest redraw cadence that still animates it
/// smoothly, rather than requesting a redraw on every ~50ms input-poll
/// cycle regardless of whether the glyph would actually look different.
pub(crate) const SPINNER_FRAME_MS: u128 = 80;

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
    /// When the in-flight query was submitted, for the results pane's
    /// elapsed-time readout. `tokio::time::Instant` for the same reason as
    /// `last_ping` -- a paused clock in tests advances it.
    running_since: Option<tokio::time::Instant>,
    /// Which `SPINNER_FRAME_MS`-sized tick of `running_since` was last
    /// reported as a change, so `tick()` only asks for a redraw when the
    /// spinner glyph would actually look different -- not on every ~50ms
    /// input-poll cycle it happens to be called on while a query runs.
    last_reported_spinner_frame: Option<u128>,
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
            running_since: None,
            last_reported_spinner_frame: None,
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

    /// How long the in-flight query has been running, for the results
    /// pane's elapsed-time readout. `None` when nothing is running.
    pub fn elapsed_running(&self) -> Option<std::time::Duration> {
        self.running_since.map(|since| since.elapsed())
    }

    /// Whether the most recent background ping succeeded -- see
    /// `PING_INTERVAL`. Not query-side: an editor left idle for minutes with
    /// a dropped connection still shows this without the user having to run
    /// anything.
    pub fn alive(&self) -> bool {
        self.alive
    }

    /// Whether this driver currently holds an open transaction -- see
    /// `QueryDriver::in_transaction`.
    pub fn in_transaction(&self) -> bool {
        self.driver.in_transaction()
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

    /// A skeleton statement for `op` against the schema entry named
    /// `name` -- see `Component::crud_snippet`. `None` when `name` isn't a
    /// known entry (schema failed to load, or it's stale) or the driver
    /// has nothing to say for it.
    pub fn crud_snippet(&self, name: &str, op: tradar_core::action::CrudOp) -> Option<String> {
        let schema = self.schema.as_ref().ok()?;
        let entry = schema.iter().find(|entry| entry.name == name)?;
        self.driver.crud_snippet(entry, op)
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
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;
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

    /// Spawns `driver.browse_entry(&entry)` -- the browse sidebar's Enter
    /// action, reported back through the same outcome channel/epoch as
    /// `submit_query`. Unlike `submit_query`, this does **not** push into
    /// `history`: a sidebar click isn't a command the user typed and might
    /// want to recall with Ctrl+R.
    /// The literal command `submit_browse(entry)` will run, for the browse
    /// sidebar to echo -- see `QueryDriver::browse_command`.
    pub fn browse_command(&self, entry: &SchemaInfo) -> Option<String> {
        self.driver.browse_command(entry)
    }

    pub fn submit_browse(&mut self, entry: SchemaInfo) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.pending = true;
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;

        let driver = Arc::clone(&self.driver);
        let tx = self.outcome_tx.clone();
        self.running = Some(tokio::spawn(async move {
            let outcome = match driver.browse_entry(&entry).await {
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
        self.running_since = Some(tokio::time::Instant::now());
        self.last_reported_spinner_frame = None;
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
        self.running_since = None;
        self.last_reported_spinner_frame = None;
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
                    self.running_since = None;
                    self.last_reported_spinner_frame = None;
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

        // Redraw while a query is running, not just when an outcome lands
        // -- the results pane's spinner/elapsed-time readout has nothing
        // else to drive its animation. Gated on the spinner's own frame
        // advancing (not every tick) so a slow query doesn't force a
        // redraw on every ~50ms input-poll cycle when the glyph would
        // look identical to the last one drawn.
        if self.pending
            && let Some(since) = self.running_since
        {
            let frame = since.elapsed().as_millis() / SPINNER_FRAME_MS;
            if self.last_reported_spinner_frame != Some(frame) {
                self.last_reported_spinner_frame = Some(frame);
                changed = true;
            }
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

    #[tokio::test(start_paused = true)]
    async fn tick_only_reports_changed_when_the_spinner_frame_actually_advances() {
        let mut engine = engine(Arc::new(HangingDriver));
        engine.submit_query("SELECT pg_sleep(3600)".to_string());

        // The very first tick after submitting always sees a fresh frame.
        assert!(engine.tick());

        // Still well inside the same SPINNER_FRAME_MS-wide frame -- nothing
        // for the spinner to redraw.
        tokio::time::advance(std::time::Duration::from_millis(30)).await;
        assert!(!engine.tick(), "the spinner glyph hasn't moved yet");

        // Past the next frame boundary -- now it has.
        tokio::time::advance(std::time::Duration::from_millis(60)).await;
        assert!(engine.tick(), "the spinner glyph should have advanced");
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

    /// A driver that overrides `browse_entry`, to exercise `submit_browse`
    /// without a real backend.
    struct BrowsableDriver {
        result: QueryResult,
    }

    #[async_trait]
    impl QueryDriver for BrowsableDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            unreachable!("submit_browse must call browse_entry, not execute")
        }
        async fn browse_entry(&self, _entry: &SchemaInfo) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn submit_browse_reports_completed_via_browse_entry() {
        let result = QueryResult::Table {
            columns: vec!["field".to_string(), "value".to_string()],
            rows: vec![vec!["name".to_string(), "Ada".to_string()]],
            truncated: false,
        };
        let mut engine = engine(Arc::new(BrowsableDriver {
            result: result.clone(),
        }));

        engine.submit_browse(SchemaInfo::new("user:1"));
        assert!(engine.is_pending());
        tick_until_settled(&mut engine).await;

        match engine.take_outcome() {
            Some(QueryOutcome::Completed { result: got }) => assert_eq!(got, result),
            _ => panic!("expected a Completed outcome"),
        }
    }

    #[tokio::test]
    async fn submit_browse_does_not_append_to_history() {
        let mut engine = engine(Arc::new(BrowsableDriver {
            result: QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
        }));

        engine.submit_browse(SchemaInfo::new("user:1"));
        tick_until_settled(&mut engine).await;

        assert!(engine.history().is_empty());
    }

    #[test]
    fn crud_snippet_delegates_to_the_driver_for_a_known_entry() {
        struct SnippetDriver;

        #[async_trait]
        impl QueryDriver for SnippetDriver {
            async fn connect(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
                Ok(Vec::new())
            }
            async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
                unreachable!()
            }
            fn crud_snippet(
                &self,
                entry: &SchemaInfo,
                _op: tradar_core::action::CrudOp,
            ) -> Option<String> {
                Some(format!("SELECT * FROM {}", entry.name))
            }
        }

        let engine = QueryEngine::new(
            Arc::new(SnippetDriver),
            connection(),
            Ok(vec![SchemaInfo::new("users")]),
        );

        assert_eq!(
            engine.crud_snippet("users", tradar_core::action::CrudOp::Read),
            Some("SELECT * FROM users".to_string())
        );
    }

    #[test]
    fn crud_snippet_is_none_for_an_entry_not_in_the_schema() {
        let engine = engine(Arc::new(FailingDriver));

        assert_eq!(
            engine.crud_snippet("ghost", tradar_core::action::CrudOp::Read),
            None
        );
    }

    #[test]
    fn crud_snippet_is_none_when_the_schema_failed_to_load() {
        let engine = QueryEngine::new(
            Arc::new(FailingDriver),
            connection(),
            Err("scan failed".to_string()),
        );

        assert_eq!(
            engine.crud_snippet("users", tradar_core::action::CrudOp::Read),
            None
        );
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
