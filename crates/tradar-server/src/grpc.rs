//! gRPC: connect, discover service/method, invoke -- fully dynamic, no
//! `.proto`-generated Rust types anywhere in this module. Not a
//! `Connector`/`QueryDriver` (see `lib.rs`'s doc comment on why it bypasses
//! `registry`), so this module owns its own connection state
//! (`server.rs`'s `State::grpc`) instead of going through `Arc<dyn
//! QueryDriver>`.
//!
//! Scope v1 (see docs/architecture.md, "Chốt lại scope v1" under gRPC):
//! unary only, no request metadata/headers. The dynamic-invocation
//! mechanics here were de-risked by a throwaway spike before this was
//! written -- see `docs/backlog/grpc-spike-2026-10-12.md`.

use std::path::Path;

use bytes::Buf;
use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
    ServiceDescriptor,
};
use tonic::client::Grpc;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::transport::Channel;
use tonic::{Request, Status};

/// A live gRPC target: the transport channel plus whatever schema has been
/// learned about it so far (reflection, and/or `.proto` files loaded by
/// hand). Kept together because `invoke` needs both and a connection with
/// an empty pool (reflection unsupported, no `.proto` loaded yet) is still
/// a valid, connected state -- just one `grpc.services` reports as empty.
pub struct GrpcConnection {
    pub channel: Channel,
    pub pool: DescriptorPool,
}

/// `target` is a bare `host:port` (same convention as Cassandra/Kafka/
/// Socket) or a full `http(s)://` URL typed by hand. Plaintext only for
/// v1 -- TLS is the same deferred non-goal Socket already has.
pub async fn connect(target: &str) -> anyhow::Result<Channel> {
    let uri = if target.contains("://") {
        target.to_string()
    } else {
        format!("http://{target}")
    };
    Ok(Channel::from_shared(uri)?.connect().await?)
}

/// Services a pool knows about, minus the reflection service itself
/// (`build_v1()` on the server being talked to registers its own
/// descriptor, so `ListServices` always reports it too -- not something a
/// user ever wants to invoke).
pub fn list_services(pool: &DescriptorPool) -> Vec<String> {
    let mut names: Vec<String> = pool
        .services()
        .map(|s| s.full_name().to_string())
        .filter(|name| !name.starts_with("grpc.reflection."))
        .collect();
    names.sort_unstable();
    names
}

fn get_service(pool: &DescriptorPool, service: &str) -> anyhow::Result<ServiceDescriptor> {
    pool.get_service_by_name(service)
        .ok_or_else(|| anyhow::anyhow!("không có service `{service}` trong schema đã biết"))
}

pub struct MethodInfo {
    pub name: String,
    pub input: String,
    pub output: String,
}

pub fn list_methods(pool: &DescriptorPool, service: &str) -> anyhow::Result<Vec<MethodInfo>> {
    let service = get_service(pool, service)?;
    Ok(service
        .methods()
        .map(|m| MethodInfo {
            name: m.name().to_string(),
            input: m.input().full_name().to_string(),
            output: m.output().full_name().to_string(),
        })
        .collect())
}

fn get_method(
    pool: &DescriptorPool,
    service: &str,
    method: &str,
) -> anyhow::Result<MethodDescriptor> {
    get_service(pool, service)?
        .methods()
        .find(|m| m.name() == method)
        .ok_or_else(|| anyhow::anyhow!("service `{service}` không có method `{method}`"))
}

/// A request body to start from: `method`'s input message as indented JSON
/// with every field present at its default (`\"\"`, `0`, `false`, the first
/// enum value, `[]`, `{}`, nested messages filled in). Written by hand
/// rather than through `serde_json::Value` so the fields stay in the
/// order the `.proto` declares them. Only the first member of a `oneof` is
/// included (the others would make the request invalid), and a message
/// that contains itself stops being expanded at the second level.
pub fn skeleton(pool: &DescriptorPool, service: &str, method: &str) -> anyhow::Result<String> {
    let method = get_method(pool, service, method)?;
    Ok(message_skeleton(&method.input(), &mut Vec::new(), 0))
}

fn message_skeleton(desc: &MessageDescriptor, stack: &mut Vec<String>, depth: usize) -> String {
    // Well-known types have a fixed JSON shape of their own.
    match desc.full_name() {
        "google.protobuf.Timestamp" => return "\"1970-01-01T00:00:00Z\"".into(),
        "google.protobuf.Duration" => return "\"0s\"".into(),
        "google.protobuf.FieldMask"
        | "google.protobuf.StringValue"
        | "google.protobuf.BytesValue" => {
            return "\"\"".into();
        }
        "google.protobuf.BoolValue" => return "false".into(),
        "google.protobuf.Int32Value"
        | "google.protobuf.UInt32Value"
        | "google.protobuf.Int64Value"
        | "google.protobuf.UInt64Value"
        | "google.protobuf.FloatValue"
        | "google.protobuf.DoubleValue" => return "0".into(),
        "google.protobuf.Value" => return "null".into(),
        "google.protobuf.ListValue" => return "[]".into(),
        "google.protobuf.Struct" | "google.protobuf.Any" | "google.protobuf.Empty" => {
            return "{}".into();
        }
        _ => {}
    }
    let name = desc.full_name().to_string();
    if stack.contains(&name) || depth > 4 {
        return "{}".into();
    }
    stack.push(name);
    let indent = "  ".repeat(depth + 1);
    let mut lines = Vec::new();
    for field in desc.fields() {
        if let Some(oneof) = field.containing_oneof() {
            let first = oneof.fields().next().map(|f| f.number());
            if first != Some(field.number()) {
                continue;
            }
        }
        lines.push(format!(
            "{indent}{}: {}",
            serde_json::Value::String(field.json_name().to_string()),
            field_skeleton(&field, stack, depth)
        ));
    }
    stack.pop();
    if lines.is_empty() {
        return "{}".into();
    }
    format!("{{\n{}\n{}}}", lines.join(",\n"), "  ".repeat(depth))
}

fn field_skeleton(field: &FieldDescriptor, stack: &mut Vec<String>, depth: usize) -> String {
    if field.is_map() {
        return "{}".into();
    }
    if field.is_list() {
        return "[]".into();
    }
    match field.kind() {
        Kind::Double
        | Kind::Float
        | Kind::Int32
        | Kind::Int64
        | Kind::Uint32
        | Kind::Uint64
        | Kind::Sint32
        | Kind::Sint64
        | Kind::Fixed32
        | Kind::Fixed64
        | Kind::Sfixed32
        | Kind::Sfixed64 => "0".into(),
        Kind::Bool => "false".into(),
        Kind::String | Kind::Bytes => "\"\"".into(),
        Kind::Enum(e) => {
            let first = e
                .values()
                .next()
                .map_or(String::new(), |v| v.name().to_string());
            serde_json::Value::String(first).to_string()
        }
        Kind::Message(m) => message_skeleton(&m, stack, depth + 1),
    }
}

/// Server reflection (`grpc.reflection.v1.ServerReflection`): `ListServices`
/// to find what's there, then `FileContainingSymbol` per service to pull
/// its `FileDescriptorProto` (and transitive dependencies) into the pool --
/// no `.proto` file needed. A server that doesn't implement the reflection
/// service fails here; the caller falls back to `load_proto`.
pub async fn discover_via_reflection(channel: Channel) -> anyhow::Result<DescriptorPool> {
    use tonic_reflection::pb::v1::ServerReflectionRequest;
    use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
    use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
    use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;

    let mut client = ServerReflectionClient::new(channel);

    let list_req = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    };
    let mut stream = client
        .server_reflection_info(tokio_stream::iter(vec![list_req]))
        .await?
        .into_inner();
    let list_resp = stream
        .message()
        .await?
        .ok_or_else(|| anyhow::anyhow!("server không trả lời ListServices"))?;
    let Some(MessageResponse::ListServicesResponse(list)) = list_resp.message_response else {
        anyhow::bail!("response không phải ListServicesResponse");
    };

    let mut pool = DescriptorPool::new();
    for service in &list.service {
        if service.name.starts_with("grpc.reflection.") {
            continue;
        }
        let sym_req = ServerReflectionRequest {
            host: String::new(),
            message_request: Some(MessageRequest::FileContainingSymbol(service.name.clone())),
        };
        let mut stream = client
            .server_reflection_info(tokio_stream::iter(vec![sym_req]))
            .await?
            .into_inner();
        let Some(resp) = stream.message().await? else {
            continue;
        };
        let Some(MessageResponse::FileDescriptorResponse(fdr)) = resp.message_response else {
            continue;
        };
        for encoded in fdr.file_descriptor_proto {
            // Same file reached via two services (a shared import) is a
            // skip inside `prost-reflect`, not an error -- see its own doc
            // comment on `decode_file_descriptor_proto`.
            pool.decode_file_descriptor_proto(encoded.as_slice())?;
        }
    }
    Ok(pool)
}

/// Fallback discovery for a server that doesn't implement reflection:
/// parse a `.proto` file straight off disk (pure-Rust via `protox`, no
/// `protoc` binary required) and merge it into the pool. `path`'s parent
/// directory becomes the one include path -- enough for a single
/// self-contained file; a `.proto` that itself imports siblings needs them
/// next to it, same limitation `protox::compile` always has without an
/// explicit include list.
pub fn load_proto(pool: &mut DescriptorPool, path: &str) -> anyhow::Result<Vec<String>> {
    let path = Path::new(path);
    let include = path.parent().unwrap_or_else(|| Path::new("."));
    let fds = protox::compile([path], [include])?;
    let before: std::collections::HashSet<String> = list_services(pool).into_iter().collect();
    pool.add_file_descriptor_set(fds)?;
    Ok(list_services(pool)
        .into_iter()
        .filter(|name| !before.contains(name))
        .collect())
}

/// A gRPC failure, kept apart from `anyhow::Error` so `server.rs` can put
/// the status code in `RpcError::data` the way `from_query_error` already
/// does for a located SQL error -- a client can branch on the code without
/// parsing prose.
pub struct GrpcError {
    pub code: i32,
    pub message: String,
}

impl From<Status> for GrpcError {
    fn from(status: Status) -> Self {
        Self {
            code: status.code() as i32,
            message: status.message().to_string(),
        }
    }
}

/// Build a request from JSON, invoke, return the response as JSON -- the
/// entire point of the spike this module is built on: neither the request
/// nor the response type is known to Rust at compile time, only the
/// `MethodDescriptor` fetched at runtime.
pub async fn invoke(
    channel: Channel,
    pool: &DescriptorPool,
    service: &str,
    method: &str,
    request_json: serde_json::Value,
) -> anyhow::Result<Result<serde_json::Value, GrpcError>> {
    let method = get_method(pool, service, method)?;
    let request_msg = DynamicMessage::deserialize(method.input(), request_json)
        .map_err(|e| anyhow::anyhow!("request không khớp schema: {e}"))?;

    let path = format!("/{}/{}", method.parent_service().full_name(), method.name())
        .parse()
        .map_err(|e| anyhow::anyhow!("path không hợp lệ: {e}"))?;

    let mut grpc = Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| anyhow::anyhow!("kênh gRPC không sẵn sàng: {e}"))?;
    let codec = DynamicCodec {
        response_desc: method.output(),
    };
    match grpc.unary(Request::new(request_msg), path, codec).await {
        Ok(response) => Ok(Ok(serde_json::to_value(response.into_inner())?)),
        Err(status) => Ok(Err(status.into())),
    }
}

/// `tonic::codec::ProstCodec` (the one `tonic`'s own generated clients use)
/// needs `T: Default` to decode -- it builds `T::default()` then merges
/// into it. `DynamicMessage` has no such `Default`: it needs a
/// `MessageDescriptor` at construction time, which differs per method. This
/// codec carries that descriptor instead.
#[derive(Clone)]
struct DynamicCodec {
    response_desc: prost_reflect::MessageDescriptor,
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            desc: self.response_desc.clone(),
        }
    }
}

struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| Status::internal(format!("encode error: {e}")))
    }
}

struct DynamicDecoder {
    desc: prost_reflect::MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        if !src.has_remaining() {
            return Ok(None);
        }
        let msg = DynamicMessage::decode(self.desc.clone(), src)
            .map_err(|e| Status::internal(format!("decode error: {e}")))?;
        Ok(Some(msg))
    }
}

/// `grpc.*` -- a connection (channel + descriptor pool) per saved name, plus
/// the in-flight `invoke`s a client named with `query_id` so `cancel` can
/// reach them.
#[derive(Default)]
pub struct GrpcExtension {
    connections: std::sync::Mutex<std::collections::HashMap<String, GrpcConnection>>,
    running:
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Notify>>>,
}

#[async_trait::async_trait]
impl crate::extension::Extension for GrpcExtension {
    fn driver(&self) -> &'static str {
        "grpc"
    }

    fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().unwrap().contains_key(name)
    }

    /// Best-effort: only drops the client's own wait, there is no gRPC-level
    /// "stop processing this call" (unlike a database's own cancel).
    fn cancel(&self, query_id: &str) -> bool {
        let target = self.running.lock().unwrap().get(query_id).cloned();
        match target {
            Some(notify) => {
                notify.notify_one();
                true
            }
            None => false,
        }
    }

    async fn handle(
        &self,
        method: &str,
        params: &serde_json::Value,
        ctx: &crate::extension::Ctx<'_>,
    ) -> Result<serde_json::Value, crate::protocol::RpcError> {
        use crate::extension::{not_connected, unknown_method};
        use crate::protocol::{RpcError, str_param};
        use serde_json::{Value, json};
        use std::sync::Arc;
        use tokio::sync::Notify;

        let name = str_param(params, "connection")?;
        match method {
            // Opens the channel and tries server reflection; a server that
            // doesn't implement it still connects (`grpc.services` just
            // comes back empty until `grpc.load_proto` fills the pool).
            "connect" => {
                if self.is_connected(name) {
                    return Ok(json!({"connected": true, "already": true}));
                }
                let saved = ctx.saved(name, "grpc")?;
                let channel = connect(&saved.target).await.map_err(|e| {
                    RpcError::app(format!("không kết nối được tới `{}`: {e}", saved.target))
                })?;
                let pool = discover_via_reflection(channel.clone())
                    .await
                    .unwrap_or_default();
                self.connections
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), GrpcConnection { channel, pool });
                Ok(json!({"connected": true, "already": false}))
            }
            "disconnect" => {
                let removed = self.connections.lock().unwrap().remove(name).is_some();
                Ok(json!({"disconnected": removed}))
            }
            "services" => {
                let connections = self.connections.lock().unwrap();
                let conn = connections
                    .get(name)
                    .ok_or_else(|| not_connected("grpc", name))?;
                Ok(json!(list_services(&conn.pool)))
            }
            // Fallback discovery for a server without reflection: parse a
            // `.proto` file on disk and merge it into the connection's pool.
            "load_proto" => {
                let path = str_param(params, "path")?;
                let mut connections = self.connections.lock().unwrap();
                let conn = connections
                    .get_mut(name)
                    .ok_or_else(|| not_connected("grpc", name))?;
                let added = load_proto(&mut conn.pool, path)
                    .map_err(|e| RpcError::app(format!("không parse được `{path}`: {e}")))?;
                Ok(json!({"added": added}))
            }
            "methods" => {
                let service = str_param(params, "service")?;
                let connections = self.connections.lock().unwrap();
                let conn = connections
                    .get(name)
                    .ok_or_else(|| not_connected("grpc", name))?;
                let methods =
                    list_methods(&conn.pool, service).map_err(|e| RpcError::app(e.to_string()))?;
                Ok(json!(
                    methods
                        .iter()
                        .map(|m| json!({"name": m.name, "input": m.input, "output": m.output}))
                        .collect::<Vec<_>>()
                ))
            }
            // A request body to start from, generated from the method's
            // input message -- see `skeleton`.
            "skeleton" => {
                let service = str_param(params, "service")?;
                let rpc_method = str_param(params, "method")?;
                let connections = self.connections.lock().unwrap();
                let conn = connections
                    .get(name)
                    .ok_or_else(|| not_connected("grpc", name))?;
                let text = skeleton(&conn.pool, service, rpc_method)
                    .map_err(|e| RpcError::app(e.to_string()))?;
                Ok(json!({"skeleton": text}))
            }
            // Only *invokes*; unlike `edit.sql` there is nothing to
            // show-then-run (a gRPC call isn't a statement the client could
            // choose to run later). An optional `query_id` lets a client
            // `cancel` a slow invoke.
            "invoke" => {
                let service = str_param(params, "service")?;
                let rpc_method = str_param(params, "method")?;
                let request = params.get("request").cloned().unwrap_or(json!({}));
                let (channel, pool) = {
                    let connections = self.connections.lock().unwrap();
                    let conn = connections
                        .get(name)
                        .ok_or_else(|| not_connected("grpc", name))?;
                    (conn.channel.clone(), conn.pool.clone())
                };

                let query_id = params.get("query_id").and_then(Value::as_str);
                let cancel = Arc::new(Notify::new());
                if let Some(id) = query_id {
                    self.running
                        .lock()
                        .unwrap()
                        .insert(id.to_string(), Arc::clone(&cancel));
                }
                let outcome = tokio::select! {
                    result = invoke(channel, &pool, service, rpc_method, request) => Some(result),
                    () = cancel.notified() => None,
                };
                if let Some(id) = query_id {
                    self.running.lock().unwrap().remove(id);
                }
                match outcome {
                    Some(Ok(Ok(response))) => Ok(json!({"response": response})),
                    Some(Ok(Err(status))) => Err(status.into()),
                    Some(Err(e)) => Err(e.into()),
                    None => Err(RpcError::app("invoke cancelled")),
                }
            }
            _ => Err(unknown_method("grpc", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTO: &str = r#"
syntax = "proto3";
package demo;
import "google/protobuf/timestamp.proto";

enum Level { LEVEL_UNSET = 0; LEVEL_HIGH = 1; }

message Address { string city = 1; Level level = 2; }

message Node { string id = 1; Node next = 2; }

message Req {
  string name = 1;
  int64 big = 2;
  bool flag = 3;
  bytes blob = 4;
  repeated string tags = 5;
  map<string, int32> counts = 6;
  Address address = 7;
  google.protobuf.Timestamp at = 8;
  oneof pick {
    string by_name = 9;
    int32 by_id = 10;
  }
  optional string note = 11;
  Node chain = 12;
}

service Svc { rpc Do (Req) returns (Address); }
"#;

    fn pool() -> DescriptorPool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.proto");
        std::fs::write(&path, PROTO).unwrap();
        let mut pool = DescriptorPool::new();
        load_proto(&mut pool, path.to_str().unwrap()).unwrap();
        pool
    }

    #[test]
    fn skeleton_has_every_field_at_its_default_in_declared_order() {
        let text = skeleton(&pool(), "demo.Svc", "Do").unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "name": "",
                "big": 0,
                "flag": false,
                "blob": "",
                "tags": [],
                "counts": {},
                "address": {"city": "", "level": "LEVEL_UNSET"},
                "at": "1970-01-01T00:00:00Z",
                "byName": "",
                "note": "",
                "chain": {"id": "", "next": {}},
            })
        );
        // Declaration order is kept (a JSON object would not promise it).
        let name = text.find("\"name\"").unwrap();
        let big = text.find("\"big\"").unwrap();
        let chain = text.find("\"chain\"").unwrap();
        assert!(name < big && big < chain, "{text}");
        assert!(
            !text.contains("byId"),
            "only the first oneof member: {text}"
        );
    }

    #[test]
    fn the_skeleton_is_a_request_the_server_would_accept() {
        let pool = pool();
        let text = skeleton(&pool, "demo.Svc", "Do").unwrap();
        let method = get_method(&pool, "demo.Svc", "Do").unwrap();
        let mut deserializer = serde_json::Deserializer::from_str(&text);
        DynamicMessage::deserialize(method.input(), &mut deserializer)
            .expect("the generated skeleton must deserialize into its own message");
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(skeleton(&pool(), "demo.Svc", "Nope").is_err());
        assert!(skeleton(&pool(), "demo.Nope", "Do").is_err());
    }
}
