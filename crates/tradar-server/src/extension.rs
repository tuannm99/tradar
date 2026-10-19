//! Connectors served outside `registry`/`QueryDriver` (gRPC, Kafka, Socket,
//! RabbitMQ, HTTP): each owns its own state and answers its own
//! `<driver>.*` methods, so `server.rs` only routes by prefix and never
//! needs a per-connector field, dispatch arm, or feature gate of its own.

// A build with every extension feature off leaves this machinery unused.
#![cfg_attr(
    not(any(
        feature = "grpc",
        feature = "kafka",
        feature = "socket",
        feature = "rabbitmq",
        feature = "http"
    )),
    allow(dead_code, unused_mut)
)]

use async_trait::async_trait;
use serde_json::Value;

use tradar_core::storage::{ConnectionStore, SavedConnection};

use crate::protocol::{METHOD_NOT_FOUND, RpcError};
use crate::server::ClientConnection;

/// What a request handler may need from the server beyond its params.
pub(crate) struct Ctx<'a> {
    pub store: Option<&'a ConnectionStore>,
    /// Only the push-capable extensions (Kafka, Socket) read this.
    #[cfg_attr(
        not(any(feature = "kafka", feature = "socket", feature = "grpc")),
        allow(dead_code)
    )]
    pub client: &'a ClientConnection,
}

impl Ctx<'_> {
    /// The saved connection `name`, which must use `driver`.
    pub fn saved(&self, name: &str, driver: &str) -> Result<SavedConnection, RpcError> {
        let all = match self.store {
            Some(store) => store.load()?,
            None => Vec::new(),
        };
        all.into_iter()
            .find(|c| c.name == name && c.driver == driver)
            .ok_or_else(|| RpcError::app(format!("no saved {driver} connection named `{name}`")))
    }
}

#[async_trait]
pub(crate) trait Extension: Send + Sync {
    /// The saved-connection `driver` id this serves -- also its method
    /// namespace: `kafka.tail.start` reaches the `kafka` extension as
    /// `tail.start`.
    fn driver(&self) -> &'static str;

    async fn handle(&self, method: &str, params: &Value, ctx: &Ctx<'_>) -> Result<Value, RpcError>;

    /// Whether `name` has live state here, for `connections.list`.
    fn is_connected(&self, name: &str) -> bool;

    /// Drops whatever background work `client_id` started and never stopped.
    fn client_disconnected(&self, _client_id: u64) {}

    /// Stops the in-flight call named `query_id`, if this extension has one.
    fn cancel(&self, _query_id: &str) -> bool {
        false
    }
}

pub(crate) fn unknown_method(driver: &str, method: &str) -> RpcError {
    RpcError {
        code: METHOD_NOT_FOUND,
        message: format!("unknown method `{driver}.{method}`"),
        data: None,
    }
}

#[cfg(any(
    feature = "grpc",
    feature = "kafka",
    feature = "rabbitmq",
    feature = "http"
))]
pub(crate) fn not_connected(driver: &str, name: &str) -> RpcError {
    RpcError::app(format!(
        "`{name}` is not connected -- call `{driver}.connect` first"
    ))
}

/// Every extension compiled into this build -- the one place their feature
/// gates live.
#[allow(clippy::vec_init_then_push)]
pub(crate) fn all() -> Vec<Box<dyn Extension>> {
    let mut extensions: Vec<Box<dyn Extension>> = Vec::new();
    #[cfg(feature = "grpc")]
    extensions.push(Box::new(crate::grpc::GrpcExtension::default()));
    #[cfg(feature = "kafka")]
    extensions.push(Box::new(crate::kafka::KafkaExtension::default()));
    #[cfg(feature = "socket")]
    extensions.push(Box::new(crate::socket::SocketExtension::default()));
    #[cfg(feature = "rabbitmq")]
    extensions.push(Box::new(crate::rabbitmq::RabbitExtension::default()));
    #[cfg(feature = "http")]
    extensions.push(Box::new(crate::http::HttpExtension::default()));
    extensions
}
