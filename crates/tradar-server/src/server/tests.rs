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
