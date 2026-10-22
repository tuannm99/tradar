use super::*;

/// What rides along with a call besides its message: request headers and an
/// optional deadline.
#[derive(Default)]
pub struct CallOptions {
    pub metadata: MetadataMap,
    pub timeout: Option<std::time::Duration>,
}

impl CallOptions {
    /// Reads `metadata` and `timeout_ms` out of a request's params.
    pub fn from_params(params: &serde_json::Value) -> anyhow::Result<Self> {
        Ok(Self {
            metadata: parse_metadata(params.get("metadata"))?,
            timeout: parse_timeout(params)?,
        })
    }

    pub(crate) fn apply<T>(self, request: &mut Request<T>) {
        *request.metadata_mut() = self.metadata;
        if let Some(timeout) = self.timeout {
            request.set_timeout(timeout);
        }
    }
}

/// `timeout_ms` of a call as the deadline gRPC sends (`grpc-timeout`).
pub fn parse_timeout(params: &serde_json::Value) -> anyhow::Result<Option<std::time::Duration>> {
    match params.get("timeout_ms") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(ms) if ms > 0 => Ok(Some(std::time::Duration::from_millis(ms))),
            _ => anyhow::bail!("`timeout_ms` must be a positive integer (milliseconds)"),
        },
    }
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
    options: CallOptions,
) -> anyhow::Result<Result<UnaryResponse, GrpcError>> {
    let method = get_method(pool, service, method)?;
    if method.is_client_streaming() || method.is_server_streaming() {
        anyhow::bail!(
            "`{}` is a streaming method -- use `grpc.stream.start`, not `grpc.invoke`",
            method.name()
        );
    }
    let request_msg = DynamicMessage::deserialize(method.input(), request_json)
        .map_err(|e| anyhow::anyhow!("request does not match the schema: {e}"))?;

    let path = method_path(&method)?;

    let mut grpc = Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| anyhow::anyhow!("gRPC channel not ready: {e}"))?;
    let codec = DynamicCodec {
        response_desc: method.output(),
    };
    let mut request = Request::new(request_msg);
    options.apply(&mut request);
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

pub(crate) fn method_path(
    method: &MethodDescriptor,
) -> anyhow::Result<tonic::codegen::http::uri::PathAndQuery> {
    format!("/{}/{}", method.parent_service().full_name(), method.name())
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid path: {e}"))
}
