//! PostgreSQL connector: implements `QueryDriver` directly against `sqlx`,
//! and exposes it to `tradar-app` only through `connector()` -- nothing
//! else in this crate is `pub`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Column, PgPool, Row, TypeInfo, ValueRef};

use tradar_connector_api::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

/// How long to wait for the initial connection before giving up. sqlx's own
/// default (`PgPool::connect`'s `acquire_timeout`) is 30s, which against an
/// unreachable host makes the TUI look hung rather than reporting a fast,
/// clear connection error.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

struct PostgresDriver {
    connection_string: String,
    pool: Option<PgPool>,
}

impl PostgresDriver {
    fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
            pool: None,
        }
    }
}

#[async_trait]
impl QueryDriver for PostgresDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        self.pool = Some(
            PgPoolOptions::new()
                .acquire_timeout(CONNECT_TIMEOUT)
                .connect(&self.connection_string)
                .await?,
        );
        Ok(())
    }

    fn keywords(&self) -> &'static [&'static str] {
        query_driver::SQL_KEYWORDS
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        // Tables and their columns in one round trip, ordered so the
        // grouping below can just walk the rows: information_schema joins
        // are cheaper than a query per table.
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT t.table_name, c.column_name, c.data_type \
             FROM information_schema.tables t \
             JOIN information_schema.columns c \
               ON c.table_schema = t.table_schema AND c.table_name = t.table_name \
             WHERE t.table_schema = 'public' AND t.table_type = 'BASE TABLE' \
             ORDER BY t.table_name, c.ordinal_position",
        )
        .fetch_all(pool)
        .await?;

        let mut schema: Vec<SchemaInfo> = Vec::new();
        for (table, column, type_name) in rows {
            if schema.last().map(|s| s.name.as_str()) != Some(table.as_str()) {
                schema.push(SchemaInfo::new(table));
            }
            let entry = schema.last_mut().expect("just pushed");
            entry.columns.push(ColumnInfo {
                name: column,
                type_name,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool.as_ref().expect("connect() must be called first");

        // A write reports how many rows it changed; fetching it as a result
        // set would just yield zero rows and look like a SELECT that
        // matched nothing.
        if !query_driver::returns_rows(query) {
            let result = sqlx::query(query).execute(pool).await?;
            return Ok(QueryResult::Affected {
                rows: result.rows_affected(),
            });
        }

        // Streamed and capped rather than `fetch_all`: the point is to
        // never pull an unbounded result set into memory. One row past the
        // cap is read purely to know whether there were more.
        let mut stream = sqlx::query(query).fetch(pool);
        let mut columns: Vec<String> = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut truncated = false;
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row.columns().iter().map(|c| c.name().to_string()).collect();
            }
            if rows.len() == query_driver::MAX_ROWS {
                truncated = true;
                break;
            }
            rows.push((0..row.len()).map(|i| stringify_column(&row, i)).collect());
        }

        Ok(QueryResult::Table {
            columns,
            rows,
            truncated,
        })
    }
}

fn stringify_column(row: &PgRow, index: usize) -> String {
    let raw = row.try_get_raw(index).expect("valid column index");
    if raw.is_null() {
        return "NULL".to_string();
    }
    match raw.type_info().name() {
        "INT2" => row.try_get::<i16, _>(index).map(|v| v.to_string()),
        "INT4" => row.try_get::<i32, _>(index).map(|v| v.to_string()),
        "INT8" => row.try_get::<i64, _>(index).map(|v| v.to_string()),
        "FLOAT4" => row.try_get::<f32, _>(index).map(|v| v.to_string()),
        "FLOAT8" | "NUMERIC" => row.try_get::<f64, _>(index).map(|v| v.to_string()),
        "BOOL" => row.try_get::<bool, _>(index).map(|v| v.to_string()),
        _ => row.try_get::<String, _>(index),
    }
    .unwrap_or_else(|_| "NULL".to_string())
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "postgres",
    display_name: "PostgreSQL",
    icon: "🐘",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct PostgresConnector;

#[async_trait]
impl Connector for PostgresConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = PostgresDriver::new(&connection.target);
        driver.connect().await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(PostgresConnector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[tokio::test]
    async fn connect_fails_quickly_against_an_unreachable_host() {
        // Port 1 is reserved and never has a Postgres server listening -- no
        // Docker/testcontainers needed, connection is refused immediately at
        // the OS level. This is a regression test for PgPool::connect()'s
        // default 30s acquire_timeout, which made a bad connection target
        // look identical to a hung UI.
        let mut driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), driver.connect())
            .await
            .expect("connect() should fail well within 10s, not hang");

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_succeeds_for_a_running_postgres() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let mut driver = PostgresDriver::new(&conn_string);
        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn list_schema_returns_created_tables() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(driver.pool.as_ref().unwrap())
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "users");
    }

    #[tokio::test]
    async fn execute_returns_columns_and_rows_for_a_select() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER, name TEXT)")
            .execute(driver.pool.as_ref().unwrap())
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'Ada')")
            .execute(driver.pool.as_ref().unwrap())
            .await
            .unwrap();

        let result = driver.execute("SELECT id, name FROM users").await.unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string(), "name".to_string()],
                rows: vec![vec!["1".to_string(), "Ada".to_string()]],
                truncated: false,
            }
        );
    }
}
