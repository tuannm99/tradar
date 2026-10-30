//! End-to-end for `grpc.*`: a real toy gRPC server (`tests/fixtures/greeter.proto`)
//! over a real TCP loopback connection, driven through `Server::handle` --
//! same shape as `tests/rpc.rs`'s SQLite tests, but for the connector
//! that bypasses `Connector`/`QueryDriver` entirely (see `src/grpc.rs`'s
//! doc comment). The dynamic-invocation mechanics were de-risked by a
//! throwaway spike first -- see `docs/backlog/grpc-spike-2026-10-12.md`.
#![cfg(feature = "grpc")]
// The streaming fixtures return `tonic::Status` as their error type, as the
// generated service trait requires.
#![allow(clippy::result_large_err)]

use serde_json::{Value, json};
use tonic::transport::Server as TonicServer;
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{ClientConnection, Server, registry};

pub mod greeter {
    include!(concat!(env!("OUT_DIR"), "/greeter.rs"));
}

const DESCRIPTOR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter_descriptor.bin"));

#[derive(Default)]
struct GreeterSvc;

#[tonic::async_trait]
impl greeter::greeter_server::Greeter for GreeterSvc {
    async fn say_hello(
        &self,
        request: tonic::Request<greeter::HelloRequest>,
    ) -> Result<tonic::Response<greeter::HelloReply>, tonic::Status> {
        let token = request
            .metadata()
            .get("x-token")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let name = request.into_inner().name;
        if name == "slow" {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
        if name == "boom" {
            return Err(tonic::Status::invalid_argument("name must not be 'boom'"));
        }
        let message = match token {
            Some(token) => format!("Hello, {name}! [token={token}]"),
            None => format!("Hello, {name}!"),
        };
        let mut response = tonic::Response::new(greeter::HelloReply {
            message,
            length: name.len() as i32,
        });
        response
            .metadata_mut()
            .insert("x-served-by", "greeter".parse().unwrap());
        Ok(response)
    }

    type CountdownStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<greeter::HelloReply, tonic::Status>> + Send>,
    >;

    async fn countdown(
        &self,
        request: tonic::Request<greeter::HelloRequest>,
    ) -> Result<tonic::Response<Self::CountdownStream>, tonic::Status> {
        let name = request.into_inner().name;
        let items: Vec<_> = (1..=3)
            .map(|i| {
                Ok(greeter::HelloReply {
                    message: format!("{name} #{i}"),
                    length: i,
                })
            })
            .collect();
        Ok(tonic::Response::new(Box::pin(tokio_stream::iter(items))))
    }

    async fn collect(
        &self,
        request: tonic::Request<tonic::Streaming<greeter::HelloRequest>>,
    ) -> Result<tonic::Response<greeter::HelloReply>, tonic::Status> {
        let mut stream = request.into_inner();
        let mut names = Vec::new();
        while let Some(msg) = stream.message().await? {
            names.push(msg.name);
        }
        Ok(tonic::Response::new(greeter::HelloReply {
            message: names.join("+"),
            length: names.len() as i32,
        }))
    }

    type ChatStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<greeter::HelloReply, tonic::Status>> + Send>,
    >;

    async fn chat(
        &self,
        request: tonic::Request<tonic::Streaming<greeter::HelloRequest>>,
    ) -> Result<tonic::Response<Self::ChatStream>, tonic::Status> {
        use tokio_stream::StreamExt;
        let replies = request.into_inner().map(|msg| {
            msg.map(|m| greeter::HelloReply {
                message: format!("echo {}", m.name),
                length: 0,
            })
        });
        Ok(tonic::Response::new(Box::pin(replies)))
    }
}

/// Starts the toy server on an ephemeral loopback port, with reflection
/// enabled only when `with_reflection` is set -- a server without it is
/// what exercises the `grpc.load_proto` fallback path.
async fn spawn_greeter(with_reflection: bool) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let mut builder =
        TonicServer::builder().add_service(greeter::greeter_server::GreeterServer::new(GreeterSvc));
    if with_reflection {
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(DESCRIPTOR)
            .build_v1()
            .unwrap();
        builder = builder.add_service(reflection);
    }
    tokio::spawn(async move {
        builder.serve_with_incoming(incoming).await.unwrap();
    });
    // Give the listener a moment to actually start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

fn server_with_grpc(dir: &std::path::Path, target: &str) -> Server {
    let store = ConnectionStore::at(dir.join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "g".into(),
            driver: "grpc".into(),
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

fn err(response: &Value) -> &Value {
    response
        .get("error")
        .unwrap_or_else(|| panic!("expected an error, got {response}"))
}

#[tokio::test]
async fn skeleton_gives_a_request_to_start_from() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let skeleton = call(
        &server,
        "grpc.skeleton",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "SayHello"}),
    )
    .await;
    assert_eq!(ok(&skeleton)["skeleton"], "{\n  \"name\": \"\"\n}");

    let missing = call(
        &server,
        "grpc.skeleton",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "Nope"}),
    )
    .await;
    err(&missing);
}

#[tokio::test]
async fn connect_discovers_services_via_reflection_and_invokes_unary() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());

    let connected = call(&server, "grpc.connect", json!({"connection": "g"})).await;
    assert_eq!(ok(&connected)["connected"], true);
    assert_eq!(ok(&connected)["already"], false);

    let services = call(&server, "grpc.services", json!({"connection": "g"})).await;
    assert_eq!(ok(&services), &json!(["greeter.Greeter"]));

    let methods = call(
        &server,
        "grpc.methods",
        json!({"connection": "g", "service": "greeter.Greeter"}),
    )
    .await;
    assert_eq!(
        ok(&methods),
        &json!([
            {"name": "SayHello", "input": "greeter.HelloRequest", "output": "greeter.HelloReply", "client_streaming": false, "server_streaming": false},
            {"name": "Countdown", "input": "greeter.HelloRequest", "output": "greeter.HelloReply", "client_streaming": false, "server_streaming": true},
            {"name": "Collect", "input": "greeter.HelloRequest", "output": "greeter.HelloReply", "client_streaming": true, "server_streaming": false},
            {"name": "Chat", "input": "greeter.HelloRequest", "output": "greeter.HelloReply", "client_streaming": true, "server_streaming": true},
        ])
    );

    let invoked = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g",
            "service": "greeter.Greeter",
            "method": "SayHello",
            "request": {"name": "World"},
        }),
    )
    .await;
    assert_eq!(
        ok(&invoked)["response"],
        json!({"message": "Hello, World!", "length": 5})
    );
}

#[tokio::test]
async fn a_grpc_status_error_carries_the_code_in_data() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let invoked = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g",
            "service": "greeter.Greeter",
            "method": "SayHello",
            "request": {"name": "boom"},
        }),
    )
    .await;
    let error = err(&invoked);
    // INVALID_ARGUMENT is tonic::Code::InvalidArgument, numeric value 3.
    assert_eq!(error["data"]["grpc_code"], 3);
    assert!(error["message"].as_str().unwrap().contains("must not be"));
}

#[tokio::test]
async fn connecting_without_reflection_leaves_the_pool_empty_until_load_proto() {
    let addr = spawn_greeter(false).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let services = call(&server, "grpc.services", json!({"connection": "g"})).await;
    assert_eq!(ok(&services), &json!([]));

    let proto_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/greeter.proto");
    let loaded = call(
        &server,
        "grpc.load_proto",
        json!({"connection": "g", "path": proto_path}),
    )
    .await;
    assert_eq!(ok(&loaded)["added"], json!(["greeter.Greeter"]));

    let services = call(&server, "grpc.services", json!({"connection": "g"})).await;
    assert_eq!(ok(&services), &json!(["greeter.Greeter"]));

    // A reconnect -- after `disconnect`, or the idle reaper closing it --
    // loads the hand-added `.proto` again rather than losing it.
    ok(&call(&server, "disconnect", json!({"connection": "g"})).await);
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);
    let services = call(&server, "grpc.services", json!({"connection": "g"})).await;
    assert_eq!(ok(&services), &json!(["greeter.Greeter"]));

    // The same unary invoke path works once the pool is filled by hand --
    // confirms the fallback produces a pool just as usable as reflection's.
    let invoked = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g",
            "service": "greeter.Greeter",
            "method": "SayHello",
            "request": {"name": "proto"},
        }),
    )
    .await;
    assert_eq!(
        ok(&invoked)["response"],
        json!({"message": "Hello, proto!", "length": 5})
    );
}

#[tokio::test]
async fn disconnect_forgets_the_connection() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let disconnected = call(&server, "grpc.disconnect", json!({"connection": "g"})).await;
    assert_eq!(ok(&disconnected)["disconnected"], true);

    let services = call(&server, "grpc.services", json!({"connection": "g"})).await;
    assert!(
        err(&services)["message"]
            .as_str()
            .unwrap()
            .contains("is not connected")
    );
}

#[tokio::test]
async fn connections_list_reports_the_grpc_connection_as_supported_and_connected() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());

    let before = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&before)[0]["supported"], true);
    assert_eq!(ok(&before)[0]["connected"], false);

    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);
    let after = call(&server, "connections.list", Value::Null).await;
    assert_eq!(ok(&after)[0]["connected"], true);
}

#[tokio::test]
async fn invoking_an_unknown_method_is_an_app_error_not_a_panic() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let invoked = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g",
            "service": "greeter.Greeter",
            "method": "NoSuchMethod",
            "request": {},
        }),
    )
    .await;
    assert!(
        err(&invoked)["message"]
            .as_str()
            .unwrap()
            .contains("NoSuchMethod")
    );
}

#[tokio::test]
async fn metadata_goes_out_with_the_request_and_response_headers_come_back() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let invoked = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g", "service": "greeter.Greeter", "method": "SayHello",
            "request": {"name": "Ann"}, "metadata": {"X-Token": "abc"},
        }),
    )
    .await;
    assert_eq!(
        ok(&invoked)["response"]["message"],
        "Hello, Ann! [token=abc]"
    );
    assert_eq!(ok(&invoked)["headers"]["x-served-by"], "greeter");

    let bad = call(
        &server,
        "grpc.invoke",
        json!({
            "connection": "g", "service": "greeter.Greeter", "method": "SayHello",
            "request": {}, "metadata": {"blob-bin": "x"},
        }),
    )
    .await;
    err(&bad);
}

#[tokio::test]
async fn invoke_refuses_a_streaming_method_and_stream_start_a_unary_one() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let streaming = call(
        &server,
        "grpc.invoke",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "Countdown", "request": {}}),
    )
    .await;
    assert!(
        err(&streaming)["message"]
            .as_str()
            .unwrap()
            .contains("grpc.stream.start")
    );

    let (client, _inbox) = ClientConnection::with_inbox(1);
    let unary = server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": "grpc.stream.start", "params": {
                "connection": "g", "service": "greeter.Greeter", "method": "SayHello"}}),
            &client,
        )
        .await;
    assert!(
        err(&unary)["message"]
            .as_str()
            .unwrap()
            .contains("grpc.invoke")
    );
}

/// Reads pushed `grpc.stream.message` items until one of kind `end`/`error`.
async fn drain_stream(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> Vec<Value> {
    let mut items = Vec::new();
    loop {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), inbox.recv())
            .await
            .expect("stream went quiet")
            .expect("inbox closed");
        let note: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(note["method"], "grpc.stream.message");
        let item = note["params"].clone();
        let done = matches!(item["kind"].as_str(), Some("end" | "error"));
        items.push(item);
        if done {
            return items;
        }
    }
}

async fn stream_call(
    server: &Server,
    client: &ClientConnection,
    method: &str,
    params: Value,
) -> Value {
    server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            client,
        )
        .await
}

#[tokio::test]
async fn a_server_streaming_call_pushes_each_reply_then_end() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let (client, mut inbox) = ClientConnection::with_inbox(1);
    let started = stream_call(
        &server,
        &client,
        "grpc.stream.start",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "Countdown",
               "request": {"name": "t"}}),
    )
    .await;
    assert_eq!(ok(&started)["client_streaming"], false);
    let subscription = ok(&started)["subscription"].clone();

    let items = drain_stream(&mut inbox).await;
    let messages: Vec<_> = items
        .iter()
        .filter(|i| i["kind"] == "message")
        .map(|i| i["message"]["message"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(messages, ["t #1", "t #2", "t #3"]);
    assert_eq!(items[0]["kind"], "headers");
    assert_eq!(items.last().unwrap()["kind"], "end");
    assert!(items.iter().all(|i| i["subscription"] == subscription));

    let stopped = stream_call(
        &server,
        &client,
        "grpc.stream.stop",
        json!({"subscription": subscription}),
    )
    .await;
    assert_eq!(ok(&stopped)["stopped"], true);
}

#[tokio::test]
async fn a_client_streaming_call_replies_once_after_close() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let (client, mut inbox) = ClientConnection::with_inbox(1);
    let started = stream_call(
        &server,
        &client,
        "grpc.stream.start",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "Collect",
               "request": {"name": "a"}}),
    )
    .await;
    assert_eq!(ok(&started)["client_streaming"], true);
    let subscription = ok(&started)["subscription"].clone();
    for name in ["b", "c"] {
        ok(&stream_call(
            &server,
            &client,
            "grpc.stream.send",
            json!({"subscription": subscription, "message": {"name": name}}),
        )
        .await);
    }
    ok(&stream_call(
        &server,
        &client,
        "grpc.stream.close",
        json!({"subscription": subscription}),
    )
    .await);
    // Sending after the half-close is refused.
    err(&stream_call(
        &server,
        &client,
        "grpc.stream.send",
        json!({"subscription": subscription, "message": {"name": "d"}}),
    )
    .await);

    let items = drain_stream(&mut inbox).await;
    let reply = items.iter().find(|i| i["kind"] == "message").unwrap();
    assert_eq!(reply["message"], json!({"message": "a+b+c", "length": 3}));
    assert_eq!(items.last().unwrap()["kind"], "end");
}

#[tokio::test]
async fn a_bidi_call_echoes_while_it_stays_open() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let (client, mut inbox) = ClientConnection::with_inbox(1);
    let started = stream_call(
        &server,
        &client,
        "grpc.stream.start",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "Chat"}),
    )
    .await;
    let subscription = ok(&started)["subscription"].clone();
    ok(&stream_call(
        &server,
        &client,
        "grpc.stream.send",
        json!({"subscription": subscription, "message": {"name": "one"}}),
    )
    .await);

    // Skip headers/the `sent` echo until the reply for "one" shows up.
    let reply = loop {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), inbox.recv())
            .await
            .expect("no reply")
            .unwrap();
        let note: Value = serde_json::from_str(&line).unwrap();
        if note["params"]["kind"] == "message" {
            break note["params"]["message"].clone();
        }
    };
    assert_eq!(reply["message"], "echo one");

    // Dropping the client's connection tears its streams down.
    server.client_disconnected(&client);
    err(&stream_call(
        &server,
        &client,
        "grpc.stream.send",
        json!({"subscription": subscription, "message": {"name": "two"}}),
    )
    .await);
}

#[tokio::test]
async fn timeout_ms_becomes_a_deadline_the_server_enforces() {
    let addr = spawn_greeter(true).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &addr.to_string());
    ok(&call(&server, "grpc.connect", json!({"connection": "g"})).await);

    let started = std::time::Instant::now();
    let invoked = call(
        &server,
        "grpc.invoke",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "SayHello",
               "request": {"name": "slow"}, "timeout_ms": 200}),
    )
    .await;
    // Go/Java servers answer DEADLINE_EXCEEDED (4); tonic's own server
    // answers CANCELLED (1) -- either way the call ends well before the 3 s
    // the handler sleeps.
    let code = err(&invoked)["data"]["grpc_code"].as_i64().unwrap();
    assert!(code == 1 || code == 4, "unexpected code {code}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));

    err(&call(
        &server,
        "grpc.invoke",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "SayHello",
               "request": {}, "timeout_ms": "soon"}),
    )
    .await);
}

#[tokio::test]
async fn an_https_target_does_a_tls_handshake_and_fails_against_a_plaintext_server() {
    let addr = spawn_greeter(false).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &format!("https://{addr}"));
    let connected = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        call(&server, "grpc.connect", json!({"connection": "g"})),
    )
    .await
    .expect("connect hung");
    err(&connected);
}

struct Pki {
    ca: String,
    server_cert: String,
    server_key: String,
    client_cert: String,
    client_key: String,
}

/// A throwaway CA with a server certificate (for `localhost`) and a client
/// certificate it signed.
fn pki() -> Pki {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "tradar test ca");
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issue = |name: &str| {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec![name.to_string()])
            .unwrap()
            .signed_by(&key, &ca, &ca_key)
            .unwrap();
        (cert.pem(), key.serialize_pem())
    };
    let (server_cert, server_key) = issue("localhost");
    let (client_cert, client_key) = issue("client.test");
    Pki {
        ca: ca.pem(),
        server_cert,
        server_key,
        client_cert,
        client_key,
    }
}

async fn spawn_greeter_tls(pki: &Pki, require_client_cert: bool) -> std::net::SocketAddr {
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let mut tls = ServerTlsConfig::new().identity(Identity::from_pem(
        pki.server_cert.clone(),
        pki.server_key.clone(),
    ));
    if require_client_cert {
        tls = tls.client_ca_root(Certificate::from_pem(pki.ca.clone()));
    }
    let router = TonicServer::builder()
        .tls_config(tls)
        .unwrap()
        .add_service(greeter::greeter_server::GreeterServer::new(GreeterSvc));
    tokio::spawn(async move {
        router.serve_with_incoming(incoming).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

/// Connects to `target` and, if that works, makes one call -- a rejected
/// client certificate can surface at either step.
async fn connect_and_say_hello(target: &str) -> Result<Value, String> {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), target);
    let connected = call(&server, "grpc.connect", json!({"connection": "g"})).await;
    if let Some(e) = connected.get("error") {
        return Err(e["message"].as_str().unwrap_or("?").to_string());
    }
    let proto_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/greeter.proto");
    ok(&call(
        &server,
        "grpc.load_proto",
        json!({"connection": "g", "path": proto_path}),
    )
    .await);
    let invoked = call(
        &server,
        "grpc.invoke",
        json!({"connection": "g", "service": "greeter.Greeter", "method": "SayHello",
               "request": {"name": "tls"}, "timeout_ms": 5000}),
    )
    .await;
    match invoked.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("?").to_string()),
        None => Ok(invoked["result"]["response"].clone()),
    }
}

#[tokio::test]
async fn tls_with_a_private_ca_and_a_domain_override() {
    let pki = pki();
    let addr = spawn_greeter_tls(&pki, false).await;
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, &pki.ca).unwrap();

    // The CA is what makes the certificate trusted; `domain` is the name on
    // it (we dial 127.0.0.1).
    let target = format!("https://{addr}?ca={}&domain=localhost", ca.display());
    let reply = connect_and_say_hello(&target).await.unwrap();
    assert_eq!(reply["message"], "Hello, tls!");

    // Without the CA the certificate is not trusted.
    let untrusted = format!("https://{addr}?domain=localhost");
    assert!(connect_and_say_hello(&untrusted).await.is_err());
}

#[tokio::test]
async fn mtls_needs_the_client_certificate() {
    let pki = pki();
    let addr = spawn_greeter_tls(&pki, true).await;
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str, content: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, content).unwrap();
        p.display().to_string()
    };
    let (ca, cert, key) = (
        path("ca.pem", &pki.ca),
        path("c.pem", &pki.client_cert),
        path("k.pem", &pki.client_key),
    );

    let without = format!("https://{addr}?ca={ca}&domain=localhost");
    assert!(connect_and_say_hello(&without).await.is_err());

    let with = format!("https://{addr}?ca={ca}&cert={cert}&key={key}&domain=localhost");
    let reply = connect_and_say_hello(&with).await.unwrap();
    assert_eq!(reply["message"], "Hello, tls!");
}

#[tokio::test]
async fn insecure_connects_without_trusting_the_certificate() {
    let pki = pki();
    let addr = spawn_greeter_tls(&pki, false).await;

    // Not trusted: the certificate is signed by a CA nobody has.
    let strict = format!("https://{addr}?domain=localhost");
    assert!(connect_and_say_hello(&strict).await.is_err());

    // `insecure` skips the check -- no CA, and not even the right name.
    let reply = connect_and_say_hello(&format!("https://{addr}?insecure=1"))
        .await
        .unwrap();
    assert_eq!(reply["message"], "Hello, tls!");
    let reply = connect_and_say_hello(&format!("https://{addr}?insecure"))
        .await
        .unwrap();
    assert_eq!(reply["message"], "Hello, tls!");

    // `insecure=0` is the normal, checked connection.
    assert!(
        connect_and_say_hello(&format!("https://{addr}?insecure=0"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn insecure_still_presents_the_client_certificate() {
    let pki = pki();
    let addr = spawn_greeter_tls(&pki, true).await;
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str, content: &str| {
        let p = dir.path().join(name);
        std::fs::write(&p, content).unwrap();
        p.display().to_string()
    };
    let (cert, key) = (
        path("c.pem", &pki.client_cert),
        path("k.pem", &pki.client_key),
    );

    // The server demands a client certificate: skipping the check of *its*
    // certificate does not change that.
    assert!(
        connect_and_say_hello(&format!("https://{addr}?insecure=1"))
            .await
            .is_err()
    );
    let with = format!("https://{addr}?insecure=1&cert={cert}&key={key}");
    let reply = connect_and_say_hello(&with).await.unwrap();
    assert_eq!(reply["message"], "Hello, tls!");
}

#[tokio::test]
async fn connect_reports_when_certificate_checking_is_off() {
    let pki = pki();
    let addr = spawn_greeter_tls(&pki, false).await;
    let dir = tempfile::tempdir().unwrap();
    let server = server_with_grpc(dir.path(), &format!("https://{addr}?insecure=1"));
    let connected = call(&server, "grpc.connect", json!({"connection": "g"})).await;
    assert_eq!(ok(&connected)["insecure"], true);
}
