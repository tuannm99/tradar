//! End-to-end for `http.*`: a hand-rolled `TcpListener` echoing a fixed
//! HTTP/1.1 response, driven through `Server::handle` -- same reasoning as
//! `tests/rpc_rabbitmq.rs` (no Docker needed for a thin HTTP client's own
//! request/response plumbing; the Docker coverage for a real server's
//! behavior already lives in `tradar-connector-http`'s own `mod docker`).
#![cfg(feature = "http")]

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{ClientConnection, Server, registry};

/// Accepts exactly one connection, reads the request (headers + body, if
/// `Content-Length` says there is one), and writes back a fixed response.
/// Returns the request's method/path/body so a test can assert on what was
/// actually sent (`http.send`'s own headers/method/body handling).
async fn echo_once(
    status: u16,
    response_headers: &'static str,
    response_body: &'static str,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let reason = if status == 200 { "OK" } else { "Error" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\n{response_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
    });
    addr
}

fn server_with_http(dir: &std::path::Path, target: &str) -> Server {
    let store = ConnectionStore::at(dir.join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "api".into(),
            driver: "http".into(),
            target: target.to_string(),
        }])
        .unwrap();
    Server::new(registry(), Some(store))
}

async fn call(server: &Server, method: &str, params: Value) -> Value {
    server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            &ClientConnection::detached(),
        )
        .await
}

fn ok(response: &Value) -> &Value {
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    &response["result"]
}

#[tokio::test]
async fn connect_never_probes_and_connections_list_reflects_it_right_away() {
    // Unlike RabbitMQ, `http.connect` makes no network call at all -- a
    // base URL that is never actually reachable still connects cleanly.
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_http(dir.path(), "http://127.0.0.1:0");

    let before = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["connected"], false);
    assert_eq!(ok(&before)[0]["supported"], true);

    let result = ok(&call(&server, "http.connect", json!({"connection": "api"})).await).clone();
    assert_eq!(result["connected"], true);
    assert_eq!(result["already"], false);

    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], true);

    let again = ok(&call(&server, "http.connect", json!({"connection": "api"})).await).clone();
    assert_eq!(again["already"], true);
}

#[tokio::test]
async fn send_reaches_a_real_server_and_reports_status_headers_and_body() {
    let addr = echo_once(201, "X-Reply: yes\r\n", "created").await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_http(dir.path(), &format!("http://{addr}"));
    ok(&call(&server, "http.connect", json!({"connection": "api"})).await);

    let response = ok(&call(
        &server,
        "http.send",
        json!({"connection": "api", "method": "POST", "url": "/users", "headers": "X-Test: 1", "body": "{}"}),
    )
    .await)
        .clone();

    assert_eq!(response["status"], 201);
    assert_eq!(response["body"], "created");
    assert!(
        response["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h[0] == "x-reply" && h[1] == "yes"),
        "{response}"
    );
}

#[tokio::test]
async fn send_defaults_to_get_with_no_body_when_only_connection_and_url_are_given() {
    let addr = echo_once(200, "", "ok").await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_http(dir.path(), &format!("http://{addr}"));
    ok(&call(&server, "http.connect", json!({"connection": "api"})).await);

    let response = ok(&call(
        &server,
        "http.send",
        json!({"connection": "api", "url": "/"}),
    )
    .await)
    .clone();

    assert_eq!(response["status"], 200);
    assert_eq!(response["body"], "ok");
}

#[tokio::test]
async fn disconnect_forgets_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_http(dir.path(), "http://127.0.0.1:0");
    ok(&call(&server, "http.connect", json!({"connection": "api"})).await);

    let disconnected =
        ok(&call(&server, "http.disconnect", json!({"connection": "api"})).await).clone();
    assert_eq!(disconnected["disconnected"], true);

    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], false);

    // Any `http.*` call against the now-forgotten connection is a clean
    // app error, not a panic.
    let result = call(
        &server,
        "http.send",
        json!({"connection": "api", "url": "/"}),
    )
    .await;
    assert!(result.get("error").is_some());
}
