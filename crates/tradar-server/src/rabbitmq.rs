//! RabbitMQ over `tradar-server`: browse queues/exchanges/bindings and
//! peek/publish messages. Unlike Kafka/Socket, RabbitMQ needs **no**
//! push-channel -- the Management HTTP API has no streaming endpoint, so
//! every operation here is a plain request/response, the same shape
//! `grpc.rs` already uses. The Management API's shapes, `parse_target` and the authenticated
//! request come from `tradar-connector-rabbitmq` (its TUI-free `api`
//! module); the calls and the JSON-RPC surface live here.

use serde_json::json;
use tradar_connector_rabbitmq::{
    BindingInfo, ExchangeInfo, MessageInfo, QueueInfo, RabbitTarget, parse_target,
};

/// Cheap to `Clone` (a `reqwest::Client` is an `Arc` handle internally, and
/// `RabbitTarget` is plain owned strings) -- `server.rs` clones one out of
/// its `Mutex<State>` guard before the `.await` that actually calls the
/// Management API, since a `std::sync::MutexGuard` can't itself live
/// across an `.await` point.
#[derive(Clone)]
pub struct RabbitConnection {
    client: reqwest::Client,
    target: RabbitTarget,
}

impl RabbitConnection {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.target.request(&self.client, method, path)
    }
}

/// Probes `/api/overview` -- fails loudly here, like every other
/// connector's own connect, rather than silently on the first real call.
pub async fn connect(target: &str) -> anyhow::Result<RabbitConnection> {
    let parsed = parse_target(target)?;
    let client = reqwest::Client::new();
    let conn = RabbitConnection {
        client,
        target: parsed,
    };
    conn.request(reqwest::Method::GET, "/api/overview")
        .send()
        .await?
        .error_for_status()?;
    Ok(conn)
}

pub async fn queues(conn: &RabbitConnection) -> anyhow::Result<Vec<QueueInfo>> {
    let response = conn
        .request(
            reqwest::Method::GET,
            &format!("/api/queues/{}", conn.target.vhost),
        )
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// The default (nameless) exchange is implicit and not something a user
/// can browse bindings for -- skipped, same as the TUI's own
/// `refresh_exchanges`.
pub async fn exchanges(conn: &RabbitConnection) -> anyhow::Result<Vec<ExchangeInfo>> {
    let response = conn
        .request(
            reqwest::Method::GET,
            &format!("/api/exchanges/{}", conn.target.vhost),
        )
        .send()
        .await?
        .error_for_status()?;
    let all: Vec<ExchangeInfo> = response.json().await?;
    Ok(all.into_iter().filter(|e| !e.name.is_empty()).collect())
}

pub async fn bindings(conn: &RabbitConnection, exchange: &str) -> anyhow::Result<Vec<BindingInfo>> {
    let response = conn
        .request(
            reqwest::Method::GET,
            &format!(
                "/api/exchanges/{}/{exchange}/bindings/source",
                conn.target.vhost
            ),
        )
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// How many messages one peek asks for -- same constant and reasoning as
/// `tradar-connector-rabbitmq`'s own `PEEK_COUNT`.
const PEEK_COUNT: u32 = 50;

pub async fn peek(conn: &RabbitConnection, queue: &str) -> anyhow::Result<Vec<MessageInfo>> {
    let response = conn
        .request(
            reqwest::Method::POST,
            &format!("/api/queues/{}/{queue}/get", conn.target.vhost),
        )
        .json(&json!({
            "count": PEEK_COUNT,
            // Requeues immediately after ack, so peeking repeatedly never
            // drains the queue -- same as the TUI's own `peek_messages`.
            "ackmode": "ack_requeue_true",
            "encoding": "auto",
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

pub async fn publish(
    conn: &RabbitConnection,
    exchange: &str,
    routing_key: &str,
    payload: &str,
) -> anyhow::Result<()> {
    conn.request(
        reqwest::Method::POST,
        &format!("/api/exchanges/{}/{exchange}/publish", conn.target.vhost),
    )
    .json(&json!({
        "properties": {},
        "routing_key": routing_key,
        "payload": payload,
        "payload_encoding": "string",
    }))
    .send()
    .await?
    .error_for_status()?;
    Ok(())
}

/// `rabbitmq.*` -- plain request/response against the Management API, no
/// push state.
#[derive(Default)]
pub struct RabbitExtension {
    connections: std::sync::Mutex<std::collections::HashMap<String, RabbitConnection>>,
}

#[async_trait::async_trait]
impl crate::extension::Extension for RabbitExtension {
    fn driver(&self) -> &'static str {
        "rabbitmq"
    }

    fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().unwrap().contains_key(name)
    }

    fn connected(&self) -> Vec<String> {
        self.connections.lock().unwrap().keys().cloned().collect()
    }

    fn close(&self, name: &str) -> bool {
        self.connections.lock().unwrap().remove(name).is_some()
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

        let name = str_param(params, "connection")?;
        let app = |e: anyhow::Error| RpcError::app(e.to_string());
        // Cloned out (not borrowed): a `std::sync::MutexGuard` can't live
        // across the `.await` every real call below makes.
        let conn = || {
            self.connections
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| not_connected("rabbitmq", name))
        };
        match method {
            "connect" => {
                if self.is_connected(name) {
                    return Ok(json!({"connected": true, "already": true}));
                }
                let saved = ctx.saved(name, "rabbitmq")?;
                let conn = connect(&saved.target).await.map_err(|e| {
                    RpcError::app(format!("could not connect to `{}`: {e}", saved.target))
                })?;
                self.connections
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), conn);
                Ok(json!({"connected": true, "already": false}))
            }
            "disconnect" => {
                let removed = self.close(name);
                Ok(json!({"disconnected": removed}))
            }
            "queues" => Ok(json!(queues(&conn()?).await.map_err(app)?)),
            "exchanges" => Ok(json!(exchanges(&conn()?).await.map_err(app)?)),
            "bindings" => {
                let exchange = str_param(params, "exchange")?;
                Ok(json!(bindings(&conn()?, exchange).await.map_err(app)?))
            }
            "peek" => {
                let queue = str_param(params, "queue")?;
                Ok(json!(peek(&conn()?, queue).await.map_err(app)?))
            }
            "publish" => {
                let exchange = str_param(params, "exchange")?;
                let routing_key = params
                    .get("routing_key")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let payload = str_param(params, "payload")?;
                publish(&conn()?, exchange, routing_key, payload)
                    .await
                    .map_err(app)?;
                Ok(json!({"ok": true}))
            }
            _ => Err(unknown_method("rabbitmq", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_with_credentials_and_no_path_defaults_to_the_default_vhost() {
        let target = parse_target("http://user:password@localhost:15672").unwrap();
        assert_eq!(target.base_url, "http://localhost:15672");
        assert_eq!(
            target.auth,
            Some(("user".to_string(), "password".to_string()))
        );
        assert_eq!(target.vhost, "%2f");
    }

    #[test]
    fn a_url_with_a_custom_vhost_keeps_it() {
        let target = parse_target("http://user:password@localhost:15672/staging").unwrap();
        assert_eq!(target.vhost, "staging");
    }

    #[test]
    fn a_url_without_credentials_has_no_auth() {
        let target = parse_target("http://localhost:15672").unwrap();
        assert_eq!(target.auth, None);
    }

    #[test]
    fn an_unparseable_target_is_a_clear_error_not_a_panic() {
        let error = parse_target("not a url").unwrap_err();
        assert!(error.to_string().contains("not a url"), "{error}");
    }

    #[test]
    fn queue_info_parses_the_fields_the_server_actually_sends_on() {
        let json = r#"{"name": "orders", "messages_ready": 3, "messages_unacknowledged": 1, "consumers": 2, "extra_field_ignored": true}"#;
        let queue: QueueInfo = serde_json::from_str(json).unwrap();
        assert_eq!(queue.name, "orders");
        assert_eq!(queue.messages_ready, 3);
    }

    #[test]
    fn exchange_info_maps_the_reserved_type_field_name() {
        let json = r#"{"name": "orders.topic", "type": "topic"}"#;
        let exchange: ExchangeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(exchange.name, "orders.topic");
        assert_eq!(exchange.kind, "topic");
    }
}
