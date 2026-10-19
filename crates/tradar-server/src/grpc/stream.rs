use super::*;

/// A running stream (server-streaming, client-streaming or bidi). Cheap to
/// `Clone` so `grpc.stream.send` can take a copy out of the subscription map
/// while the original stays there for `stop`.
#[derive(Clone)]
pub struct StreamHandle(std::sync::Arc<StreamInner>);

pub(crate) struct StreamInner {
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
    options: CallOptions,
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
            options.apply(&mut req);
            grpc.server_streaming(req, path, codec).await
        } else if server_streaming {
            let mut req = Request::new(UnboundedReceiverStream::new(rx));
            options.apply(&mut req);
            grpc.streaming(req, path, codec).await
        } else {
            let mut req = Request::new(UnboundedReceiverStream::new(rx));
            options.apply(&mut req);
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
