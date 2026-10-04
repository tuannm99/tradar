//! `tradar-server`: the connectors and `QueryDriver`s of `tradar`, served
//! over a local socket so an editor (Neovim) can be the UI. See "Server
//! headless" in docs/architecture.md.

mod protocol;
mod server;
mod transport;

use std::collections::HashMap;

use tradar_connector_spi::Connector;

pub use server::Server;
pub use transport::{bind, default_socket_path, serve};

/// Every query-language connector compiled into this binary -- same
/// feature-gated shape as `tradar-app`'s own `registry()`.
#[allow(clippy::vec_init_then_push)]
pub fn registry() -> HashMap<String, Box<dyn Connector>> {
    let mut connectors: Vec<Box<dyn Connector>> = Vec::new();
    #[cfg(feature = "postgres")]
    connectors.push(tradar_connector_postgres::connector());
    #[cfg(feature = "mysql")]
    connectors.push(tradar_connector_mysql::connector());
    #[cfg(feature = "sqlite")]
    connectors.push(tradar_connector_sqlite::connector());
    #[cfg(feature = "elasticsearch")]
    connectors.push(tradar_connector_elasticsearch::connector());
    #[cfg(feature = "redis")]
    connectors.push(tradar_connector_redis::connector());
    #[cfg(feature = "mongo")]
    connectors.push(tradar_connector_mongo::connector());
    #[cfg(feature = "cassandra")]
    connectors.push(tradar_connector_cassandra::connector());
    #[cfg(feature = "clickhouse")]
    connectors.push(tradar_connector_clickhouse::connector());
    connectors
        .into_iter()
        .map(|c| (c.descriptor().id.to_string(), c))
        .collect()
}
