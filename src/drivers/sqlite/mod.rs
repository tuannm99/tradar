//! SQLite driver. Not yet implemented — connection, schema
//! introspection, and query execution land in a later plan.

use async_trait::async_trait;

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct SqliteDriver;

#[async_trait]
impl Driver for SqliteDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        todo!("sqlite connect")
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        todo!("sqlite list_schema")
    }

    async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
        todo!("sqlite execute")
    }
}
