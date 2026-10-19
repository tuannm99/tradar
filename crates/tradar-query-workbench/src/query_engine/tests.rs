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

#[test]
fn build_screen_applies_the_configured_vim_mode() {
    let engine = engine(Arc::new(FakeDriver {
        result: QueryResult::Documents {
            items: Vec::new(),
            truncated: false,
        },
    }));
    let (tx, _rx) = mpsc::unbounded_channel();

    let mut screen = Box::new(engine).build_screen(tx, None);

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| screen.draw(frame, frame.area()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    // The editor's own mode badge shows only when it's vim-enabled --
    // whatever `tradar_core::config::vim_mode()` says right now (this
    // test doesn't control it, just checks `build_screen` actually
    // wired the setting through rather than leaving the editor on its
    // own `new()` default of vim-on).
    assert_eq!(
        text.contains("NORMAL") || text.contains("INSERT"),
        tradar_core::config::vim_mode(),
        "buffer was: {text}"
    );
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
            _columns: &[String],
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
        engine.crud_snippet("users", tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM users".to_string())
    );
}

#[test]
fn crud_snippet_is_none_for_an_entry_not_in_the_schema() {
    let engine = engine(Arc::new(FailingDriver));

    assert_eq!(
        engine.crud_snippet("ghost", tradar_core::action::CrudOp::Read, &[]),
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
        engine.crud_snippet("users", tradar_core::action::CrudOp::Read, &[]),
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
