use super::*;

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

pub(crate) fn get_service(
    pool: &DescriptorPool,
    service: &str,
) -> anyhow::Result<ServiceDescriptor> {
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

pub(crate) fn get_method(
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
    // Source info is what carries the `.proto`'s comments, which the request
    // skeleton shows as hints.
    let mut compiler = protox::Compiler::new([include])?;
    compiler.include_source_info(true).include_imports(true);
    compiler.open_file(path)?;
    let fds = compiler.file_descriptor_set();
    let before: std::collections::HashSet<String> = list_services(pool).into_iter().collect();
    pool.add_file_descriptor_set(fds)?;
    Ok(list_services(pool)
        .into_iter()
        .filter(|name| !before.contains(name))
        .collect())
}
