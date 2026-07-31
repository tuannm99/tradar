//! SQLite driver. Not yet implemented — connection, schema
//! introspection, and query execution land in a later plan.

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, Row, SqlitePool, TypeInfo, ValueRef};

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct SqliteDriver {
    path: String,
    pool: Option<SqlitePool>,
}

impl SqliteDriver {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            pool: None,
        }
    }
}

#[async_trait]
impl Driver for SqliteDriver {
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
        let rows = sqlx::query(query).fetch_all(pool).await?;

        let columns = rows
            .first()
            .map(|row| row.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let rows = rows
            .iter()
            .map(|row| (0..row.len()).map(|i| stringify_column(row, i)).collect())
            .collect();

        Ok(QueryResult { columns, rows })
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

        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.rows, vec![vec!["1".to_string(), "Ada".to_string()]]);
    }
}
