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
