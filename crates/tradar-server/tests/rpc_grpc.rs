//! End-to-end for `grpc.*`: a real toy gRPC server (`tests/fixtures/greeter.proto`)
//! over a real TCP loopback connection, driven through `Server::handle` --
//! same shape as `tests/rpc.rs`'s SQLite tests, but for the connector
//! that bypasses `Connector`/`QueryDriver` entirely (see `src/grpc.rs`'s
//! doc comment). The dynamic-invocation mechanics were de-risked by a
//! throwaway spike first -- see `docs/backlog/grpc-spike-2026-10-12.md`.
#![cfg(feature = "grpc")]

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
        let name = request.into_inner().name;
        if name == "boom" {
            return Err(tonic::Status::invalid_argument("name must not be 'boom'"));
        }
        Ok(tonic::Response::new(greeter::HelloReply {
            message: format!("Hello, {name}!"),
            length: name.len() as i32,
        }))
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
        &json!([{"name": "SayHello", "input": "greeter.HelloRequest", "output": "greeter.HelloReply"}])
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
