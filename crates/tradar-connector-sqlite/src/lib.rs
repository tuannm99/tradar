//! SQLite connector: implements `QueryDriver` directly against `sqlx`, and
//! exposes it to `tradar-app` only through `connector()` -- nothing else in
//! this crate is `pub`, so the driver's internals stay this crate's own
//! business, not something the rest of the app can reach into.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Executor, Row, Sqlite, SqlitePool, Transaction, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

/// A raw SQLite connection handle, cached once right after connecting (see
/// `connect()`) so `cancel_query` can call `sqlite3_interrupt` on it
/// directly later -- going around sqlx's own `lock_handle()`, which would
/// have to round-trip through this same connection's dedicated worker
/// thread, and so can't be served while that thread is itself blocked
/// inside the one `sqlite3_step()` call a slow query is stuck in, which is
/// exactly when a cancel is needed. `sqlite3_interrupt` is documented safe
/// to call from any thread at any time against a connection that hasn't
/// been closed, which is why going around sqlx's lock here is sound.
///
/// Valid only as long as the one pooled connection it came from stays
/// open -- this driver forces `max_connections(1)` for exactly this
/// reason (see `connect()`): with more than one physical connection in the
/// pool, a cached handle from just one of them wouldn't reliably be the
/// connection a later query actually runs on.
struct RawHandle(*mut libsqlite3_sys::sqlite3);

// SAFETY: the pointer is read only by passing it straight to
// `libsqlite3_sys::sqlite3_interrupt`, which SQLite's own documentation
// says is safe to call concurrently from any thread against a handle that
// is still open -- see `RawHandle`'s own doc comment.
unsafe impl Send for RawHandle {}
unsafe impl Sync for RawHandle {}

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
    /// Set once by `connect()`, read by `cancel_query` -- see `RawHandle`.
    handle: Option<RawHandle>,
}

impl SqliteDriver {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            pool: None,
            transaction: Mutex::new(None),
            in_transaction: AtomicBool::new(false),
            handle: None,
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

/// Formats a SQLite error the same `LINE N: ... ^` shape
/// `tradar-connector-postgres`'s `format_pg_error` produces, when there's a
/// token in the message to point at. SQLite's own C API reports no
/// structured position the way Postgres's protocol does --
/// `sqlx::sqlite::SqliteError` exposes only a message and an error code,
/// nothing a query's character offset could come from -- but a syntax
/// error's message conventionally quotes the exact offending token as
/// `near "X": ...` (SQLite's own long-standing wording), which is enough
/// to recover *a* position to point at after the fact, via
/// `near_token_marker`. Anything that doesn't match that shape (a
/// constraint violation, "no such table", a dropped connection) falls
/// straight through to `sqlx::Error`'s own `Display` unchanged -- there's
/// no token to search for.
fn format_sqlite_error(error: sqlx::Error, query: &str) -> anyhow::Error {
    let Some(db_error) = error.as_database_error() else {
        return error.into();
    };
    let message = db_error.message().to_string();
    let Some(marker) = near_token_marker(&message, query) else {
        return error.into();
    };
    anyhow::anyhow!("{message}\n{marker}")
}

/// The `LINE N: ... ^` marker for the token SQLite's message quotes as
/// `near "X": ...`, found by the token's first occurrence in `query` --
/// approximate (the real mistake could in principle be a later occurrence
/// of the same token, e.g. a repeated typo), but still the closest thing
/// to a position SQLite's own message offers. `None` when the message
/// isn't shaped that way, or the token it names doesn't actually occur in
/// `query` (nothing to point at either way).
fn near_token_marker(message: &str, query: &str) -> Option<String> {
    let after = message.strip_prefix("near \"")?;
    let end = after.find('"')?;
    let token = &after[..end];
    if token.is_empty() {
        return None;
    }
    let byte_index = query.find(token)?;
    // `line_and_caret` wants a 1-based *character* index, matching
    // Postgres's own convention -- counting chars up to the byte offset
    // `find` returns, so multi-byte UTF-8 ahead of the token doesn't throw
    // the caret off.
    let char_index = query[..byte_index].chars().count() + 1;
    query_driver::line_and_caret(query, char_index)
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
        let result = sqlx::query(query)
            .execute(executor)
            .await
            .map_err(|e| format_sqlite_error(e, query))?;
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
    let mut budget = query_driver::RowBudget::default();
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format_sqlite_error(e, query))?
    {
        if columns.is_empty() {
            columns = row.columns().iter().map(|c| c.name().to_string()).collect();
        }
        if budget.is_full() {
            truncated = true;
            break;
        }
        let cells: Vec<String> = (0..row.len()).map(|i| stringify_column(&row, i)).collect();
        budget.keep(&cells);
        rows.push(cells);
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
        // One connection, not sqlx's own default pool of several -- see
        // `RawHandle`'s doc comment for why `cancel_query` needs every
        // query to land on the exact connection whose raw handle was
        // cached below.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        {
            let mut conn = pool.acquire().await?;
            let mut locked = conn.lock_handle().await?;
            self.handle = Some(RawHandle(locked.as_raw_handle().as_ptr()));
        }
        self.pool = Some(pool);
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

    fn insert_sql(&self, table: &str, values: &[(String, String)]) -> Option<String> {
        Some(query_driver::build_sql_insert(table, values))
    }

    fn edit_source(&self, query: &str) -> Option<String> {
        query_driver::single_table_source(query)
    }

    fn column_sources(&self, query: &str, columns: &[String]) -> Option<Vec<Option<String>>> {
        Some(query_driver::joined_column_sources(query, columns))
    }

    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_core::action::CrudOp,
        columns: &[String],
    ) -> Option<String> {
        Some(query_driver::build_crud_snippet(entry, op, columns))
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

        // SQLite has no information_schema, so columns and foreign keys come
        // from the table-valued `pragma_*` functions. Joined against
        // `sqlite_master` that is one query each for the whole database, not
        // one round trip per table -- the difference between ~3 s and ~0.1 s
        // for a few thousand tables, paid on every connect and `snippet`.
        let mut all_columns: std::collections::HashMap<String, Vec<(String, String, i64)>> =
            std::collections::HashMap::new();
        // `pk` is 0 for an ordinary column and 1-based position within the
        // primary key otherwise -- so any non-zero means "part of the key",
        // which is what the results grid needs to address a row it wants to
        // change.
        let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
            "SELECT m.name, p.name, p.type, p.pk \
             FROM sqlite_master m, pragma_table_info(m.name) p \
             WHERE m.type = 'table' ORDER BY m.rowid, p.cid",
        )
        .fetch_all(pool)
        .await?;
        for (table, name, type_name, pk) in rows {
            all_columns
                .entry(table)
                .or_default()
                .push((name, type_name, pk));
        }
        // Foreign keys -- `to` is the referenced column, `NULL` when the FK
        // targets the referenced table's primary key implicitly rather than
        // naming a column, which this connector doesn't try to resolve
        // further.
        let mut all_fks: std::collections::HashMap<String, Vec<(String, String, Option<String>)>> =
            std::collections::HashMap::new();
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT m.name, f.\"from\", f.\"table\", f.\"to\" \
             FROM sqlite_master m, pragma_foreign_key_list(m.name) f \
             WHERE m.type = 'table' ORDER BY m.rowid, f.id, f.seq",
        )
        .fetch_all(pool)
        .await?;
        for (table, from, target, to) in rows {
            all_fks.entry(table).or_default().push((from, target, to));
        }

        let mut schema = Vec::with_capacity(tables.len());
        for (name,) in tables {
            let columns = all_columns.remove(&name).unwrap_or_default();
            let fks = all_fks.remove(&name).unwrap_or_default();
            schema.push(SchemaInfo {
                name,
                columns: columns
                    .into_iter()
                    .map(|(name, type_name, pk)| {
                        let foreign_key = fks
                            .iter()
                            .find(|(from, _, to)| *from == name && to.is_some())
                            .map(|(_, table, to)| query_driver::ForeignKeyRef {
                                table: table.clone(),
                                column: to.clone().expect("filtered to Some above"),
                            });
                        ColumnInfo {
                            name,
                            type_name,
                            primary_key: pk != 0,
                            foreign_key,
                            indexed: false,
                        }
                    })
                    .collect(),
                kind: None,
                ttl: None,
                // SQLite has no real multi-schema/database concept in the
                // normal case (a single file; multi-db needs `ATTACH
                // DATABASE`, which this connector never does), and no more
                // than one object kind is reported here.
                schema: None,
                object_kind: None,
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

    async fn cancel_query(&self) -> anyhow::Result<()> {
        if let Some(handle) = &self.handle {
            // SAFETY: `handle.0` came from this same (still-open, single)
            // connection's own `as_raw_handle()`; `sqlite3_interrupt` is
            // documented safe to call concurrently from any thread.
            unsafe { libsqlite3_sys::sqlite3_interrupt(handle.0) };
        }
        Ok(())
    }
}

fn stringify_column(row: &SqliteRow, index: usize) -> String {
    use sqlx::Decode;
    let raw = row.try_get_raw(index).expect("valid column index");
    if raw.is_null() {
        return "NULL".to_string();
    }
    // Decoding the value already in hand: `row.try_get` would look the
    // column up and check its type compatibility a second time, per cell.
    match raw.type_info().name() {
        "INTEGER" => <i64 as Decode<Sqlite>>::decode(raw).map(|v| v.to_string()),
        "REAL" => <f64 as Decode<Sqlite>>::decode(raw).map(|v| v.to_string()),
        "TEXT" => <String as Decode<Sqlite>>::decode(raw),
        "BLOB" => Ok("<blob>".to_string()),
        _ => <String as Decode<Sqlite>>::decode(raw),
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
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(SqliteConnector)
}

#[cfg(test)]
mod tests;
