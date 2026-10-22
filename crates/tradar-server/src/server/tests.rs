use super::*;
use crate::protocol::PROTOCOL_VERSION;

#[test]
fn ddl_is_recognised_behind_leading_comments() {
    assert!(is_ddl("CREATE TABLE t (id INT)"));
    assert!(is_ddl("  drop table t"));
    assert!(is_ddl("-- tradar: demo\nCREATE TABLE t (id INT)"));
    assert!(is_ddl(
        "/* make it */ -- and more\n  ALTER TABLE t ADD c INT"
    ));
    assert!(is_ddl("db.createCollection('x')"));
    assert!(is_ddl("db.users.insertMany([{a: 1}])"));
    assert!(!is_ddl("db.users.find({})"));
    assert!(!is_ddl("-- CREATE is only mentioned here\nSELECT 1"));
    assert!(!is_ddl("SELECT 1"));
    assert!(!is_ddl("-- only a comment"));
}

/// No connector feature needed -- `version` is one of the handful of
/// methods (`connectors.list`, `shutdown`, ...) that answer regardless
/// of which connectors this build compiled in.
#[tokio::test]
async fn version_reports_the_compiled_in_protocol_version() {
    let server = Server::new(HashMap::new(), None);

    let response = server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": "version"}),
            &ClientConnection::detached(),
        )
        .await;

    assert_eq!(response["result"]["major"], PROTOCOL_VERSION.0);
    assert_eq!(response["result"]["minor"], PROTOCOL_VERSION.1);
    // It names the running binary, and a server that has not been
    // rebuilt since it started is not stale.
    assert!(response["result"]["exe"].is_string());
    assert_eq!(response["result"]["stale"], false);
}

#[tokio::test]
async fn connections_path_reports_the_store_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.toml");
    let server = Server::new(
        HashMap::new(),
        Some(tradar_core::storage::ConnectionStore::at(path.clone())),
    );
    async fn call(server: &Server) -> Value {
        server
            .handle(
                json!({"jsonrpc": "2.0", "id": 1, "method": "connections.path"}),
                &ClientConnection::detached(),
            )
            .await
    }
    assert_eq!(
        call(&server).await["result"]["path"],
        path.display().to_string()
    );
    assert!(call(&Server::new(HashMap::new(), None)).await["result"]["path"].is_null());
}

#[test]
fn idle_ttl_accepts_seconds_and_suffixes_and_can_be_turned_off() {
    use std::time::Duration;
    let secs = |n| Some(Duration::from_secs(n));
    assert_eq!(idle::parse_ttl(None), Some(idle::DEFAULT_IDLE_TTL));
    assert_eq!(idle::parse_ttl(Some("")), Some(idle::DEFAULT_IDLE_TTL));
    assert_eq!(idle::parse_ttl(Some("90")), secs(90));
    assert_eq!(idle::parse_ttl(Some("90s")), secs(90));
    assert_eq!(idle::parse_ttl(Some("10m")), secs(600));
    assert_eq!(idle::parse_ttl(Some(" 2H ")), secs(7200));
    assert_eq!(idle::parse_ttl(Some("0")), None);
    assert_eq!(idle::parse_ttl(Some("off")), None);
    assert_eq!(idle::parse_ttl(Some("Never")), None);
    // Garbage keeps the default instead of quietly disabling the feature.
    assert_eq!(idle::parse_ttl(Some("soon")), Some(idle::DEFAULT_IDLE_TTL));
    assert_eq!(idle::parse_ttl(Some("5d")), Some(idle::DEFAULT_IDLE_TTL));
}

/// `connect` opens the driver itself and loads the schema exactly once --
/// it used to go through the TUI's `QueryEngine`, which loaded it too.
#[tokio::test]
async fn connect_loads_the_schema_once_and_needs_no_tui() {
    use std::sync::atomic::AtomicUsize;
    use tradar_query::{DriverOpener, QueryResult, SchemaInfo};

    struct Fake(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl QueryDriver for Fake {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            anyhow::bail!("unused")
        }
    }
    struct FakeOpener(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl DriverOpener for FakeOpener {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn open(&self, _target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
            Ok(Arc::new(Fake(Arc::clone(&self.0))))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.toml"));
    store
        .save(&[tradar_core::storage::SavedConnection {
            name: "f".into(),
            driver: "fake".into(),
            target: "anything".into(),
        }])
        .unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    let mut registry: HashMap<String, Box<dyn DriverOpener>> = HashMap::new();
    registry.insert("fake".into(), Box::new(FakeOpener(Arc::clone(&loads))));
    let server = Server::new(registry, Some(store));

    let reply = server
        .handle(
            json!({"id": 1, "method": "connect", "params": {"connection": "f"}}),
            &ClientConnection::detached(),
        )
        .await;
    assert_eq!(reply["result"]["connected"], true, "{reply}");
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}
