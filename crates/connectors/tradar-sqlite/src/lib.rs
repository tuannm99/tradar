//! SQLite connector: implements `QueryDriver` directly against `sqlx`, and
//! exposes it to `tradar-app` only through `connector()` -- nothing else in
//! this crate is `pub`, so the driver's internals stay this crate's own
//! business, not something the rest of the app can reach into.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, Executor, Row, Sqlite, SqlitePool, Transaction, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use tradar_connector_api::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct SqliteDriver {
    path: String,
    pool: Option<SqlitePool>,
    /// Held across `execute` calls between a `BEGIN` and its matching
    /// `COMMIT`/`ROLLBACK` -- see `transaction_control`. `None` means every
    /// statement runs straight against the pool and commits on its own,
    /// same as before this existed.
    transaction: Mutex<Option<Transaction<'static, Sqlite>>>,
    /// Mirrors whether `transaction` is currently `Some`, readable without
    /// locking it -- `QueryDriver::in_transaction` is a plain sync method
    /// the UI calls every frame, and locking an async `Mutex` from there
    /// would mean either blocking the draw or making the call async for
    /// every other driver's sake.
    in_transaction: AtomicBool,
}

impl SqliteDriver {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            pool: None,
            transaction: Mutex::new(None),
            in_transaction: AtomicBool::new(false),
        }
    }

    /// Handles a `BEGIN`/`COMMIT`/`ROLLBACK` statement by driving the held
    /// transaction directly, rather than sending the literal text to
    /// SQLite -- see `transaction_control`'s doc comment for why that
    /// wouldn't work against a connection pool anyway. Idempotent: a
    /// `BEGIN` while already in one, or a `COMMIT`/`ROLLBACK` with nothing
    /// open, is a harmless no-op rather than an error -- the UI gates F8/F9
    /// on `in_transaction()`, so a mismatch here means the grid is already
    /// showing stale state, not that the user did anything wrong.
    async fn handle_transaction_control(
        &self,
        control: query_driver::TransactionControl,
    ) -> anyhow::Result<QueryResult> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let mut guard = self.transaction.lock().await;
        match control {
            query_driver::TransactionControl::Begin => {
                if guard.is_none() {
                    *guard = Some(pool.begin().await?);
                    self.in_transaction.store(true, Ordering::Relaxed);
                }
            }
            query_driver::TransactionControl::Commit => {
                if let Some(tx) = guard.take() {
                    tx.commit().await?;
                }
                self.in_transaction.store(false, Ordering::Relaxed);
            }
            query_driver::TransactionControl::Rollback => {
                if let Some(tx) = guard.take() {
                    tx.rollback().await?;
                }
                self.in_transaction.store(false, Ordering::Relaxed);
            }
        }
        Ok(QueryResult::Affected { rows: 0 })
    }
}

/// The body of `execute`, generic over what it runs against -- the pool
/// directly, or a held transaction when one is open. Identical either way
/// from here down; only `execute` itself decides which `Executor` to pass.
async fn run<'e, E>(executor: E, query: &str) -> anyhow::Result<QueryResult>
where
    E: Executor<'e, Database = Sqlite>,
{
    // A write reports how many rows it changed; fetching it as a result
    // set would just yield zero rows and look like a SELECT that matched
    // nothing.
    if !query_driver::returns_rows(query) {
        let result = sqlx::query(query).execute(executor).await?;
        return Ok(QueryResult::Affected {
            rows: result.rows_affected(),
        });
    }

    // Streamed and capped rather than `fetch_all`: the point is to never
    // pull an unbounded result set into memory. One row past the cap is
    // read purely to know whether there were more.
    let mut stream = sqlx::query(query).fetch(executor);
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

#[async_trait]
impl QueryDriver for SqliteDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let options = SqliteConnectOptions::new()
            .filename(&self.path)
            .create_if_missing(true);
        self.pool = Some(SqlitePool::connect_with(options).await?);
        Ok(())
    }

    fn keywords(&self) -> &'static [&'static str] {
        query_driver::SQL_KEYWORDS
    }

    fn split_statements(&self, text: &str) -> Vec<query_driver::Statement> {
        query_driver::split_sql_statements(text)
    }

    fn edit_sql(&self, edit: &query_driver::RowEdit) -> Option<String> {
        Some(query_driver::build_sql_edit(edit))
    }

    fn edit_source(&self, query: &str) -> Option<String> {
        query_driver::single_table_source(query)
    }

    fn crud_snippet(&self, entry: &SchemaInfo, op: tradar_core::action::CrudOp) -> Option<String> {
        Some(query_driver::build_crud_snippet(entry, op))
    }

    async fn ping(&self) -> anyhow::Result<()> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        sqlx::query("SELECT 1").execute(pool).await?;
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let tables: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(pool)
                .await?;

        // SQLite has no information_schema, so columns come from a
        // `PRAGMA` per table. That's one round trip each, which is fine
        // against a local file -- and there's no join to fetch them all at
        // once the way Postgres has.
        let mut schema = Vec::with_capacity(tables.len());
        for (name,) in tables {
            // `pk` is 0 for an ordinary column and 1-based position within
            // the primary key otherwise -- so any non-zero means "part of
            // the key", which is what the results grid needs to address a
            // row it wants to change.
            let columns: Vec<(i64, String, String, i64)> =
                sqlx::query_as("SELECT cid, name, type, pk FROM pragma_table_info($1)")
                    .bind(&name)
                    .fetch_all(pool)
                    .await?;
            schema.push(SchemaInfo {
                name,
                columns: columns
                    .into_iter()
                    .map(|(_, name, type_name, pk)| ColumnInfo {
                        name,
                        type_name,
                        primary_key: pk != 0,
                    })
                    .collect(),
                kind: None,
                ttl: None,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        if let Some(control) = query_driver::transaction_control(query) {
            return self.handle_transaction_control(control).await;
        }

        // A held transaction takes every statement until it's closed --
        // the whole point is that they share one connection and see each
        // other's uncommitted changes. Otherwise, straight to the pool,
        // same as before transactions existed.
        let mut guard = self.transaction.lock().await;
        if let Some(tx) = guard.as_mut() {
            return run(&mut **tx, query).await;
        }
        drop(guard);
        let pool = self.pool.as_ref().expect("connect() must be called first");
        run(pool, query).await
    }

    fn in_transaction(&self) -> bool {
        self.in_transaction.load(Ordering::Relaxed)
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
        tradar_connector_api::with_connect_timeout(&connection.target, driver.connect()).await?;
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
    use tradar_query_workbench::query_engine::QueryOutcome;

    #[test]
    fn crud_snippet_delegates_to_the_shared_sql_builder() {
        let driver = SqliteDriver::new("test.db");
        let entry = SchemaInfo::new("users");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
        );
    }

    #[tokio::test]
    async fn a_transaction_through_query_engine_does_not_get_stuck_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let mut engine = QueryEngine::new(
            driver,
            SavedConnection {
                name: "test".to_string(),
                driver: "sqlite".to_string(),
                target: path.to_str().unwrap().to_string(),
            },
            Ok(Vec::new()),
        );

        async fn settle(engine: &mut QueryEngine) {
            for _ in 0..10_000 {
                tokio::task::yield_now().await;
                engine.tick();
                if !engine.is_pending() {
                    return;
                }
            }
            panic!("query never settled");
        }

        engine.submit_query("BEGIN".to_string());
        settle(&mut engine).await;
        assert!(
            !engine.is_pending(),
            "BEGIN's outcome must be drained, not leave `pending` stuck"
        );

        engine.submit_query("INSERT INTO users VALUES (1)".to_string());
        settle(&mut engine).await;
        assert!(
            !engine.is_pending(),
            "the insert inside the transaction must also settle"
        );
        match engine.take_outcome() {
            Some(QueryOutcome::Completed {
                result: QueryResult::Affected { rows },
            }) => assert_eq!(rows, 1, "the insert must report the row it actually added"),
            Some(QueryOutcome::Completed { .. }) => panic!("expected an Affected result"),
            Some(QueryOutcome::Failed { error }) => panic!("insert failed: {error}"),
            None => panic!("no outcome to take"),
        }
    }

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
    async fn list_schema_marks_the_primary_key_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            .execute(driver.pool.as_ref().unwrap())
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let key: Vec<&str> = schema[0]
            .columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            key,
            vec!["id"],
            "without this the results grid can't build a WHERE clause"
        );
    }

    #[tokio::test]
    async fn a_composite_primary_key_reports_every_one_of_its_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        sqlx::query(
            "CREATE TABLE memberships (user_id INTEGER, group_id INTEGER, role TEXT, \
             PRIMARY KEY (user_id, group_id))",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let key: Vec<&str> = schema[0]
            .columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(key, vec!["user_id", "group_id"]);
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
    async fn a_result_bigger_than_the_cap_is_truncated_rather_than_fully_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        // A recursive CTE is an unbounded source without needing a huge
        // table on disk -- exactly the shape that used to be able to
        // exhaust memory.
        let over_the_cap = query_driver::MAX_ROWS + 500;
        let result = driver
            .execute(&format!(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {over_the_cap}) SELECT i FROM n"
            ))
            .await
            .unwrap();

        match result {
            QueryResult::Table {
                rows, truncated, ..
            } => {
                assert_eq!(rows.len(), query_driver::MAX_ROWS);
                assert!(truncated, "the user has to be told this isn't everything");
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_result_inside_the_cap_is_not_marked_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        let result = driver
            .execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10) SELECT i FROM n")
            .await
            .unwrap();

        match result {
            QueryResult::Table {
                rows, truncated, ..
            } => {
                assert_eq!(rows.len(), 10);
                assert!(!truncated);
            }
            other => panic!("expected a table, got {other:?}"),
        }
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
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn a_fresh_driver_is_not_in_a_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        assert!(!driver.in_transaction());
    }

    #[tokio::test]
    async fn begin_opens_a_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        driver.execute("BEGIN").await.unwrap();

        assert!(driver.in_transaction());
    }

    #[tokio::test]
    async fn commit_closes_the_transaction_and_keeps_the_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();

        driver.execute("BEGIN").await.unwrap();
        driver
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();
        driver.execute("COMMIT").await.unwrap();

        assert!(!driver.in_transaction());
        let result = driver.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn rollback_closes_the_transaction_and_discards_the_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();

        driver.execute("BEGIN").await.unwrap();
        driver
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();
        driver.execute("ROLLBACK").await.unwrap();

        assert!(!driver.in_transaction());
        let result = driver.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            result,
            // Column names come from the first row streamed back; a select
            // with nothing to return never sees one, same pre-existing
            // behavior `a_select_that_matches_nothing_is_still_an_empty_table`
            // already covers -- not something this change alters.
            QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
            "the insert must not have survived the rollback"
        );
    }

    #[tokio::test]
    async fn a_second_begin_while_already_in_a_transaction_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();
        driver.execute("BEGIN").await.unwrap();
        driver
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();

        // A second `BEGIN` must not open a fresh transaction and orphan the
        // one already holding the uncommitted insert.
        driver.execute("BEGIN").await.unwrap();
        driver.execute("COMMIT").await.unwrap();

        let result = driver.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            },
            "the insert from before the second BEGIN must still have committed"
        );
    }

    #[tokio::test]
    async fn commit_with_nothing_open_is_a_harmless_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        let result = driver.execute("COMMIT").await;

        assert!(result.is_ok());
        assert!(!driver.in_transaction());
    }

    #[tokio::test]
    async fn rollback_with_nothing_open_is_a_harmless_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut driver = SqliteDriver::new(path.to_str().unwrap());
        driver.connect().await.unwrap();

        let result = driver.execute("ROLLBACK").await;

        assert!(result.is_ok());
        assert!(!driver.in_transaction());
    }

    #[tokio::test]
    async fn an_uncommitted_insert_is_invisible_to_another_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut writer = SqliteDriver::new(path.to_str().unwrap());
        writer.connect().await.unwrap();
        writer
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();
        let mut reader = SqliteDriver::new(path.to_str().unwrap());
        reader.connect().await.unwrap();

        writer.execute("BEGIN").await.unwrap();
        writer
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();

        // The whole point of holding one connection for the transaction:
        // a second, independent connection to the same file must not see
        // the insert until it's committed.
        let seen_before_commit = reader.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            seen_before_commit,
            // No column names for a zero-row result -- see the comment in
            // `rollback_closes_the_transaction_and_discards_the_change`.
            QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            }
        );

        writer.execute("COMMIT").await.unwrap();
        let seen_after_commit = reader.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            seen_after_commit,
            QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            }
        );
    }
}
