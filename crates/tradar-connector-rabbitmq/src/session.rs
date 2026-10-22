//! The deprecated TUI's side of this connector: its `Session`, `Connector`
//! and event plumbing. Built only with the `tui` feature; `tradar-server`
//! uses the shared, TUI-free parts in `api` instead.

use crate::api::*;
use async_trait::async_trait;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

pub(crate) use crate::screen::RabbitScreen;

/// How many messages `peek_messages` asks for in one call -- enough to see
/// what's flowing through a queue without pulling down its whole backlog.
const PEEK_COUNT: u32 = 50;

/// Bounded per `tick()` call, same reasoning as every other `Session` --
/// see "Screen không bao giờ làm IO" in docs/architecture.md. RabbitMQ never
/// produces more than one event per user action, so this is a formality
/// here rather than a real firehose limit (that's Kafka).
const MAX_DRAIN_PER_TICK: usize = 64;

pub(crate) enum RabbitEvent {
    Queues(anyhow::Result<Vec<QueueInfo>>),
    Exchanges(anyhow::Result<Vec<ExchangeInfo>>),
    Messages(anyhow::Result<Vec<MessageInfo>>),
    Bindings(anyhow::Result<Vec<BindingInfo>>),
    Published(anyhow::Result<()>),
}

pub struct RabbitSession {
    pub(crate) client: reqwest::Client,
    pub(crate) target: RabbitTarget,
    pub(crate) event_tx: UnboundedSender<RabbitEvent>,
    pub(crate) event_rx: UnboundedReceiver<RabbitEvent>,
    pub(crate) queues: Vec<QueueInfo>,
    pub(crate) exchanges: Vec<ExchangeInfo>,
    pub(crate) messages: Vec<MessageInfo>,
    pub(crate) bindings: Vec<BindingInfo>,
    pub(crate) error: Option<String>,
    pub(crate) publishing: bool,
}

impl RabbitSession {
    pub(crate) fn new(client: reqwest::Client, target: RabbitTarget) -> Self {
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

    pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.target.request(&self.client, method, path)
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
#[path = "tests.rs"]
mod tests;
