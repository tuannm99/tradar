//! PostgreSQL driver. Not yet implemented — connection, schema
//! introspection, and query execution land in a later plan.

use async_trait::async_trait;

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct PostgresDriver;

#[async_trait]
impl Driver for PostgresDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        todo!("postgres connect")
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        todo!("postgres list_schema")
    }

    async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
        todo!("postgres execute")
    }
}
