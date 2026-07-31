//! The `Driver` trait every database backend implements.
//!
//! Code outside `drivers/*` (in `app`, `tui`, `query_engine`) must depend
//! only on this trait, never on `drivers::postgres` or `drivers::sqlite`
//! directly — that's what keeps drivers isolated and pluggable.

// Nothing wires up a Driver yet — app/query_engine land in the next plan.
#![allow(dead_code)]

pub mod postgres;
pub mod sqlite;

use async_trait::async_trait;

pub struct SchemaInfo {
    pub name: String,
    // extended per-database (columns, indexes, etc.) in a later plan
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;
}
