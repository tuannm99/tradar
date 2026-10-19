//! gRPC: connect, discover service/method, invoke -- fully dynamic, no
//! `.proto`-generated Rust types anywhere in this module. Not a
//! `Connector`/`QueryDriver` (see `lib.rs`'s doc comment on why it bypasses
//! `registry`), so this module owns its own connection state
//! (`server.rs`'s `State::grpc`) instead of going through `Arc<dyn
//! QueryDriver>`.
//!
//! Unary goes through `grpc.invoke`; server-/client-streaming and bidi go
//! through `grpc.stream.*` over the push channel (the same shape as Socket's
//! tail). Both take optional request `metadata` (ASCII headers) and report
//! the response headers (and, for streams, trailers). The dynamic-invocation
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
use tonic::metadata::{KeyAndValueRef, MetadataKey, MetadataMap, MetadataValue};
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
    pub client_streaming: bool,
    pub server_streaming: bool,
}

pub fn list_methods(pool: &DescriptorPool, service: &str) -> anyhow::Result<Vec<MethodInfo>> {
    let service = get_service(pool, service)?;
    Ok(service
        .methods()
        .map(|m| MethodInfo {
            name: m.name().to_string(),
            input: m.input().full_name().to_string(),
            output: m.output().full_name().to_string(),
            client_streaming: m.is_client_streaming(),
            server_streaming: m.is_server_streaming(),
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

/// A JSON object of `name: value` strings into request metadata. Names are
/// lower-cased (gRPC metadata keys are case-insensitive, HTTP/2 wants them
/// lower); `-bin` keys carry binary values and aren't supported.
pub fn parse_metadata(value: Option<&serde_json::Value>) -> anyhow::Result<MetadataMap> {
    let mut map = MetadataMap::new();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(map);
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("`metadata` phải là object {{tên: giá trị}}"))?;
    for (name, value) in object {
        let name = name.trim().to_ascii_lowercase();
        if name.ends_with("-bin") {
            anyhow::bail!("metadata `{name}`: key `-bin` (giá trị nhị phân) chưa được hỗ trợ");
        }
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let key = MetadataKey::from_bytes(name.as_bytes())
            .map_err(|e| anyhow::anyhow!("metadata key `{name}` không hợp lệ: {e}"))?;
        let val = MetadataValue::try_from(text.as_str())
            .map_err(|e| anyhow::anyhow!("metadata `{name}`: giá trị không hợp lệ: {e}"))?;
        map.append(key, val);
    }
    Ok(map)
}

/// Response headers/trailers as a JSON object. A repeated key is joined with
/// `, ` (how HTTP folds them); a binary value shows its base64 form.
pub fn metadata_to_json(map: &MetadataMap) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for entry in map.iter() {
        let (key, value) = match entry {
            KeyAndValueRef::Ascii(k, v) => (
                k.as_str().to_string(),
                v.to_str().unwrap_or("<không đọc được>").to_string(),
            ),
            KeyAndValueRef::Binary(k, v) => (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_encoded_bytes()).into_owned(),
            ),
        };
        match out.get_mut(&key) {
            Some(serde_json::Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                out.insert(key, serde_json::Value::String(value));
            }
        }
    }
    serde_json::Value::Object(out)
}

/// A unary call's response plus the headers that came with it.
pub struct UnaryResponse {
    pub response: serde_json::Value,
    pub headers: serde_json::Value,
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
    metadata: MetadataMap,
) -> anyhow::Result<Result<UnaryResponse, GrpcError>> {
    let method = get_method(pool, service, method)?;
    if method.is_client_streaming() || method.is_server_streaming() {
        anyhow::bail!(
            "`{}` là method streaming -- dùng `grpc.stream.start`, không phải `grpc.invoke`",
            method.name()
        );
    }
    let request_msg = DynamicMessage::deserialize(method.input(), request_json)
        .map_err(|e| anyhow::anyhow!("request không khớp schema: {e}"))?;

    let path = method_path(&method)?;

    let mut grpc = Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| anyhow::anyhow!("kênh gRPC không sẵn sàng: {e}"))?;
    let codec = DynamicCodec {
        response_desc: method.output(),
    };
    let mut request = Request::new(request_msg);
    *request.metadata_mut() = metadata;
    match grpc.unary(request, path, codec).await {
        Ok(response) => {
            let headers = metadata_to_json(response.metadata());
            Ok(Ok(UnaryResponse {
                response: serde_json::to_value(response.into_inner())?,
                headers,
            }))
        }
        Err(status) => Ok(Err(status.into())),
    }
}

fn method_path(
    method: &MethodDescriptor,
) -> anyhow::Result<tonic::codegen::http::uri::PathAndQuery> {
    format!("/{}/{}", method.parent_service().full_name(), method.name())
        .parse()
        .map_err(|e| anyhow::anyhow!("path không hợp lệ: {e}"))
}

/// A running stream (server-streaming, client-streaming or bidi). Cheap to
/// `Clone` so `grpc.stream.send` can take a copy out of the subscription map
/// while the original stays there for `stop`.
#[derive(Clone)]
pub struct StreamHandle(std::sync::Arc<StreamInner>);

struct StreamInner {
    /// `None` once the client half-closed (`close`), or if the method only
    /// streams server -> client.
    tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<DynamicMessage>>>,
    input: MessageDescriptor,
    client_streaming: bool,
    task: tokio::task::JoinHandle<()>,
}

impl StreamHandle {
    pub fn client_streaming(&self) -> bool {
        self.0.client_streaming
    }

    /// Queue one request message on a client-/bidi-streaming call.
    pub fn send(&self, message: serde_json::Value) -> anyhow::Result<()> {
        if !self.0.client_streaming {
            anyhow::bail!("method này chỉ stream server -> client, không gửi thêm message được");
        }
        let msg = DynamicMessage::deserialize(self.0.input.clone(), message)
            .map_err(|e| anyhow::anyhow!("message không khớp schema: {e}"))?;
        match self.0.tx.lock().unwrap().as_ref() {
            Some(tx) => tx
                .send(msg)
                .map_err(|_| anyhow::anyhow!("stream đã kết thúc")),
            None => anyhow::bail!("đã đóng chiều gửi (half-close)"),
        }
    }

    /// Half-close: no more requests, the server's replies keep coming.
    pub fn close(&self) {
        self.0.tx.lock().unwrap().take();
    }

    pub fn stop(&self) {
        self.close();
        self.0.task.abort();
    }
}

/// Starts a streaming call and pushes what happens as `{kind, ...}` items:
/// `headers`, `message` (one response), `trailers`, then `end` -- or `error`
/// (with the gRPC `code`) in place of `end`. `request` is the one request of
/// a server-streaming call, or an optional first message of a client/bidi
/// one.
pub fn start_stream(
    channel: Channel,
    pool: &DescriptorPool,
    service: &str,
    method: &str,
    request: Option<serde_json::Value>,
    metadata: MetadataMap,
    push: impl Fn(serde_json::Value) + Send + Sync + 'static,
) -> anyhow::Result<StreamHandle> {
    use serde_json::json;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    let method = get_method(pool, service, method)?;
    let (client_streaming, server_streaming) =
        (method.is_client_streaming(), method.is_server_streaming());
    if !client_streaming && !server_streaming {
        anyhow::bail!(
            "`{}` là unary -- dùng `grpc.invoke`, không phải `grpc.stream.start`",
            method.name()
        );
    }
    let input = method.input();
    let first = request
        .map(|v| DynamicMessage::deserialize(input.clone(), v))
        .transpose()
        .map_err(|e| anyhow::anyhow!("request không khớp schema: {e}"))?;
    let path = method_path(&method)?;
    let codec = DynamicCodec {
        response_desc: method.output(),
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<DynamicMessage>();
    if client_streaming && let Some(msg) = first.clone() {
        let _ = tx.send(msg);
    }
    let kept_tx = client_streaming.then_some(tx);

    let task = tokio::spawn(async move {
        let fail = |status: Status| {
            push(
                json!({"kind": "error", "code": status.code() as i32, "message": status.message()}),
            );
        };
        let mut grpc = Grpc::new(channel);
        if let Err(e) = grpc.ready().await {
            return fail(Status::unavailable(format!(
                "kênh gRPC không sẵn sàng: {e}"
            )));
        }
        // The three shapes differ only in how the request goes out and
        // whether the reply is one message or a stream.
        let reply = if !client_streaming {
            let mut req = Request::new(first.unwrap_or_else(|| DynamicMessage::new(input)));
            *req.metadata_mut() = metadata;
            grpc.server_streaming(req, path, codec).await
        } else if server_streaming {
            let mut req = Request::new(UnboundedReceiverStream::new(rx));
            *req.metadata_mut() = metadata;
            grpc.streaming(req, path, codec).await
        } else {
            let mut req = Request::new(UnboundedReceiverStream::new(rx));
            *req.metadata_mut() = metadata;
            match grpc.client_streaming(req, path, codec).await {
                Ok(response) => {
                    push(
                        json!({"kind": "headers", "metadata": metadata_to_json(response.metadata())}),
                    );
                    match serde_json::to_value(response.into_inner()) {
                        Ok(message) => push(json!({"kind": "message", "message": message})),
                        Err(e) => return fail(Status::internal(e.to_string())),
                    }
                    return push(json!({"kind": "end"}));
                }
                Err(status) => return fail(status),
            }
        };
        let response = match reply {
            Ok(response) => response,
            Err(status) => return fail(status),
        };
        push(json!({"kind": "headers", "metadata": metadata_to_json(response.metadata())}));
        let mut stream = response.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(message)) => match serde_json::to_value(message) {
                    Ok(message) => push(json!({"kind": "message", "message": message})),
                    Err(e) => return fail(Status::internal(e.to_string())),
                },
                Ok(None) => break,
                Err(status) => return fail(status),
            }
        }
        if let Ok(Some(trailers)) = stream.trailers().await {
            push(json!({"kind": "trailers", "metadata": metadata_to_json(&trailers)}));
        }
        push(json!({"kind": "end"}));
    });

    Ok(StreamHandle(std::sync::Arc::new(StreamInner {
        tx: std::sync::Mutex::new(kept_tx),
        input: method.input(),
        client_streaming,
        task,
    })))
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
pub struct GrpcExtension {
    connections: std::sync::Mutex<std::collections::HashMap<String, GrpcConnection>>,
    running:
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Notify>>>,
    streams: std::sync::Mutex<crate::subscriptions::Subscriptions<StreamHandle>>,
}

impl GrpcExtension {
    fn stream(
        &self,
        params: &serde_json::Value,
    ) -> Result<StreamHandle, crate::protocol::RpcError> {
        let subscription = crate::protocol::str_param(params, "subscription")?;
        self.streams
            .lock()
            .unwrap()
            .get(subscription)
            .cloned()
            .ok_or_else(|| {
                crate::protocol::RpcError::app(format!(
                    "`{subscription}` is not streaming -- call `grpc.stream.start` first"
                ))
            })
    }
}

impl Default for GrpcExtension {
    fn default() -> Self {
        Self {
            connections: Default::default(),
            running: Default::default(),
            streams: std::sync::Mutex::new(crate::subscriptions::Subscriptions::new("grpc")),
        }
    }
}

#[async_trait::async_trait]
impl crate::extension::Extension for GrpcExtension {
    fn driver(&self) -> &'static str {
        "grpc"
    }

    fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().unwrap().contains_key(name)
    }

    fn client_disconnected(&self, client_id: u64) {
        let handles = self.streams.lock().unwrap().remove_client(client_id);
        for handle in handles {
            handle.stop();
        }
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

        // These name a subscription, not a saved connection.
        match method {
            "stream.send" => {
                let handle = self.stream(params)?;
                let message = params.get("message").cloned().unwrap_or(json!({}));
                handle
                    .send(message.clone())
                    .map_err(|e| RpcError::app(e.to_string()))?;
                ctx.client.notify(
                    "grpc.stream.message",
                    json!({"subscription": str_param(params, "subscription")?, "kind": "sent", "message": message}),
                );
                return Ok(json!({"sent": true}));
            }
            "stream.close" => {
                self.stream(params)?.close();
                return Ok(json!({"closed": true}));
            }
            "stream.stop" => {
                let subscription = str_param(params, "subscription")?;
                let handle = self.streams.lock().unwrap().remove(subscription);
                let stopped = handle.is_some();
                if let Some(handle) = handle {
                    handle.stop();
                }
                return Ok(json!({"stopped": stopped}));
            }
            _ => {}
        }

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
                        .map(|m| json!({
                            "name": m.name,
                            "input": m.input,
                            "output": m.output,
                            "client_streaming": m.client_streaming,
                            "server_streaming": m.server_streaming,
                        }))
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
            // Streaming calls (server-/client-streaming, bidi). Replies arrive
            // as `grpc.stream.message` notifications on the caller's socket:
            // `{subscription, kind: headers|message|trailers|sent|end|error, ..}`.
            "stream.start" => {
                let service = str_param(params, "service")?;
                let rpc_method = str_param(params, "method")?;
                let request = params.get("request").filter(|v| !v.is_null()).cloned();
                let metadata = parse_metadata(params.get("metadata"))
                    .map_err(|e| RpcError::app(e.to_string()))?;
                let (channel, pool) = {
                    let connections = self.connections.lock().unwrap();
                    let conn = connections
                        .get(name)
                        .ok_or_else(|| not_connected("grpc", name))?;
                    (conn.channel.clone(), conn.pool.clone())
                };
                let subscription = self.streams.lock().unwrap().next_id();
                let push_client = ctx.client.clone();
                let id = subscription.clone();
                let handle = start_stream(
                    channel,
                    &pool,
                    service,
                    rpc_method,
                    request,
                    metadata,
                    move |mut item| {
                        item["subscription"] = json!(id);
                        push_client.notify("grpc.stream.message", item)
                    },
                )
                .map_err(|e| RpcError::app(e.to_string()))?;
                let client_streaming = handle.client_streaming();
                self.streams.lock().unwrap().insert(
                    subscription.clone(),
                    ctx.client.id,
                    (),
                    handle,
                );
                Ok(json!({"subscription": subscription, "client_streaming": client_streaming}))
            }
            // Only *invokes*; unlike `edit.sql` there is nothing to
            // show-then-run (a gRPC call isn't a statement the client could
            // choose to run later). An optional `query_id` lets a client
            // `cancel` a slow invoke.
            "invoke" => {
                let service = str_param(params, "service")?;
                let rpc_method = str_param(params, "method")?;
                let request = params.get("request").cloned().unwrap_or(json!({}));
                let metadata = parse_metadata(params.get("metadata"))
                    .map_err(|e| RpcError::app(e.to_string()))?;
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
                    result = invoke(channel, &pool, service, rpc_method, request, metadata) => Some(result),
                    () = cancel.notified() => None,
                };
                if let Some(id) = query_id {
                    self.running.lock().unwrap().remove(id);
                }
                match outcome {
                    Some(Ok(Ok(reply))) => {
                        Ok(json!({"response": reply.response, "headers": reply.headers}))
                    }
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
