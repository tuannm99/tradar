//! The deprecated TUI's side of this connector: its `Connector` (a
//! descriptor plus a `connect` that hands the TUI a `QueryEngine`). Built
//! only with the `tui` feature; `tradar-server` uses `opener()` instead.

pub(crate) use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
pub(crate) use tradar_core::capability::Capability;
pub(crate) use tradar_core::storage::SavedConnection;
pub(crate) use tradar_query_workbench::query_engine::QueryEngine;

use super::*;

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "clickhouse",
    display_name: "ClickHouse",
    icon: "📊",
    capabilities: &[Capability::Query, Capability::Schema],
};

struct ClickHouseConnector;

#[async_trait]
impl Connector for ClickHouseConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = ClickHouseDriver::new(&connection.target);
        tradar_query::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(ClickHouseConnector)
}
