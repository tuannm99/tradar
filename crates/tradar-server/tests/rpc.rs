//! End-to-end against a real SQLite file: saved connection -> connect ->
//! execute -> page -> edit, first through `Server::handle` directly, then
//! once over the actual unix socket.
#![cfg(feature = "sqlite")]

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{Server, bind, registry, serve};

fn server_with_sqlite(dir: &std::path::Path) -> Server {
    let store = ConnectionStore::at(dir.join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "local".into(),
            driver: "sqlite".into(),
            target: dir.join("t.db").to_string_lossy().into_owned(),
        }])
        .unwrap();
    Server::new(registry(), Some(store))
}

async fn call(server: &Server, method: &str, params: Value) -> Value {
    server
        .handle(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .await
}

fn ok(response: &Value) -> &Value {
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    &response["result"]
}

async fn connected(dir: &std::path::Path) -> Server {
    let server = server_with_sqlite(dir);
    ok(&call(&server, "connect", json!({"connection": "local"})).await);
    server
}

#[tokio::test]
async fn lists_saved_connections_with_their_connected_state() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_sqlite(dir.path());

    let before = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["connected"], false);
    assert_eq!(ok(&before)[0]["supported"], true);

    ok(&call(&server, "connect", json!({"connection": "local"})).await);
    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], true);
}

#[tokio::test]
async fn execute_pages_a_result_through_a_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = |sql: &str| json!({"connection": "local", "query": sql});
    ok(&call(
        &server,
        "execute",
        q("CREATE TABLE n (id INTEGER PRIMARY KEY, v TEXT)"),
    )
    .await);
    for i in 0..5 {
        ok(&call(
            &server,
            "execute",
            q(&format!("INSERT INTO n VALUES ({i}, 'v{i}')")),
        )
        .await);
    }

    let first = call(
        &server,
        "execute",
        json!({"connection": "local", "query": "SELECT * FROM n ORDER BY id", "page_size": 2}),
    )
    .await;
    let first = ok(&first);
    assert_eq!(first["kind"], "table");
    assert_eq!(first["columns"], json!(["id", "v"]));
    assert_eq!(first["total"], 5);
    assert_eq!(first["rows"], json!([["0", "v0"], ["1", "v1"]]));

    let page = call(
        &server,
        "fetch",
        json!({"cursor": first["cursor"], "offset": 3, "limit": 10}),
    )
    .await;
    assert_eq!(ok(&page)["rows"], json!([["3", "v3"], ["4", "v4"]]));

    // Past the end is an empty page, not an error.
    let beyond = call(
        &server,
        "fetch",
        json!({"cursor": first["cursor"], "offset": 99}),
    )
    .await;
    assert_eq!(ok(&beyond)["rows"], json!([]));

    ok(&call(&server, "cursor.close", json!({"cursor": first["cursor"]})).await);
    let gone = call(&server, "fetch", json!({"cursor": first["cursor"]})).await;
    assert!(gone["error"]["message"].as_str().unwrap().contains("gone"));
}

#[tokio::test]
async fn a_write_reports_affected_rows_and_makes_no_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = |sql: &str| json!({"connection": "local", "query": sql});
    ok(&call(&server, "execute", q("CREATE TABLE n (id INTEGER)")).await);

    let response = call(&server, "execute", q("INSERT INTO n VALUES (1)")).await;

    assert_eq!(ok(&response)["kind"], "affected");
    assert_eq!(ok(&response)["rows"], 1);
    assert!(ok(&response).get("cursor").is_none());
}

#[tokio::test]
async fn schema_reports_columns_and_primary_keys() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q =
        json!({"connection": "local", "query": "CREATE TABLE n (id INTEGER PRIMARY KEY, v TEXT)"});
    ok(&call(&server, "execute", q).await);

    let schema = call(&server, "schema", json!({"connection": "local"})).await;
    let table = &ok(&schema)[0];

    assert_eq!(table["name"], "n");
    assert_eq!(table["columns"][0]["name"], "id");
    assert_eq!(table["columns"][0]["primary_key"], true);
}

#[tokio::test]
async fn edit_builds_a_statement_without_running_it() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = |sql: &str| json!({"connection": "local", "query": sql});
    ok(&call(
        &server,
        "execute",
        q("CREATE TABLE n (id INTEGER PRIMARY KEY, v TEXT)"),
    )
    .await);
    ok(&call(&server, "execute", q("INSERT INTO n VALUES (1, 'old')")).await);

    let source = call(&server, "edit.source", q("SELECT * FROM n")).await;
    assert_eq!(ok(&source)["table"], "n");

    let sql = call(
        &server,
        "edit.sql",
        json!({
            "connection": "local", "table": "n", "key": {"id": "1"},
            "change": {"set": {"column": "v", "value": "new"}},
        }),
    )
    .await;
    let sql = ok(&sql)["sql"].as_str().unwrap().to_string();
    assert!(sql.starts_with("UPDATE"), "{sql}");

    let still = call(&server, "execute", q("SELECT v FROM n")).await;
    assert_eq!(ok(&still)["rows"], json!([["old"]]));
}

#[tokio::test]
async fn split_uses_the_drivers_own_statement_rules() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;

    let response = call(
        &server,
        "split",
        json!({"connection": "local", "text": "SELECT 1; SELECT ';'"}),
    )
    .await;

    assert_eq!(ok(&response).as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn errors_are_reported_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;

    let bad_sql = call(
        &server,
        "execute",
        json!({"connection": "local", "query": "SELEC"}),
    )
    .await;
    assert!(bad_sql["error"]["message"].as_str().is_some());

    let not_connected = call(&server, "schema", json!({"connection": "nope"})).await;
    assert!(
        not_connected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not connected")
    );

    let missing = call(&server, "nope", Value::Null).await;
    assert_eq!(missing["error"]["code"], -32601);

    let bad_params = call(&server, "execute", json!({"connection": "local"})).await;
    assert_eq!(bad_params["error"]["code"], -32602);

    // Still alive afterwards.
    let status = call(&server, "status", json!({"connection": "local"})).await;
    assert_eq!(ok(&status)["alive"], true);
}

#[tokio::test]
async fn answers_over_a_real_unix_socket_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server_with_sqlite(dir.path()));
    let path = dir.path().join("sock").join("s.sock");
    let listener = bind(&path).await.unwrap();
    tokio::spawn(serve(listener, server));

    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(b"{\"id\":7,\"method\":\"connectors.list\"}\n")
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["result"], json!(["sqlite"]));

    write.write_all(b"not json\n").await.unwrap();
    let reply: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["error"]["code"], -32700);

    // A second server must refuse to steal a live socket.
    assert!(bind(&path).await.is_err());
}

#[tokio::test]
async fn complete_is_context_aware_like_the_tui() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = |sql: &str| json!({"connection": "local", "query": sql});
    ok(&call(
        &server,
        "execute",
        q("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)"),
    )
    .await);
    ok(&call(&server, "execute", q("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id), total REAL)")).await);
    // `connect` built the completion source before these tables existed;
    // `schema` is what refreshes it.
    ok(&call(&server, "schema", json!({"connection": "local"})).await);

    let texts = |r: &Value| -> Vec<String> {
        ok(r)["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["text"].as_str().unwrap().to_string())
            .collect()
    };
    let complete = |text: &str| {
        call(
            &server,
            "complete",
            json!({"connection": "local", "text": text}),
        )
    };

    // `alias.` -> only that table's own columns.
    let cols = complete("SELECT * FROM orders o WHERE o.").await;
    assert_eq!(texts(&cols), ["id", "total", "user_id"]);
    assert_eq!(ok(&cols)["items"][0]["kind"], "column");

    // Partial word after the dot narrows, and `prefix` reports what was typed.
    let narrowed = complete("SELECT * FROM orders o WHERE o.us").await;
    assert_eq!(texts(&narrowed), ["user_id"]);
    assert_eq!(ok(&narrowed)["prefix"], "us");

    // Multi-line context still resolves the alias.
    let multi = complete("SELECT *\nFROM orders o\nWHERE o.t").await;
    assert_eq!(texts(&multi), ["total"]);

    // After JOIN, a table already in the query is not offered again.
    let join = complete("SELECT * FROM users u JOIN ").await;
    assert!(!texts(&join).contains(&"users".to_string()));

    // Flat fallback: schema names, then keywords.
    let flat = complete("SELECT * FROM us").await;
    assert_eq!(texts(&flat)[0], "users");
}

#[tokio::test]
async fn ddl_refreshes_completion_without_a_schema_call() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = |sql: &str| json!({"connection": "local", "query": sql});

    ok(&call(
        &server,
        "execute",
        q("CREATE TABLE widgets (id INTEGER, sku TEXT)"),
    )
    .await);

    let r = call(
        &server,
        "complete",
        json!({"connection": "local", "text": "SELECT * FROM wid"}),
    )
    .await;
    assert_eq!(ok(&r)["items"][0]["text"], "widgets");

    ok(&call(&server, "execute", q("DROP TABLE widgets")).await);
    let r = call(
        &server,
        "complete",
        json!({"connection": "local", "text": "SELECT * FROM wid"}),
    )
    .await;
    assert_eq!(ok(&r)["items"], json!([]));
}

#[tokio::test]
async fn a_located_error_carries_its_position() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;

    let r = call(
        &server,
        "execute",
        json!({"connection": "local", "query": "SELECT 1\nFROM"}),
    )
    .await;

    // SQLite quotes the offending token and the driver turns that into a
    // `LINE N: ... ^` marker; the server lifts it into structured data.
    let message = r["error"]["message"].as_str().unwrap();
    if message.contains("LINE ") {
        assert!(r["error"]["data"]["line"].as_u64().unwrap() >= 1, "{r}");
        assert!(r["error"]["data"]["column"].is_u64(), "{r}");
    } else {
        assert!(r["error"]["data"].is_null(), "{r}");
    }
}

#[tokio::test]
async fn cancel_stops_a_running_query_and_other_requests_are_not_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(connected(dir.path()).await);
    // A recursive CTE that would run for a very long time.
    let slow =
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c";

    let running = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            call(
                &server,
                "execute",
                json!({"connection": "local", "query": slow, "query_id": "q1"}),
            )
            .await
        })
    };
    // Wait until it is registered, then prove an unrelated request still answers.
    for _ in 0..100 {
        let status = call(&server, "status", json!({"connection": "local"})).await;
        assert!(status.get("error").is_none());
        let cancelled = call(&server, "cancel", json!({"query_id": "q1"})).await;
        if ok(&cancelled)["cancelled"] == true {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), running)
        .await
        .expect("a cancelled query must return promptly")
        .unwrap();
    assert_eq!(outcome["error"]["message"], "query cancelled");

    let unknown = call(&server, "cancel", json!({"query_id": "nope"})).await;
    assert_eq!(ok(&unknown)["cancelled"], false);
}

#[tokio::test]
async fn one_connection_runs_requests_concurrently() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server_with_sqlite(dir.path()));
    let path = dir.path().join("c.sock");
    tokio::spawn(serve(bind(&path).await.unwrap(), server));

    let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let slow =
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c";
    for request in [json!({"id": 1, "method": "connect", "params": {"connection": "local"}})] {
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    let _connect = lines.next_line().await.unwrap().unwrap();

    // The slow query goes first, the cancel second -- on the same socket.
    for request in [
        json!({"id": 2, "method": "execute", "params": {"connection": "local", "query": slow, "query_id": "z"}}),
        json!({"id": 3, "method": "connectors.list"}),
    ] {
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    // The fast reply (id 3) must arrive while the slow one (id 2) is still running.
    let first: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(
        first["id"], 3,
        "a slow query must not hold up later requests: {first}"
    );

    write
        .write_all(
            format!(
                "{}\n",
                json!({"id": 4, "method": "cancel", "params": {"query_id": "z"}})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut seen = Vec::new();
    for _ in 0..2 {
        let v: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        seen.push(v["id"].as_u64().unwrap());
    }
    seen.sort();
    assert_eq!(seen, [2, 4]);
}

#[tokio::test]
async fn snippet_is_written_in_the_drivers_own_language() {
    let dir = tempfile::tempdir().unwrap();
    let server = connected(dir.path()).await;
    let q = json!({"connection": "local", "query": "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)"});
    ok(&call(&server, "execute", q).await);

    let read = call(
        &server,
        "snippet",
        json!({"connection": "local", "name": "users"}),
    )
    .await;
    assert_eq!(ok(&read)["text"], "SELECT * FROM \"users\" LIMIT 100;");

    let missing = call(
        &server,
        "snippet",
        json!({"connection": "local", "name": "nope"}),
    )
    .await;
    assert!(
        missing["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no table")
    );

    let bad = call(
        &server,
        "snippet",
        json!({"connection": "local", "name": "users", "op": "zap"}),
    )
    .await;
    assert_eq!(bad["error"]["code"], -32602);
}
