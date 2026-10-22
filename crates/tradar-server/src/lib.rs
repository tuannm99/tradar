//! `tradar-server`: the connectors and `QueryDriver`s of `tradar`, served
//! over a local socket so an editor (Neovim) can be the UI. See "Server
//! headless" in docs/architecture.md.

mod extension;
#[cfg(feature = "grpc")]
mod grpc;
#[cfg(feature = "http")]
mod http;
#[cfg(feature = "kafka")]
mod kafka;
pub mod logging;
mod protocol;
#[cfg(feature = "rabbitmq")]
mod rabbitmq;
mod server;
#[cfg(feature = "socket")]
mod socket;
#[cfg(any(feature = "kafka", feature = "socket"))]
mod subscriptions;
mod transport;

use std::collections::HashMap;

use tradar_query::DriverOpener;

pub use server::idle::{DEFAULT_IDLE_TTL, parse_ttl, reap_loop};
pub use server::{ClientConnection, Server};
pub use transport::{MAX_SOCKET_PATH, bind, default_socket_path, serve};

/// Every query-language connector compiled into this binary -- same
/// feature-gated shape as `tradar-app`'s own `registry()`.
#[allow(clippy::vec_init_then_push)]
pub fn registry() -> HashMap<String, Box<dyn DriverOpener>> {
    let mut connectors: Vec<Box<dyn DriverOpener>> = Vec::new();
    #[cfg(feature = "postgres")]
    connectors.push(tradar_connector_postgres::opener());
    #[cfg(feature = "mysql")]
    connectors.push(tradar_connector_mysql::opener());
    #[cfg(feature = "sqlite")]
    connectors.push(tradar_connector_sqlite::opener());
    #[cfg(feature = "elasticsearch")]
    connectors.push(tradar_connector_elasticsearch::opener());
    #[cfg(feature = "redis")]
    connectors.push(tradar_connector_redis::opener());
    #[cfg(feature = "mongo")]
    connectors.push(tradar_connector_mongo::opener());
    #[cfg(feature = "cassandra")]
    connectors.push(tradar_connector_cassandra::opener());
    #[cfg(feature = "clickhouse")]
    connectors.push(tradar_connector_clickhouse::opener());
    connectors
        .into_iter()
        .map(|c| (c.id().to_string(), c))
        .collect()
}
