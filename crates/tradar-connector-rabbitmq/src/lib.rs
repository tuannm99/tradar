//! RabbitMQ connector -- the first connector in this workspace that does
//! *not* implement `QueryDriver`/reuse `tradar-query-workbench`. RabbitMQ
//! has no query language: browsing queues/exchanges and peeking/publishing
//! messages is a different shape ("Screen không bao giờ làm IO" in
//! docs/architecture.md), so this crate implements `Connector`/`Session`
//! directly and builds its own `Screen` (`RabbitScreen`, in `screen.rs`).
//!
//! Talks to the RabbitMQ **Management HTTP API** (via `reqwest`) rather than
//! AMQP -- see "Thiết kế UI: Kafka và RabbitMQ" in docs/architecture.md for
//! why: it reuses a dependency already in the workspace (Elasticsearch),
//! needs no new client crate, and covers browse/peek/publish. The tradeoff
//! is no real-time tail (the Management API has no streaming endpoint) --
//! peeking is polled on demand, not followed like Kafka.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

mod screen;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

pub(crate) use screen::RabbitScreen;

/// How many messages `peek_messages` asks for in one call -- enough to see
/// what's flowing through a queue without pulling down its whole backlog.
const PEEK_COUNT: u32 = 50;

/// Bounded per `tick()` call, same reasoning as every other `Session` --
/// see "Screen không bao giờ làm IO" in docs/architecture.md. RabbitMQ never
/// produces more than one event per user action, so this is a formality
/// here rather than a real firehose limit (that's Kafka).
const MAX_DRAIN_PER_TICK: usize = 64;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QueueInfo {
    pub name: String,
    #[serde(default)]
    pub messages_ready: u64,
    #[serde(default)]
    pub messages_unacknowledged: u64,
    #[serde(default)]
    pub consumers: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExchangeInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BindingInfo {
    pub destination: String,
    #[serde(default)]
    pub routing_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MessageInfo {
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub routing_key: String,
    #[serde(default)]
    pub redelivered: bool,
    #[serde(default)]
    pub payload: String,
}

/// A parsed `target`: everything needed to call the Management API.
#[derive(Debug, Clone)]
pub(crate) struct RabbitTarget {
    pub base_url: String,
    pub auth: Option<(String, String)>,
    /// Already the URL-encoded path segment the Management API expects
    /// (e.g. `%2f` for the default vhost) -- see `parse_target`.
    pub vhost: String,
}

/// Parses `target` (a full URL, e.g.
/// `http://user:password@localhost:15672/%2f`) into what's needed to call
/// the Management API. The vhost is taken as-is from the URL path, already
/// percent-encoded exactly how the Management API wants it in its own URLs
/// -- so `%2f` (default vhost) or a custom vhost name round-trip without
/// this crate needing its own percent-encoding logic.
pub(crate) fn parse_target(target: &str) -> anyhow::Result<RabbitTarget> {
    let url = reqwest::Url::parse(target)
        .map_err(|e| anyhow::anyhow!("invalid RabbitMQ management URL '{target}': {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("'{target}' has no host"))?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let base_url = format!("{}://{host}{port}", url.scheme());
    let auth = if url.username().is_empty() {
        None
    } else {
        Some((
            url.username().to_string(),
            url.password().unwrap_or("").to_string(),
        ))
    };
    let vhost = match url.path().trim_start_matches('/') {
        "" => "%2f".to_string(),
        other => other.to_string(),
    };
    Ok(RabbitTarget {
        base_url,
        auth,
        vhost,
    })
}

pub(crate) enum RabbitEvent {
    Queues(anyhow::Result<Vec<QueueInfo>>),
    Exchanges(anyhow::Result<Vec<ExchangeInfo>>),
    Messages(anyhow::Result<Vec<MessageInfo>>),
    Bindings(anyhow::Result<Vec<BindingInfo>>),
    Published(anyhow::Result<()>),
}

pub struct RabbitSession {
    client: reqwest::Client,
    target: RabbitTarget,
    event_tx: UnboundedSender<RabbitEvent>,
    event_rx: UnboundedReceiver<RabbitEvent>,
    pub(crate) queues: Vec<QueueInfo>,
    pub(crate) exchanges: Vec<ExchangeInfo>,
    pub(crate) messages: Vec<MessageInfo>,
    pub(crate) bindings: Vec<BindingInfo>,
    pub(crate) error: Option<String>,
    pub(crate) publishing: bool,
}

impl RabbitSession {
    fn new(client: reqwest::Client, target: RabbitTarget) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let session = Self {
            client,
            target,
            event_tx,
            event_rx,
            queues: Vec::new(),
            exchanges: Vec::new(),
            messages: Vec::new(),
            bindings: Vec::new(),
            error: None,
            publishing: false,
        };
        session.refresh_queues();
        session.refresh_exchanges();
        session
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.target.base_url);
        let request = self.client.request(method, url);
        match &self.target.auth {
            Some((user, pass)) => request.basic_auth(user, Some(pass)),
            None => request,
        }
    }

    pub(crate) fn refresh_queues(&self) {
        let request = self.request(
            reqwest::Method::GET,
            &format!("/api/queues/{}", self.target.vhost),
        );
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                let response = request.send().await?.error_for_status()?;
                Ok(response.json::<Vec<QueueInfo>>().await?)
            }
            .await;
            let _ = tx.send(RabbitEvent::Queues(result));
        });
    }

    pub(crate) fn refresh_exchanges(&self) {
        let request = self.request(
            reqwest::Method::GET,
            &format!("/api/exchanges/{}", self.target.vhost),
        );
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                let response = request.send().await?.error_for_status()?;
                let all = response.json::<Vec<ExchangeInfo>>().await?;
                // The default (nameless) exchange is implicit and not
                // something a user can browse bindings for -- skip it.
                Ok(all.into_iter().filter(|e| !e.name.is_empty()).collect())
            }
            .await;
            let _ = tx.send(RabbitEvent::Exchanges(result));
        });
    }

    pub(crate) fn peek_messages(&self, queue: &str) {
        let request = self
            .request(
                reqwest::Method::POST,
                &format!("/api/queues/{}/{queue}/get", self.target.vhost),
            )
            .json(&serde_json::json!({
                "count": PEEK_COUNT,
                // Requeues immediately after ack, so peeking repeatedly
                // never drains the queue -- see the module doc comment.
                "ackmode": "ack_requeue_true",
                "encoding": "auto",
            }));
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                let response = request.send().await?.error_for_status()?;
                Ok(response.json::<Vec<MessageInfo>>().await?)
            }
            .await;
            let _ = tx.send(RabbitEvent::Messages(result));
        });
    }

    pub(crate) fn list_bindings(&self, exchange: &str) {
        let request = self.request(
            reqwest::Method::GET,
            &format!(
                "/api/exchanges/{}/{exchange}/bindings/source",
                self.target.vhost
            ),
        );
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                let response = request.send().await?.error_for_status()?;
                Ok(response.json::<Vec<BindingInfo>>().await?)
            }
            .await;
            let _ = tx.send(RabbitEvent::Bindings(result));
        });
    }

    pub(crate) fn publish(&mut self, exchange: &str, routing_key: &str, payload: &str) {
        self.publishing = true;
        let request = self
            .request(
                reqwest::Method::POST,
                &format!("/api/exchanges/{}/{exchange}/publish", self.target.vhost),
            )
            .json(&serde_json::json!({
                "properties": {},
                "routing_key": routing_key,
                "payload": payload,
                "payload_encoding": "string",
            }));
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                request.send().await?.error_for_status()?;
                Ok(())
            }
            .await;
            let _ = tx.send(RabbitEvent::Published(result));
        });
    }
}

impl Session for RabbitSession {
    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            let event = match self.event_rx.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            changed = true;
            match event {
                RabbitEvent::Queues(Ok(queues)) => {
                    self.queues = queues;
                    self.error = None;
                }
                RabbitEvent::Exchanges(Ok(exchanges)) => {
                    self.exchanges = exchanges;
                    self.error = None;
                }
                RabbitEvent::Messages(Ok(messages)) => {
                    self.messages = messages;
                    self.error = None;
                }
                RabbitEvent::Bindings(Ok(bindings)) => {
                    self.bindings = bindings;
                    self.error = None;
                }
                RabbitEvent::Published(Ok(())) => {
                    self.publishing = false;
                    self.error = None;
                }
                RabbitEvent::Queues(Err(e))
                | RabbitEvent::Exchanges(Err(e))
                | RabbitEvent::Messages(Err(e))
                | RabbitEvent::Bindings(Err(e)) => {
                    self.error = Some(e.to_string());
                }
                RabbitEvent::Published(Err(e)) => {
                    self.publishing = false;
                    self.error = Some(e.to_string());
                }
            }
        }
        changed
    }

    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        _restore: Option<&str>,
    ) -> Box<dyn Component> {
        Box::new(RabbitScreen::new(*self, action_tx))
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "rabbitmq",
    display_name: "RabbitMQ",
    icon: "🐇",
    capabilities: &[Capability::Schema, Capability::Publish],
};

struct RabbitConnector;

#[async_trait]
impl Connector for RabbitConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let target = parse_target(&connection.target)?;
        let client = reqwest::Client::new();

        let overview_url = format!("{}/api/overview", target.base_url);
        let mut overview = client.get(&overview_url);
        if let Some((user, pass)) = &target.auth {
            overview = overview.basic_auth(user, Some(pass));
        }
        tradar_connector_spi::with_connect_timeout(&connection.target, async {
            overview
                .send()
                .await?
                .error_for_status()
                .map_err(anyhow::Error::from)?;
            Ok(())
        })
        .await?;

        let session = RabbitSession::new(client, target);
        Ok(Box::new(session))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(RabbitConnector)
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
    fn a_url_with_an_explicit_encoded_default_vhost_is_passed_through() {
        let target = parse_target("http://user:password@localhost:15672/%2f").unwrap();

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
    fn queue_info_parses_the_fields_the_screen_actually_shows() {
        let json = r#"{"name": "orders", "messages_ready": 3, "messages_unacknowledged": 1, "consumers": 2, "extra_field_ignored": true}"#;

        let queue: QueueInfo = serde_json::from_str(json).unwrap();

        assert_eq!(queue.name, "orders");
        assert_eq!(queue.messages_ready, 3);
        assert_eq!(queue.messages_unacknowledged, 1);
        assert_eq!(queue.consumers, 2);
    }

    #[test]
    fn exchange_info_maps_the_reserved_type_field_name() {
        let json = r#"{"name": "orders.topic", "type": "topic"}"#;

        let exchange: ExchangeInfo = serde_json::from_str(json).unwrap();

        assert_eq!(exchange.name, "orders.topic");
        assert_eq!(exchange.kind, "topic");
    }

    #[test]
    fn descriptor_declares_schema_and_publish_but_not_streaming_or_tail() {
        assert_eq!(DESCRIPTOR.id, "rabbitmq");
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Schema));
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Publish));
        assert!(!DESCRIPTOR.capabilities.contains(&Capability::Streaming));
        assert!(!DESCRIPTOR.capabilities.contains(&Capability::Tail));
    }

    mod docker {
        //! Integration tests against a real RabbitMQ, via `testcontainers`
        //! directly -- same reasoning as Cassandra's `mod docker`
        //! (`testcontainers-modules` has no RabbitMQ feature either, and
        //! consistency with the rest of the workspace's connectors beats
        //! saving one dependency).

        use std::time::Duration;

        use testcontainers::core::{IntoContainerPort, WaitFor};
        use testcontainers::runners::AsyncRunner;
        use testcontainers::{ContainerAsync, GenericImage, ImageExt};

        use super::*;

        /// Needs host ports 5672/15672 free -- can't run alongside the
        /// long-lived dev instance from `docker compose up rabbitmq`, same
        /// constraint as Cassandra's docker tests.
        async fn connected_session() -> (ContainerAsync<GenericImage>, RabbitSession) {
            let container = GenericImage::new("rabbitmq", "4-management-alpine")
                .with_wait_for(WaitFor::message_on_stdout("Server startup complete"))
                .with_mapped_port(5672, 5672.tcp())
                .with_mapped_port(15672, 15672.tcp())
                .with_env_var("RABBITMQ_DEFAULT_USER", "user")
                .with_env_var("RABBITMQ_DEFAULT_PASS", "password")
                .with_startup_timeout(Duration::from_secs(90))
                .start()
                .await
                .expect("rabbitmq container must start");

            let target = parse_target("http://user:password@127.0.0.1:15672").unwrap();
            let client = reqwest::Client::new();

            // The management plugin can take a few seconds after the log
            // line above before it actually answers HTTP -- retry rather
            // than assume the very first request lands.
            let mut last_error = None;
            for _ in 0..10 {
                let mut request = client.get(format!("{}/api/overview", target.base_url));
                if let Some((user, pass)) = &target.auth {
                    request = request.basic_auth(user, Some(pass));
                }
                match tokio::time::timeout(Duration::from_secs(5), request.send()).await {
                    Ok(Ok(response)) if response.status().is_success() => {
                        last_error = None;
                        break;
                    }
                    Ok(Ok(response)) => last_error = Some(format!("status {}", response.status())),
                    Ok(Err(e)) => last_error = Some(e.to_string()),
                    Err(_) => last_error = Some("overview request timed out".to_string()),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            if let Some(error) = last_error {
                panic!("rabbitmq management API never became ready: {error}");
            }

            let session = RabbitSession::new(client, target);
            (container, session)
        }

        async fn declare_queue(session: &RabbitSession, queue: &str) {
            let response = session
                .request(
                    reqwest::Method::PUT,
                    &format!("/api/queues/{}/{queue}", session.target.vhost),
                )
                // RabbitMQ 4.x deprecates non-durable, non-exclusive queues
                // by default ("transient_nonexcl_queues") -- durable is the
                // only shape a plain `PUT /api/queues` can declare there.
                .json(&serde_json::json!({ "durable": true, "auto_delete": false }))
                .send()
                .await
                .expect("declare queue request must be sendable");
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                panic!("declaring queue '{queue}' failed: {status} {body}");
            }
        }

        async fn drain_until<T>(
            session: &mut RabbitSession,
            timeout: Duration,
            mut ready: impl FnMut(&RabbitSession) -> Option<T>,
        ) -> T {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                session.tick();
                if let Some(value) = ready(session) {
                    return value;
                }
                if tokio::time::Instant::now() >= deadline {
                    panic!("condition never became true within {timeout:?}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        // One test, one container: two fixed host ports (5672/15672) mean a
        // second concurrently-started container can't bind them, so this
        // covers both the publish/peek round trip and `refresh_queues`
        // rather than racing two `#[tokio::test]`s against the same ports.
        #[tokio::test]
        async fn publish_peek_and_refresh_round_trip_through_a_real_broker() {
            let (_container, mut session) =
                tokio::time::timeout(Duration::from_secs(150), connected_session())
                    .await
                    .expect("container must become ready within 150s");

            declare_queue(&session, "orders").await;
            declare_queue(&session, "notifications").await;

            // Publish via the default exchange (name `""`), routed by queue
            // name -- avoids needing a custom exchange/binding for this test.
            session.publish("", "orders", "hello from tradar");
            drain_until(&mut session, Duration::from_secs(10), |s| {
                (!s.publishing && s.error.is_none()).then_some(())
            })
            .await;

            session.peek_messages("orders");
            let messages = drain_until(&mut session, Duration::from_secs(10), |s| {
                (!s.messages.is_empty()).then(|| s.messages.clone())
            })
            .await;
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].payload, "hello from tradar");
            assert_eq!(messages[0].routing_key, "orders");

            session.refresh_queues();
            let queues = drain_until(&mut session, Duration::from_secs(10), |s| {
                s.queues
                    .iter()
                    .any(|q| q.name == "notifications")
                    .then(|| s.queues.clone())
            })
            .await;
            assert!(queues.iter().any(|q| q.name == "notifications"));
        }
    }
}
