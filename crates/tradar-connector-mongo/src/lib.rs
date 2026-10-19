//! MongoDB connector: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)` with optional chained cursor
//! calls (`.find(...).sort(...).limit(...).skip(...)`, `.count()`), not a
//! real JS engine. Anything outside that shape — `$where`, arbitrary
//! expressions, a chain method this doesn't recognize — is rejected with a
//! clear error rather than guessed at. Two mongosh shell helpers are
//! special-cased on top of that: `use <db>` switches which database
//! `db.<collection>...` targets, and `show dbs`/`show databases` lists
//! every database on the server.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::QueryDriver;
use tradar_query_workbench::query_engine::QueryEngine;

mod convert;
mod db_helpers;
mod driver;
mod keywords;
mod relax;
mod run;
mod shell;
mod snippets;

use driver::*;

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "mongo",
    display_name: "MongoDB",
    icon: "🍃",
    capabilities: &[Capability::Query, Capability::Schema],
};

struct MongoConnector;

#[async_trait]
impl Connector for MongoConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = MongoDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(MongoConnector)
}

#[cfg(test)]
mod tests;
