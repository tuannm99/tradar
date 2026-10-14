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
use prost_reflect::{DescriptorPool, DynamicMessage, MethodDescriptor, ServiceDescriptor};
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
