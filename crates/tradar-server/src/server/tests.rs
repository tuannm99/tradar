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

#[test]
fn an_adhoc_connection_name_carries_its_driver_and_target() {
    use super::connections::{adhoc_connection, redact};
    let c = adhoc_connection("@elasticsearch:http://localhost:9201").unwrap();
    assert_eq!(
        (c.driver.as_str(), c.target.as_str()),
        ("elasticsearch", "http://localhost:9201")
    );
    // The first colon ends the driver: the target keeps its own.
    let c = adhoc_connection("@Postgres:postgres://u:p@h:5432/db").unwrap();
    assert_eq!(
        (c.driver.as_str(), c.target.as_str()),
        ("postgres", "postgres://u:p@h:5432/db")
    );
    assert_eq!(
        adhoc_connection("@sqlite:/tmp/x.db").unwrap().target,
        "/tmp/x.db"
    );
    // Not ad-hoc: a plain alias, no driver, no target.
    assert!(adhoc_connection("local").is_none());
    assert!(adhoc_connection("@:x").is_none());
    assert!(adhoc_connection("@sqlite:").is_none());
    assert!(adhoc_connection("@no-target").is_none());
    assert!(adhoc_connection("@bad driver:x").is_none());

    // A password does not reach the log.
    assert_eq!(
        redact("@postgres:postgres://ann:s3cret@h:5432/db"),
        "@postgres:postgres://ann:***@h:5432/db"
    );
    assert_eq!(
        redact("@elasticsearch:http://localhost:9201"),
        "@elasticsearch:http://localhost:9201"
    );
    assert_eq!(
        redact("@postgres:postgres://ann@h/db"),
        "@postgres:postgres://ann@h/db"
    );
    assert_eq!(redact("local"), "local");
}

/// A connection written as `@driver:target` connects with no saved entry.
#[tokio::test]
async fn connect_opens_an_adhoc_connection_without_a_saved_entry() {
    use tradar_query::{DriverOpener, QueryResult, SchemaInfo};

    struct Fake;
    #[async_trait::async_trait]
    impl QueryDriver for Fake {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            anyhow::bail!("unused")
        }
    }
    /// Remembers the target it was asked to open.
    struct Opener(Arc<std::sync::Mutex<Option<String>>>);
    #[async_trait::async_trait]
    impl DriverOpener for Opener {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn open(&self, target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
            *self.0.lock().unwrap() = Some(target.to_string());
            Ok(Arc::new(Fake))
        }
    }

    let opened = Arc::new(std::sync::Mutex::new(None));
    let mut registry: HashMap<String, Box<dyn DriverOpener>> = HashMap::new();
    registry.insert("fake".into(), Box::new(Opener(Arc::clone(&opened))));
    // No store at all: nothing is saved anywhere.
    let server = Server::new(registry, None);
    let call = |name: &str| json!({"id": 1, "method": "connect", "params": {"connection": name}});

    let reply = server
        .handle(call("@fake:db://somewhere"), &ClientConnection::detached())
        .await;
    assert_eq!(reply["result"]["connected"], true, "{reply}");
    assert_eq!(opened.lock().unwrap().as_deref(), Some("db://somewhere"));

    // An alias that is not saved is still refused, and a driver this build lacks is named.
    let reply = server
        .handle(call("plain"), &ClientConnection::detached())
        .await;
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no saved connection"),
        "{reply}"
    );
    let reply = server
        .handle(call("@nosuch:x"), &ClientConnection::detached())
        .await;
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no `nosuch` connector"),
        "{reply}"
    );
}

/// `keywords` with a `driver` answers without any connection.
#[tokio::test]
async fn keywords_of_a_driver_need_no_connection() {
    use tradar_query::DriverOpener;

    struct Opener;
    #[async_trait::async_trait]
    impl DriverOpener for Opener {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn open(&self, _target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
            anyhow::bail!("never opened")
        }
        fn keywords(&self) -> &'static [&'static str] {
            &["FOO", "BAR"]
        }
    }
    let mut registry: HashMap<String, Box<dyn DriverOpener>> = HashMap::new();
    registry.insert("fake".into(), Box::new(Opener));
    let server = Server::new(registry, None);
    let ask = |params: Value| json!({"id": 1, "method": "keywords", "params": params});

    let reply = server
        .handle(
            ask(json!({"driver": "Fake"})),
            &ClientConnection::detached(),
        )
        .await;
    assert_eq!(reply["result"], json!(["FOO", "BAR"]), "{reply}");

    let reply = server
        .handle(
            ask(json!({"driver": "nosuch"})),
            &ClientConnection::detached(),
        )
        .await;
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no `nosuch` connector"),
        "{reply}"
    );

    // Without either, it is still the open connection's, as before.
    let reply = server
        .handle(
            ask(json!({"connection": "x"})),
            &ClientConnection::detached(),
        )
        .await;
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not connected"),
        "{reply}"
    );
}
