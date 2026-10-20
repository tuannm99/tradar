//! MySQL/MariaDB connector: implements `QueryDriver` directly against
//! `sqlx`, and exposes it to `tradar-app` only through `connector()` --
//! nothing else in this crate is `pub`. One driver for both: MariaDB speaks
//! the same wire protocol sqlx's `mysql` feature targets, and the
//! `information_schema` queries below are plain standard SQL both servers
//! support identically -- no server-specific branching needed anywhere in
//! this file.

use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlPoolOptions, MySqlRow};
use sqlx::{Column, Executor, MySql, MySqlPool, Row, Transaction, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use tradar_connector_spi::{CONNECT_TIMEOUT, Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct MySqlDriver {
    connection_string: String,
    pool: Option<MySqlPool>,
    /// Held across `execute` calls between a `BEGIN` and its matching
    /// `COMMIT`/`ROLLBACK` -- see `transaction_control`. `None` means every
    /// statement runs straight against the pool and commits on its own,
    /// same as before this existed.
    transaction: Mutex<Option<Transaction<'static, MySql>>>,
    /// Mirrors whether `transaction` is currently `Some`, readable without
    /// locking it -- `QueryDriver::in_transaction` is a plain sync method
    /// the UI calls every frame, and locking an async `Mutex` from there
    /// would mean either blocking the draw or making the call async for
    /// every other driver's sake.
    in_transaction: AtomicBool,
}

impl MySqlDriver {
    fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
            pool: None,
            transaction: Mutex::new(None),
            in_transaction: AtomicBool::new(false),
        }
    }

    /// Handles a `BEGIN`/`COMMIT`/`ROLLBACK` statement by driving the held
    /// transaction directly, rather than sending the literal text to the
    /// server -- see `transaction_control`'s doc comment for why that
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

/// Formats a MySQL/MariaDB error the same `LINE N: ... ^` shape
/// `tradar-connector-postgres`'s `format_pg_error` produces, when there's a
/// token in the message to point at. Neither server reports a structured
/// character position the way Postgres's wire protocol does, but a syntax
/// error's message conventionally quotes the exact offending token as
/// `... right syntax to use near 'X' at line N` -- enough to recover *a*
/// position to point at after the fact, via `near_token_marker`, the same
/// trick `tradar-connector-sqlite`'s `format_sqlite_error` plays against
/// SQLite's differently-quoted `near "X": ...` message. Anything that
/// doesn't match that shape (a constraint violation, "table doesn't exist",
/// a dropped connection) falls straight through to `sqlx::Error`'s own
/// `Display` unchanged -- there's no token to search for.
fn format_mysql_error(error: sqlx::Error, query: &str) -> anyhow::Error {
    let Some(db_error) = error.as_database_error() else {
        return error.into();
    };
    let message = db_error.message().to_string();
    let Some(marker) = near_token_marker(&message, query) else {
        return error.into();
    };
    anyhow::anyhow!("{message}\n{marker}")
}

/// The `LINE N: ... ^` marker for the token a `near 'X' at line N` message
/// quotes, found by the token's first occurrence in `query` -- approximate
/// (the real mistake could in principle be a later occurrence of the same
/// token, e.g. a repeated typo), but still the closest thing to a position
/// MySQL/MariaDB's own message offers; the explicit line number the message
/// also carries is ignored in favor of `line_and_caret` recomputing it from
/// the token's own position, to stay consistent with the Postgres/SQLite
/// callers of that function. `None` when the message isn't shaped that way,
/// or the token it names doesn't actually occur in `query` (nothing to
/// point at either way).
fn near_token_marker(message: &str, query: &str) -> Option<String> {
    let start = message.find("near '")? + "near '".len();
    let after = &message[start..];
    let end = after.find('\'')?;
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
async fn run<'e, E>(executor: E, query: &'e str) -> anyhow::Result<QueryResult>
where
    E: Executor<'e, Database = MySql>,
{
    // A write reports how many rows it changed; fetching it as a result
    // set would just yield zero rows and look like a SELECT that matched
    // nothing.
    if !query_driver::returns_rows(query) {
        let result = sqlx::query(query)
            .execute(executor)
            .await
            .map_err(|e| format_mysql_error(e, query))?;
        return Ok(QueryResult::Affected {
            rows: result.rows_affected(),
        });
    }

    // Streamed and capped rather than `fetch_all`: the point is to never
    // pull an unbounded result set into memory. One row past the cap is
    // read purely to know whether there were more.
    //
    // `raw_sql`, not `query`: it uses MySQL's text protocol (`COM_QUERY`),
    // so every value arrives as the server's own text -- see
    // `stringify_column`.
    let mut stream = sqlx::raw_sql(query).fetch(executor);
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let mut budget = query_driver::RowBudget::default();
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format_mysql_error(e, query))?
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

/// A `SELECT` that matched nothing streams no rows, and the column names
/// only ever arrive with a row -- so the grid would show no header at all.
/// Ask the database to describe the statement (prepare, don't run) instead.
/// Best effort: any failure just leaves the empty result as it was.
async fn with_empty_columns(
    pool: &sqlx::Pool<MySql>,
    query: &str,
    result: anyhow::Result<QueryResult>,
) -> anyhow::Result<QueryResult> {
    match result {
        Ok(QueryResult::Table {
            columns,
            rows,
            truncated,
        }) if columns.is_empty() && rows.is_empty() => {
            let columns = match pool.describe(query).await {
                Ok(described) => described
                    .columns()
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect(),
                Err(_) => columns,
            };
            Ok(QueryResult::Table {
                columns,
                rows,
                truncated,
            })
        }
        other => other,
    }
}

#[async_trait]
impl QueryDriver for MySqlDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        // sqlx's own default here is 30s, which against an unreachable
        // host made the TUI look hung rather than reporting a failed
        // connect -- same fix as `tradar-connector-postgres`'s `connect()`.
        self.pool = Some(
            MySqlPoolOptions::new()
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
        // Every string column is `CAST(.. AS CHAR)`: on recent MySQL the
        // `information_schema` views report them as VARBINARY, which sqlx
        // refuses to decode as `String` -- found by running against a real
        // server (the schema browser and completion came back empty).
        //
        // Tables/views and their columns in one round trip, ordered so the
        // grouping below can just walk the rows -- same join shape as
        // `tradar-connector-postgres`'s `list_schema`, grouped by (schema,
        // name) since two different databases can each have their own
        // "users". Primary keys ride along for free here: unlike Postgres,
        // MySQL's `information_schema.columns` already carries
        // `column_key = 'PRI'` directly, no extra join needed.
        let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT CAST(c.table_schema AS CHAR), CAST(c.table_name AS CHAR), \
                    CAST(t.table_type AS CHAR), CAST(c.column_name AS CHAR), \
                    CAST(c.data_type AS CHAR), CAST(c.column_key AS CHAR) \
             FROM information_schema.columns c \
             JOIN information_schema.tables t \
               ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
             WHERE c.table_schema NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
             ORDER BY c.table_schema, c.table_name, c.ordinal_position",
        )
        .fetch_all(pool)
        .await?;

        // Foreign keys, one more round trip -- unlike Postgres's
        // `information_schema`, MySQL's own `key_column_usage` already
        // carries the referenced (table, column) directly on each FK row,
        // no `table_constraints`/`constraint_column_usage` join needed:
        // `referenced_table_name IS NOT NULL` is exactly what distinguishes
        // a foreign-key entry from the primary/unique-key entries this same
        // view also lists.
        let fk_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT CAST(table_schema AS CHAR), CAST(table_name AS CHAR), \
                    CAST(column_name AS CHAR), CAST(referenced_table_name AS CHAR), \
                    CAST(referenced_column_name AS CHAR) \
             FROM information_schema.key_column_usage \
             WHERE referenced_table_name IS NOT NULL",
        )
        .fetch_all(pool)
        .await?;
        let mut foreign_keys: std::collections::HashMap<
            (String, String, String),
            query_driver::ForeignKeyRef,
        > = std::collections::HashMap::new();
        for (fk_schema, fk_table, fk_column, ref_table, ref_column) in fk_rows {
            foreign_keys.insert(
                (fk_schema, fk_table, fk_column),
                query_driver::ForeignKeyRef {
                    table: ref_table,
                    column: ref_column,
                },
            );
        }

        let mut schema: Vec<SchemaInfo> = Vec::new();
        for (table_schema, table, table_type, column, type_name, column_key) in rows {
            let same_as_last = schema.last().is_some_and(|s| {
                s.schema.as_deref() == Some(table_schema.as_str()) && s.name == table
            });
            if !same_as_last {
                schema.push(SchemaInfo {
                    schema: Some(table_schema.clone()),
                    object_kind: Some(
                        if table_type == "VIEW" {
                            "view"
                        } else {
                            "table"
                        }
                        .to_string(),
                    ),
                    ..SchemaInfo::new(table.clone())
                });
            }
            let foreign_key = foreign_keys
                .get(&(table_schema, table, column.clone()))
                .cloned();
            let entry = schema.last_mut().expect("just pushed");
            entry.columns.push(ColumnInfo {
                name: column,
                type_name,
                primary_key: column_key == "PRI",
                foreign_key,
                indexed: false,
            });
        }

        // Functions/procedures have no columns of their own -- a second,
        // unrelated round trip rather than trying to force them into the
        // tables/columns join above.
        let routines: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT CAST(routine_schema AS CHAR), CAST(routine_name AS CHAR), \
                    CAST(routine_type AS CHAR) \
             FROM information_schema.routines \
             WHERE routine_schema NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
             ORDER BY routine_schema, routine_name",
        )
        .fetch_all(pool)
        .await?;
        for (routine_schema, name, routine_type) in routines {
            schema.push(SchemaInfo {
                schema: Some(routine_schema),
                object_kind: Some(
                    if routine_type == "PROCEDURE" {
                        "procedure"
                    } else {
                        "function"
                    }
                    .to_string(),
                ),
                ..SchemaInfo::new(name)
            });
        }

        // Only the rare same-name-in-two-schemas entry gets qualified --
        // see the function's own doc comment for why a bare name has to
        // stay bare whenever it's already unambiguous.
        query_driver::qualify_colliding_names(&mut schema);
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
        let result = run(pool, query).await;
        with_empty_columns(pool, query, result).await
    }

    fn in_transaction(&self) -> bool {
        self.in_transaction.load(Ordering::Relaxed)
    }
}

/// Renders one cell as text for the results grid. Same rationale as
/// `tradar-connector-postgres`'s `stringify_column`: rows come from
/// `sqlx::raw_sql`, i.e. MySQL's text protocol, where the server sends every
/// value already formatted as text -- what the `mysql` client prints. The
/// binary protocol `sqlx::query()` uses needs a typed decoder per type, and
/// an earlier version of this function listed the common ones and showed
/// `NULL` for the rest -- indistinguishable from a real null, and wrong for
/// `DECIMAL` (where money lives), `ENUM`, `SET`, `BLOB`/`BINARY`, `BIT`,
/// `YEAR` and geometry types.
///
/// Presentation over the raw text: a real SQL null is `NULL`; `BOOLEAN`
/// (`TINYINT(1)`) reads `true`/`false`; bytes that are not valid UTF-8
/// (a `BLOB`, a `BINARY`) show as `0x<hex>` instead of mangled text.
fn stringify_column(row: &MySqlRow, index: usize) -> String {
    let raw = row.try_get_raw(index).expect("valid column index");
    if raw.is_null() {
        return "NULL".to_string();
    }
    // `unchecked`: skip sqlx's "is this Rust type compatible with that
    // column type" gate (which is exactly what rejected DECIMAL/ENUM/SET) --
    // under the text protocol the bytes are the text, whatever the type.
    let Ok(bytes) = row.try_get_unchecked::<&[u8], _>(index) else {
        return "NULL".to_string();
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        return format!("0x{hex}");
    };
    if raw.type_info().name() == "BOOLEAN" {
        return match text {
            "1" => "true".to_string(),
            "0" => "false".to_string(),
            _ => text.to_string(),
        };
    }
    text.to_string()
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "mysql",
    display_name: "MySQL/MariaDB",
    icon: "🐬",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct MySqlConnector;

#[async_trait]
impl Connector for MySqlConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = MySqlDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(MySqlConnector)
}

#[cfg(test)]
mod tests;
