//! End-to-end for `rabbitmq.*`: a hand-rolled, minimal HTTP/1.1 server
//! standing in for the Management API, driven through `Server::handle` --
//! same shape as `tests/rpc_grpc.rs`'s toy gRPC server and
//! `tests/rpc_socket.rs`'s TCP echo server, same reason (no Docker needed
//! for a thin HTTP client's own request/response plumbing -- the Docker
//! coverage for the Management API's actual behavior already lives in
//! `tradar-connector-rabbitmq`'s own `mod docker` tests). Not axum/hyper:
//! both are only transitive dependencies elsewhere in this workspace, and
//! the handful of fixed routes this needs don't justify a new direct one
//! (same "plain `TcpListener`, no framework" choice `tests/rpc_socket.rs`
//! already made for its own echo server).
#![cfg(feature = "rabbitmq")]

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{ClientConnection, Server, registry};

/// One canned `(status, body)` response per exact `METHOD path`, looked up
/// per request; an unlisted route is a 404 -- enough to drive every
/// `rabbitmq.*` method without a real broker.
async fn mock_management_api(routes: Vec<(&'static str, u16, Value)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let request_line = read_request_line(&mut socket).await;
                let mut handled = false;
                for (route, status, body) in &routes {
                    if request_line == *route {
                        write_response(&mut socket, *status, body).await;
                        handled = true;
                        break;
                    }
                }
                if !handled {
                    write_response(&mut socket, 404, &json!({"error": "not found"})).await;
                }
            });
        }
    });
    addr
}

/// Reads exactly the request line + headers (up to the blank line) and,
/// when `Content-Length` is present, the body too -- discarded, since no
/// route here needs to inspect what was published/sent, only that the
/// right path and method were hit.
async fn read_request_line(socket: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(header_end) = find_subsequence(&buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length: usize = headers
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_string())
                })
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let body_so_far = buf.len() - (header_end + 4);
            let mut remaining = content_length.saturating_sub(body_so_far);
            while remaining > 0 {
                let n = socket.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                remaining = remaining.saturating_sub(n);
            }
            let first_line = headers.lines().next().unwrap_or("");
            let mut parts = first_line.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("");
            return format!("{method} {path}");
        }
    }
    String::new()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn write_response(socket: &mut tokio::net::TcpStream, status: u16, body: &Value) {
    let text = body.to_string();
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

fn server_with_rabbitmq(dir: &std::path::Path, target: &str) -> Server {
    let store = ConnectionStore::at(dir.join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "bunny".into(),
            driver: "rabbitmq".into(),
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
async fn connect_probes_overview_and_connections_list_reflects_it() {
    let addr = mock_management_api(vec![("GET /api/overview", 200, json!({}))]).await;
    let dir = tempfile::tempdir().unwrap();
    let target = format!("http://{addr}");
    let server = server_with_rabbitmq(dir.path(), &target);

    let before = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["connected"], false);
    assert_eq!(ok(&before)[0]["supported"], true);

    let result =
        ok(&call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await).clone();
    assert_eq!(result["connected"], true);
    assert_eq!(result["already"], false);

    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], true);

    // Connecting again is reported, not re-dialed.
    let again =
        ok(&call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await).clone();
    assert_eq!(again["already"], true);
}

#[tokio::test]
async fn queues_and_exchanges_round_trip_and_the_default_exchange_is_filtered_out() {
    let addr = mock_management_api(vec![
        ("GET /api/overview", 200, json!({})),
        (
            "GET /api/queues/%2f",
            200,
            json!([{"name": "orders", "messages_ready": 3, "messages_unacknowledged": 1, "consumers": 2}]),
        ),
        (
            "GET /api/exchanges/%2f",
            200,
            json!([{"name": "orders.topic", "type": "topic"}, {"name": "", "type": "direct"}]),
        ),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_rabbitmq(dir.path(), &format!("http://{addr}"));
    ok(&call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await);

    let queues =
        ok(&call(&server, "rabbitmq.queues", json!({"connection": "bunny"})).await).clone();
    assert_eq!(queues[0]["name"], "orders");
    assert_eq!(queues[0]["messages_ready"], 3);

    let exchanges = ok(&call(
        &server,
        "rabbitmq.exchanges",
        json!({"connection": "bunny"}),
    )
    .await)
    .clone();
    assert_eq!(
        exchanges.as_array().unwrap().len(),
        1,
        "the nameless default exchange must be filtered out"
    );
    assert_eq!(exchanges[0]["name"], "orders.topic");
}

#[tokio::test]
async fn bindings_peek_and_publish_round_trip() {
    let addr = mock_management_api(vec![
        ("GET /api/overview", 200, json!({})),
        (
            "GET /api/exchanges/%2f/orders.topic/bindings/source",
            200,
            json!([{"destination": "orders", "routing_key": "orders.created"}]),
        ),
        (
            "POST /api/queues/%2f/orders/get",
            200,
            json!([{"exchange": "", "routing_key": "orders", "redelivered": false, "payload": "hello from test"}]),
        ),
        ("POST /api/exchanges/%2f/orders.topic/publish", 200, json!({"routed": true})),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_rabbitmq(dir.path(), &format!("http://{addr}"));
    ok(&call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await);

    let bindings = ok(&call(
        &server,
        "rabbitmq.bindings",
        json!({"connection": "bunny", "exchange": "orders.topic"}),
    )
    .await)
    .clone();
    assert_eq!(bindings[0]["destination"], "orders");
    assert_eq!(bindings[0]["routing_key"], "orders.created");

    let messages = ok(&call(
        &server,
        "rabbitmq.peek",
        json!({"connection": "bunny", "queue": "orders"}),
    )
    .await)
    .clone();
    assert_eq!(messages[0]["payload"], "hello from test");

    let published = ok(&call(
        &server,
        "rabbitmq.publish",
        json!({"connection": "bunny", "exchange": "orders.topic", "routing_key": "orders.created", "payload": "hi"}),
    )
    .await)
        .clone();
    assert_eq!(published["ok"], true);
}

#[tokio::test]
async fn disconnect_forgets_the_connection() {
    let addr = mock_management_api(vec![("GET /api/overview", 200, json!({}))]).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_rabbitmq(dir.path(), &format!("http://{addr}"));
    ok(&call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await);

    let disconnected = ok(&call(
        &server,
        "rabbitmq.disconnect",
        json!({"connection": "bunny"}),
    )
    .await)
    .clone();
    assert_eq!(disconnected["disconnected"], true);

    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], false);

    // Any `rabbitmq.*` call against the now-forgotten connection is a
    // clean app error, not a panic.
    let result = call(&server, "rabbitmq.queues", json!({"connection": "bunny"})).await;
    assert!(result.get("error").is_some());
}

#[tokio::test]
async fn connecting_to_an_unreachable_target_is_a_clean_app_error() {
    let dir = tempfile::tempdir().unwrap();
    // Port 0 always refuses a connect attempt.
    let server = server_with_rabbitmq(dir.path(), "http://127.0.0.1:0");

    let result = call(&server, "rabbitmq.connect", json!({"connection": "bunny"})).await;
    assert!(result.get("error").is_some());
}
