use super::*;

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
                let comments = params
                    .get("comments")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let text = skeleton(&conn.pool, service, rpc_method, comments)
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
                let options =
                    CallOptions::from_params(params).map_err(|e| RpcError::app(e.to_string()))?;
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
                    options,
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
                let options =
                    CallOptions::from_params(params).map_err(|e| RpcError::app(e.to_string()))?;
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
                    result = invoke(channel, &pool, service, rpc_method, request, options) => Some(result),
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
