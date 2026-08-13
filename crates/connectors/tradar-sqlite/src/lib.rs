//! SQLite connector: implements `QueryDriver` directly against `sqlx`, and
//! exposes it to `tradar-app` only through `connector()` -- nothing else in
//! this crate is `pub`, so the driver's internals stay this crate's own
//! business, not something the rest of the app can reach into.

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, Row, SqlitePool, TypeInfo, ValueRef};

use tradar_connector_api::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct SqliteDriver {
    path: String,
    pool: Option<SqlitePool>,
}

impl SqliteDriver {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            pool: None,
        }
    }
}

#[async_trait]
impl QueryDriver for SqliteDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let options = SqliteConnectOptions::new()
            .filename(&self.path)
            .create_if_missing(true);
        self.pool = Some(SqlitePool::connect_with(options).await?);
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(name,)| SchemaInfo { name })
            .collect())
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

fn stringify_column(row: &SqliteRow, index: usize) -> String {
    let raw = row.try_get_raw(index).expect("valid column index");
    if raw.is_null() {
        return "NULL".to_string();
    }
    match raw.type_info().name() {
        "INTEGER" => row.try_get::<i64, _>(index).map(|v| v.to_string()),
        "REAL" => row.try_get::<f64, _>(index).map(|v| v.to_string()),
        "TEXT" => row.try_get::<String, _>(index),
        "BLOB" => Ok("<blob>".to_string()),
        _ => row.try_get::<String, _>(index),
    }
    .unwrap_or_else(|_| "NULL".to_string())
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "sqlite",
    display_name: "SQLite",
    icon: "🗄",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct SqliteConnector;

#[async_trait]
impl Connector for SqliteConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = SqliteDriver::new(&connection.target);
        driver.connect().await?;
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(SqliteConnector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_succeeds_for_a_new_sqlite_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");

        let mut driver = SqliteDriver::new(path.to_str().unwrap());

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn connect_fails_when_parent_directory_does_not_exist() {
        let mut driver = SqliteDriver::new("/no/such/directory/test.db");

        let result = driver.connect().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_schema_returns_created_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
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
    async fn execute_reports_affected_rows_for_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        let created = driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();
        assert_eq!(created, QueryResult::Affected { rows: 0 });

        let inserted = driver
            .execute("INSERT INTO users VALUES (1), (2), (3)")
            .await
            .unwrap();
        assert_eq!(
            inserted,
            QueryResult::Affected { rows: 3 },
            "a write must report what it changed, not an empty table"
        );

        let deleted = driver
            .execute("DELETE FROM users WHERE id > 1")
            .await
            .unwrap();
        assert_eq!(deleted, QueryResult::Affected { rows: 2 });
    }

    #[tokio::test]
    async fn a_select_that_matches_nothing_is_still_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();

        let result = driver.execute("SELECT * FROM users").await.unwrap();

        // The distinction this whole change exists for: no rows found is
        // not the same shape as no rows returned by a write.
        assert!(matches!(result, QueryResult::Table { .. }));
    }

    #[tokio::test]
    async fn execute_returns_columns_and_rows_for_a_select() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
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
