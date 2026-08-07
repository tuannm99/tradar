//! PostgreSQL driver. Not yet implemented — connection, schema
//! introspection, and query execution land in a later plan.

use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Column, PgPool, Row, TypeInfo, ValueRef};

/// How long to wait for the initial connection before giving up. sqlx's own
/// default (`PgPool::connect`'s `acquire_timeout`) is 30s, which against an
/// unreachable host makes the TUI look hung rather than reporting a fast,
/// clear connection error.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct PostgresDriver {
    connection_string: String,
    pool: Option<PgPool>,
}

impl PostgresDriver {
    pub fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
            pool: None,
        }
    }
}

#[async_trait]
impl Driver for PostgresDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        self.pool = Some(
            PgPoolOptions::new()
                .acquire_timeout(CONNECT_TIMEOUT)
                .connect(&self.connection_string)
                .await?,
        );
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name,)| SchemaInfo { name })
            .collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let rows = sqlx::query(query).fetch_all(pool).await?;

        let columns = rows
            .first()
            .map(|row| row.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let rows = rows
            .iter()
            .map(|row| (0..row.len()).map(|i| stringify_column(row, i)).collect())
            .collect();

        Ok(QueryResult::Table { columns, rows })
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
            }
        );
    }
}
