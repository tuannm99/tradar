//! The `QueryDriver` trait every query-shaped database backend implements
//! (Postgres, SQLite, Elasticsearch, Redis, MongoDB today). Code outside a
//! concrete driver module must depend only on this trait, never on a
//! specific driver crate/module directly -- that's what keeps drivers
//! isolated and pluggable.

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    // extended per-database (columns, indexes, etc.) in a later plan
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Documents(Vec<serde_json::Value>),
}

#[async_trait]
pub trait QueryDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;

    /// Render `query` as a shell command that reproduces it against this
    /// driver's backend, if the driver supports export at all (currently
    /// only Elasticsearch, via `curl`). `None` means "not supported" rather
    /// than an error -- most drivers just don't implement this.
    fn export_curl(&self, _query: &str) -> Option<String> {
        None
    }
}
