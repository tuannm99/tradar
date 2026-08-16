//! PostgreSQL connector: implements `QueryDriver` directly against `sqlx`,
//! and exposes it to `tradar-app` only through `connector()` -- nothing
//! else in this crate is `pub`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Column, Executor, PgPool, Postgres, Row, Transaction, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use tradar_connector_spi::{CONNECT_TIMEOUT, Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct PostgresDriver {
    connection_string: String,
    pool: Option<PgPool>,
    /// Held across `execute` calls between a `BEGIN` and its matching
    /// `COMMIT`/`ROLLBACK` -- see `transaction_control`. `None` means every
    /// statement runs straight against the pool and commits on its own,
    /// same as before this existed.
    transaction: Mutex<Option<Transaction<'static, Postgres>>>,
    /// Mirrors whether `transaction` is currently `Some`, readable without
    /// locking it -- `QueryDriver::in_transaction` is a plain sync method
    /// the UI calls every frame, and locking an async `Mutex` from there
    /// would mean either blocking the draw or making the call async for
    /// every other driver's sake.
    in_transaction: AtomicBool,
}

impl PostgresDriver {
    fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
            pool: None,
            transaction: Mutex::new(None),
            in_transaction: AtomicBool::new(false),
        }
    }

    /// Handles a `BEGIN`/`COMMIT`/`ROLLBACK` statement by driving the held
    /// transaction directly, rather than sending the literal text to
    /// Postgres -- see `transaction_control`'s doc comment for why that
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
    E: Executor<'e, Database = Postgres>,
{
    // A write reports how many rows it changed; fetching it as a result
    // set would just yield zero rows and look like a SELECT that matched
    // nothing.
    if !query_driver::returns_rows(query) {
        let result = sqlx::query(query)
            .execute(executor)
            .await
            .map_err(|e| format_pg_error(e, query))?;
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
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format_pg_error(e, query))?
    {
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

/// Formats a Postgres error `psql`-style: the primary message, then the
/// offending line with a `^` under the parser's own cursor position when
/// the server sent one (a syntax error, mainly), then `DETAIL`/`HINT` when
/// present -- both routinely carry the actual useful information (which
/// constraint, which value) that the primary message alone doesn't.
/// Anything that isn't `sqlx::Error::Database` at all (a dropped
/// connection, a pool timeout) has none of this to add, so it falls
/// straight through to `sqlx::Error`'s own `Display` unchanged.
fn format_pg_error(error: sqlx::Error, query: &str) -> anyhow::Error {
    let pg_error = error
        .as_database_error()
        .and_then(|e| e.try_downcast_ref::<sqlx::postgres::PgDatabaseError>());
    let Some(pg_error) = pg_error else {
        return error.into();
    };

    let mut message = pg_error.message().to_string();
    if let Some(sqlx::postgres::PgErrorPosition::Original(position)) = pg_error.position()
        && let Some(marker) = line_and_caret(query, position)
    {
        message.push('\n');
        message.push_str(&marker);
    }
    if let Some(detail) = pg_error.detail() {
        message.push_str("\nDETAIL: ");
        message.push_str(detail);
    }
    if let Some(hint) = pg_error.hint() {
        message.push_str("\nHINT: ");
        message.push_str(hint);
    }
    anyhow::anyhow!(message)
}

/// The line of `query` containing `position` (Postgres' own 1-based
/// **character** index into the exact text that was sent), plus a `^`
/// marker under it -- e.g. `LINE 2:   WHERE bad_col = 1` / `         ^`.
/// `None` when `position` doesn't land inside `query` at all: an
/// internally-generated query (a PL/pgSQL function body) reports a
/// position into *different* text than what this driver sent, which
/// `PgDatabaseError::position`'s `Internal` variant already distinguishes
/// -- `format_pg_error` only ever calls this for `Original`.
fn line_and_caret(query: &str, position: usize) -> Option<String> {
    let index = position.checked_sub(1)?;
    let chars: Vec<char> = query.chars().collect();
    if index >= chars.len() {
        return None;
    }
    let mut line_number = 1;
    let mut line_start = 0;
    for (i, &c) in chars.iter().enumerate().take(index) {
        if c == '\n' {
            line_number += 1;
            line_start = i + 1;
        }
    }
    let line_end = chars[line_start..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |n| line_start + n);
    let line: String = chars[line_start..line_end].iter().collect();

    let prefix = format!("LINE {line_number}: ");
    let caret_offset = index - line_start;
    let caret_line = format!(
        "{}{}^",
        " ".repeat(prefix.chars().count()),
        " ".repeat(caret_offset)
    );
    Some(format!("{prefix}{line}\n{caret_line}"))
}

#[async_trait]
impl QueryDriver for PostgresDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        // sqlx's own default here is 30s, which against an unreachable
        // host made the TUI look hung rather than reporting a failed
        // connect. Set from the shared budget every connector now honours
        // -- and kept on the pool rather than left to the caller's wrapper,
        // because it also bounds acquires made later, mid-query.
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
        // Tables and their columns in one round trip, ordered so the
        // grouping below can just walk the rows: information_schema joins
        // are cheaper than a query per table.
        // The primary-key columns ride along in the same round trip: the
        // results grid needs them to address a row it wants to change, and
        // asking for them separately would mean a second query per table.
        let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
            "SELECT t.table_name, c.column_name, c.data_type, (k.column_name IS NOT NULL) \
             FROM information_schema.tables t \
             JOIN information_schema.columns c \
               ON c.table_schema = t.table_schema AND c.table_name = t.table_name \
             LEFT JOIN ( \
               SELECT kcu.table_schema, kcu.table_name, kcu.column_name \
               FROM information_schema.table_constraints tc \
               JOIN information_schema.key_column_usage kcu \
                 ON kcu.constraint_name = tc.constraint_name \
                AND kcu.table_schema = tc.table_schema \
               WHERE tc.constraint_type = 'PRIMARY KEY' \
             ) k ON k.table_schema = c.table_schema \
               AND k.table_name = c.table_name \
               AND k.column_name = c.column_name \
             WHERE t.table_schema = 'public' AND t.table_type = 'BASE TABLE' \
             ORDER BY t.table_name, c.ordinal_position",
        )
        .fetch_all(pool)
        .await?;

        let mut schema: Vec<SchemaInfo> = Vec::new();
        for (table, column, type_name, primary_key) in rows {
            if schema.last().map(|s| s.name.as_str()) != Some(table.as_str()) {
                schema.push(SchemaInfo::new(table));
            }
            let entry = schema.last_mut().expect("just pushed");
            entry.columns.push(ColumnInfo {
                name: column,
                type_name,
                primary_key,
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

/// Renders one cell as text for the results grid. `sqlx::query()` (the
/// dynamic, non-macro API this driver uses throughout, since it has to run
/// arbitrary user-typed SQL) always fetches results over the *binary*
/// wire protocol -- confirmed in sqlx-postgres's executor, which only
/// drops to the text protocol for a statement with a `None` argument list,
/// and `sqlx::query()` always starts one at `Some(..)` even with nothing
/// bound. That means `String`'s `Decode` impl -- which only declares
/// itself compatible with genuinely text-shaped columns (`TEXT`,
/// `VARCHAR`, `BPCHAR`, `NAME`, `UNKNOWN`, `citext`) -- silently fails
/// `try_get` for every other type, including extremely common ones like
/// `UUID`/`JSON(B)`/timestamps, and the old fallback here swallowed that
/// `Err` into a plain `"NULL"` -- indistinguishable from an actual null
/// value, and wrong for every row. Each of the types below needs its own
/// typed decode (there is no single decoder that works for arbitrary
/// binary-format columns) before it can be turned into text.
///
/// **Known gap**: types not listed here (arrays, enums, `inet`/`macaddr`,
/// `money`, `interval`, `bytea`, ranges, composite/PostGIS types, ...)
/// still fall through to the `String` attempt and still show as `NULL` --
/// covers what a typical schema actually uses, not a claim of
/// completeness. Extend this match arm by arm as a real gap turns up,
/// same as the rest of this list was built.
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
        "UUID" => row
            .try_get::<sqlx::types::Uuid, _>(index)
            .map(|v| v.to_string()),
        "JSON" | "JSONB" => row
            .try_get::<sqlx::types::JsonValue, _>(index)
            .map(|v| v.to_string()),
        "TIMESTAMP" => row
            .try_get::<sqlx::types::chrono::NaiveDateTime, _>(index)
            .map(|v| v.to_string()),
        "TIMESTAMPTZ" => row
            .try_get::<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>, _>(index)
            .map(|v| v.to_string()),
        "DATE" => row
            .try_get::<sqlx::types::chrono::NaiveDate, _>(index)
            .map(|v| v.to_string()),
        "TIME" => row
            .try_get::<sqlx::types::chrono::NaiveTime, _>(index)
            .map(|v| v.to_string()),
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
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
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

    #[test]
    fn crud_snippet_delegates_to_the_shared_sql_builder() {
        let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");
        let entry = SchemaInfo::new("users");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
        );
    }

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

    /// Regression test: `stringify_column`'s old fallback (a plain
    /// `String` decode for anything it didn't special-case) fails outright
    /// for these types under sqlx's binary wire protocol -- and used to
    /// get swallowed into a plain "NULL", indistinguishable from an
    /// actual null and wrong for every row that had one.
    #[tokio::test]
    async fn uuid_jsonb_and_timestamp_columns_decode_to_real_text_not_null() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query(
            "CREATE TABLE events (
                id UUID NOT NULL DEFAULT '3fa85f64-5717-4562-b3fc-2c963f66afa6',
                data JSONB NOT NULL,
                at TIMESTAMP NOT NULL,
                at_tz TIMESTAMPTZ NOT NULL,
                on_day DATE NOT NULL,
                nothing UUID
            )",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events (data, at, at_tz, on_day, nothing) VALUES
             ('{\"code\": \"ok\"}', '2026-08-04 00:46:57', '2026-08-04 00:46:57+00', '2026-08-04', NULL)",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

        let result = driver
            .execute("SELECT id, data, at, at_tz, on_day, nothing FROM events")
            .await
            .unwrap();

        let QueryResult::Table { rows, .. } = result else {
            panic!("expected a Table result");
        };
        let row = &rows[0];
        assert_eq!(row[0], "3fa85f64-5717-4562-b3fc-2c963f66afa6");
        assert_eq!(row[1], "{\"code\":\"ok\"}");
        assert_eq!(row[2], "2026-08-04 00:46:57");
        assert!(row[3].starts_with("2026-08-04 00:46:57"), "was: {}", row[3]);
        assert_eq!(row[4], "2026-08-04");
        assert_eq!(
            row[5], "NULL",
            "an actual null must still say NULL, distinct from a decode failure"
        );
    }

    #[tokio::test]
    async fn a_fresh_driver_is_not_in_a_transaction() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();

        assert!(!driver.in_transaction());
    }

    #[tokio::test]
    async fn commit_closes_the_transaction_and_keeps_the_change() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();

        driver.execute("BEGIN").await.unwrap();
        assert!(driver.in_transaction());
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
    async fn an_uncommitted_insert_is_invisible_to_another_connection() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut writer = PostgresDriver::new(&conn_string);
        writer.connect().await.unwrap();
        writer
            .execute("CREATE TABLE users (id INTEGER)")
            .await
            .unwrap();
        let mut reader = PostgresDriver::new(&conn_string);
        reader.connect().await.unwrap();

        writer.execute("BEGIN").await.unwrap();
        writer
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();

        // The point of holding one pooled connection for the whole
        // transaction: a second, independent connection to the same
        // database must not see the insert until it's committed.
        let seen_before_commit = reader.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            seen_before_commit,
            QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            }
        );

        writer.execute("ROLLBACK").await.unwrap();
        let seen_after_rollback = reader.execute("SELECT * FROM users").await.unwrap();
        assert_eq!(
            seen_after_rollback,
            QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
            "a rolled-back insert must never become visible to anyone"
        );
    }

    /// Where `^` landed relative to the start of the query text on the
    /// line above it, plus that line's own text -- what actually matters,
    /// rather than the exact width of the `LINE N: ` prefix in front of
    /// it.
    fn caret_position(marker: &str) -> (usize, &str) {
        let mut lines = marker.lines();
        let line = lines.next().unwrap();
        let caret_line = lines.next().unwrap();
        let caret_column = caret_line.find('^').unwrap();
        // `line` is `LINE N: <text>` -- the first `": "` marks where
        // `<text>` starts.
        let text_start = line.find(": ").unwrap() + 2;
        (caret_column - text_start, &line[text_start..])
    }

    #[test]
    fn line_and_caret_points_at_a_one_based_position_on_a_single_line() {
        let query = "SELECT * FRO users";
        let position = query.find("FRO").unwrap() + 1; // 1-based index of 'F'

        let marker = line_and_caret(query, position).unwrap();

        let (offset, text) = caret_position(&marker);
        assert_eq!(text, query);
        assert_eq!(offset, query.find("FRO").unwrap());
    }

    #[test]
    fn line_and_caret_finds_the_right_line_in_a_multi_line_query() {
        let query = "SELECT id\nFROM usres\nWHERE id = 1";
        // 1-based char index of the `u` in `usres` (line 2).
        let position = query.find("usres").unwrap() + 1;

        let marker = line_and_caret(query, position).unwrap();

        let (offset, text) = caret_position(&marker);
        assert_eq!(text, "FROM usres");
        assert_eq!(offset, text.find('u').unwrap());
        assert!(marker.starts_with("LINE 2:"), "marker was: {marker}");
    }

    #[test]
    fn line_and_caret_is_none_past_the_end_of_the_query() {
        assert_eq!(line_and_caret("SELECT 1", 100), None);
    }

    #[test]
    fn line_and_caret_is_none_for_position_zero() {
        // Postgres never actually sends 0, but the type is an unsigned
        // int -- this is what "no position" would look like if it did.
        assert_eq!(line_and_caret("SELECT 1", 0), None);
    }

    #[tokio::test]
    async fn a_syntax_error_reports_the_offending_line_with_a_caret() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();

        let error = driver
            .execute("SELECT * FRO users")
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("LINE 1:"), "error was: {error}");
        assert!(error.contains('^'), "error was: {error}");
    }

    #[tokio::test]
    async fn a_constraint_violation_reports_its_detail() {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let mut driver = PostgresDriver::new(&conn_string);
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        driver
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap();

        let error = driver
            .execute("INSERT INTO users VALUES (1)")
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("DETAIL:"), "error was: {error}");
    }
}
