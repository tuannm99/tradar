//! End-to-end for `socket.*`: a real TCP echo server (no Docker -- a raw
//! socket listener needs nothing to containerize, same reason
//! `tradar-connector-socket`'s own tests need none either), driven through
//! `Server::handle`/`Server::new_client` so pushed `socket.tail.message`
//! notifications can be read back off the same channel `transport.rs`
//! itself writes to -- same shape as `tests/rpc.rs`/`tests/rpc_grpc.rs`.
#![cfg(feature = "socket")]

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{ClientConnection, Server, registry};

/// A throwaway TCP listener that echoes back whatever it reads.
async fn echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if socket.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

fn server_with_socket(dir: &std::path::Path, target: &str) -> Server {
    let store = ConnectionStore::at(dir.join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "echo".into(),
            driver: "socket".into(),
            target: target.to_string(),
        }])
        .unwrap();
    Server::new(registry(), Some(store))
}

/// A `ClientConnection` plus the receiving end of its own outbox -- a real
/// socket's reply/notification stream, just read directly instead of over a
/// unix socket.
struct TestClient {
    conn: ClientConnection,
    lines: mpsc::UnboundedReceiver<String>,
}

fn test_client(server: &Server) -> TestClient {
    let (tx, rx) = mpsc::unbounded_channel();
    TestClient {
        conn: server.new_client(tx),
        lines: rx,
    }
}

async fn call(server: &Server, client: &ClientConnection, method: &str, params: Value) -> Value {
    server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            client,
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

/// Waits for the next line on `client`'s own channel that is a notification
/// (no `id`) rather than a reply to a request -- `Server::handle`'s own
/// reply always lands on this same channel too (see `TestClient`'s doc
/// comment), so a test awaiting a push has to skip past it.
async fn next_notification(client: &mut TestClient) -> Value {
    timeout(Duration::from_secs(5), async {
        loop {
            let line = client.lines.recv().await.expect("channel closed");
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg.get("id").is_none() {
                return msg;
            }
        }
    })
    .await
    .expect("no notification within 5s")
}

#[tokio::test]
async fn tail_start_connects_and_receives_data_as_notifications() {
    let addr = echo_server().await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_socket(dir.path(), &addr);
    let mut client = test_client(&server);

    let result = ok(&call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await)
    .clone();
    let subscription = result["subscription"].as_str().unwrap().to_string();

    let sent = ok(&call(
        &server,
        &client.conn,
        "socket.send",
        json!({"subscription": subscription, "text": "hello"}),
    )
    .await)
    .clone();
    assert_eq!(sent["sent"], true);

    // Two notifications arrive, in no guaranteed order: the send itself
    // echoes as "sent" (pushed synchronously once the write lands), and
    // the echo server's reply comes back as "received" (pushed once the
    // background reader task observes it) -- a genuine network round trip,
    // not ordered against the first by anything this test can assume.
    let first = next_notification(&mut client).await;
    let second = next_notification(&mut client).await;
    for msg in [&first, &second] {
        assert_eq!(msg["method"], "socket.tail.message");
        assert_eq!(msg["params"]["subscription"], subscription);
        assert_eq!(msg["params"]["text"], "hello\n");
    }
    let directions: std::collections::HashSet<_> = [&first, &second]
        .iter()
        .map(|m| m["params"]["direction"].as_str().unwrap())
        .collect();
    assert_eq!(
        directions,
        std::collections::HashSet::from(["sent", "received"])
    );
}

#[tokio::test]
async fn send_without_the_newline_flag_sends_exactly_the_typed_text() {
    let addr = echo_server().await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_socket(dir.path(), &addr);
    let client = test_client(&server);

    let result = ok(&call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await)
    .clone();
    let subscription = result["subscription"].as_str().unwrap();

    ok(&call(
        &server,
        &client.conn,
        "socket.send",
        json!({"subscription": subscription, "text": "ping", "newline": false}),
    )
    .await);

    let mut client = client;
    let first = next_notification(&mut client).await;
    assert_eq!(first["params"]["text"], "ping");
}

#[tokio::test]
async fn connections_list_reports_the_socket_connection_as_connected_while_tailing() {
    let addr = echo_server().await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_socket(dir.path(), &addr);
    let client = test_client(&server);

    let before = call(&server, &client.conn, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["connected"], false);
    assert_eq!(ok(&before)[0]["supported"], true);

    call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await;

    let after = call(&server, &client.conn, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], true);
}

#[tokio::test]
async fn tail_stop_sends_no_more_notifications() {
    let addr = echo_server().await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_socket(dir.path(), &addr);
    let mut client = test_client(&server);

    let result = ok(&call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await)
    .clone();
    let subscription = result["subscription"].as_str().unwrap().to_string();

    let stopped = ok(&call(
        &server,
        &client.conn,
        "socket.tail.stop",
        json!({"subscription": subscription}),
    )
    .await)
    .clone();
    assert_eq!(stopped["stopped"], true);

    // A second stop on an already-gone subscription is reported, not an error.
    let stopped_again = ok(&call(
        &server,
        &client.conn,
        "socket.tail.stop",
        json!({"subscription": subscription}),
    )
    .await)
    .clone();
    assert_eq!(stopped_again["stopped"], false);

    // Sending on a stopped subscription is a clean app error, not a panic.
    let send_result = call(
        &server,
        &client.conn,
        "socket.send",
        json!({"subscription": subscription, "text": "too late"}),
    )
    .await;
    assert!(send_result.get("error").is_some());

    assert!(
        client.lines.try_recv().is_err(),
        "no notification should arrive after stop"
    );
}

#[tokio::test]
async fn client_disconnected_stops_every_tail_that_client_started_without_stopping_it() {
    let addr = echo_server().await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_socket(dir.path(), &addr);
    let client = test_client(&server);

    call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await;
    let before = call(&server, &client.conn, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["connected"], true);

    // Same thing `transport.rs`'s `serve_connection` does once a client's
    // socket read loop ends -- never explicitly `socket.tail.stop`ped.
    server.client_disconnected(&client.conn);

    // A fresh client (a different socket) sees the tail is gone.
    let other = test_client(&server);
    let after = call(&server, &other.conn, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], false);
}

#[tokio::test]
async fn an_unreachable_target_is_a_clean_app_error() {
    let dir = tempfile::tempdir().unwrap();
    // Port 0 always refuses a connect attempt.
    let server = server_with_socket(dir.path(), "127.0.0.1:0");
    let client = test_client(&server);

    let result = call(
        &server,
        &client.conn,
        "socket.tail.start",
        json!({"connection": "echo"}),
    )
    .await;
    assert!(result.get("error").is_some());
}
