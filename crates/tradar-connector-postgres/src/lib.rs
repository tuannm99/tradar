//! PostgreSQL connector: implements `QueryDriver` directly against `sqlx`,
//! and exposes it to `tradar-app` only through `connector()` -- nothing
//! else in this crate is `pub`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::postgres::{PgConnection, PgPoolOptions, PgRow};
use sqlx::{Column, Executor, PgPool, Postgres, Row, Transaction, TypeInfo, ValueRef};
use tokio::sync::Mutex;

use tradar_query::CONNECT_TIMEOUT;
use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};

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
    /// The backend pid of whatever `execute` call is running right now
    /// (`SELECT pg_backend_pid()`, fetched on the exact same connection the
    /// real query then runs on -- see `run_tracked`), plus a token unique
    /// to that call so a slower call's cleanup can never clobber a newer
    /// call's entry after it's already started (checked, not just
    /// overwritten, when clearing). `None` whenever nothing is running.
    current: std::sync::Mutex<Option<(u64, i32)>>,
    /// Source of the tokens `current` uses -- just needs to be unique per
    /// call, never reset, ordering between calls doesn't matter.
    next_token: AtomicU64,
}

impl PostgresDriver {
    fn new(connection_string: &str) -> Self {
        Self {
            connection_string: connection_string.to_string(),
            pool: None,
            transaction: Mutex::new(None),
            in_transaction: AtomicBool::new(false),
            current: std::sync::Mutex::new(None),
            next_token: AtomicU64::new(0),
        }
    }

    /// Runs `query` against `conn`, with `self.current` tracking its own
    /// backend pid for the duration -- see `current`'s own doc comment for
    /// why this has to be the exact connection the query itself runs on,
    /// not a separate one from the pool. The `SELECT pg_backend_pid()`
    /// lookup failing (never observed, but it's one more fallible round
    /// trip) just means this one call can't be real-cancelled; it still
    /// runs normally.
    async fn run_tracked(
        &self,
        conn: &mut PgConnection,
        query: &str,
    ) -> anyhow::Result<QueryResult> {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        if let Ok(pid) = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut *conn)
            .await
        {
            *self.current.lock().unwrap() = Some((token, pid));
        }
        let mut result = run(&mut *conn, query).await;
        // A `SELECT` that matched nothing streams no rows, and the column
        // names only ever arrive with a row -- ask Postgres to describe the
        // statement (prepare, don't run) so the grid still gets its header.
        // Best effort: a failure (e.g. several statements) leaves it as is.
        if let Ok(QueryResult::Table { columns, rows, .. }) = &mut result
            && columns.is_empty()
            && rows.is_empty()
            && let Ok(described) = (&mut *conn).describe(query).await
        {
            *columns = described
                .columns()
                .iter()
                .map(|c| c.name().to_string())
                .collect();
        }
        let mut current = self.current.lock().unwrap();
        if matches!(*current, Some((t, _)) if t == token) {
            *current = None;
        }
        drop(current);
        result
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
async fn run<'e, E>(executor: E, query: &'e str) -> anyhow::Result<QueryResult>
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
    //
    // `raw_sql`, not `query`: it uses Postgres's simple query protocol, so
    // every value arrives as the server's own text -- see `stringify_column`.
    let mut stream = sqlx::raw_sql(query).fetch(executor);
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    let mut budget = query_driver::RowBudget::default();
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format_pg_error(e, query))?
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
        && let Some(marker) = query_driver::line_and_caret(query, position)
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

mod cheatsheet;

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

    fn cheatsheet(&self) -> &'static [tradar_query::CheatEntry] {
        crate::cheatsheet::ENTRIES
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

    fn table_ddl(&self, op: &query_driver::TableDesignerOp) -> Option<String> {
        Some(query_driver::build_table_ddl(op))
    }

    fn supports_migrations(&self) -> bool {
        true
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
        op: tradar_query::CrudOp,
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
        // Tables/views and their columns in one round trip, ordered so the
        // grouping below can just walk the rows: information_schema joins
        // are cheaper than a query per table. Grouped by (schema, name),
        // not just name -- two different schemas can each have their own
        // "users", and rows for both would otherwise sort next to each
        // other (same `ORDER BY ... table_name`) and get merged into one
        // entry with columns from both.
        // The primary-key columns ride along in the same round trip: the
        // results grid needs them to address a row it wants to change, and
        // asking for them separately would mean a second query per table.
        let rows: Vec<(String, String, String, String, String, bool)> = sqlx::query_as(
            "SELECT t.table_schema, t.table_name, t.table_type, c.column_name, c.data_type, \
                    (k.column_name IS NOT NULL) \
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
             WHERE t.table_schema NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY t.table_schema, t.table_name, c.ordinal_position",
        )
        .fetch_all(pool)
        .await?;

        // Foreign keys, one more round trip in the same information_schema
        // join style as the primary-key subquery above: `key_column_usage`
        // for the referencing (schema, table, column) side of the
        // constraint, joined to `constraint_column_usage` for the
        // referenced side. Keyed by bare table name, not schema-qualified
        // -- `qualify_colliding_names` below only prefixes a `SchemaInfo`
        // when its name collides across schemas, and that's rare enough
        // this stays a known miss rather than resolving it here too.
        let fk_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT kcu.table_schema, kcu.table_name, kcu.column_name, \
                    ccu.table_name, ccu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON kcu.constraint_name = tc.constraint_name \
              AND kcu.table_schema = tc.table_schema \
             JOIN information_schema.constraint_column_usage ccu \
               ON ccu.constraint_name = tc.constraint_name \
              AND ccu.table_schema = tc.table_schema \
             WHERE tc.constraint_type = 'FOREIGN KEY'",
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
        for (table_schema, table, table_type, column, type_name, primary_key) in rows {
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
                primary_key,
                foreign_key,
                indexed: false,
            });
        }

        // Functions/procedures have no columns of their own -- a second,
        // unrelated round trip rather than trying to force them into the
        // tables/columns join above.
        let routines: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT routine_schema, routine_name, routine_type \
             FROM information_schema.routines \
             WHERE routine_schema NOT IN ('pg_catalog', 'information_schema') \
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
            return self.run_tracked(tx, query).await;
        }
        drop(guard);
        let pool = self.pool.as_ref().expect("connect() must be called first");
        let mut conn = pool.acquire().await?;
        self.run_tracked(&mut conn, query).await
    }

    fn in_transaction(&self) -> bool {
        self.in_transaction.load(Ordering::Relaxed)
    }

    async fn cancel_query(&self) -> anyhow::Result<()> {
        let Some((_, pid)) = *self.current.lock().unwrap() else {
            return Ok(());
        };
        let pool = self.pool.as_ref().expect("connect() must be called first");
        sqlx::query("SELECT pg_cancel_backend($1)")
            .bind(pid)
            .execute(pool)
            .await?;
        Ok(())
    }
}

/// Renders one cell as text for the results grid.
///
/// Rows come from `sqlx::raw_sql`, i.e. Postgres's *simple* query protocol,
/// where the server sends every value already formatted as text -- exactly
/// what `psql` prints. That is the only way to show *every* type: the
/// extended protocol `sqlx::query()` uses is binary, where each type needs
/// its own typed decoder (there is no generic one), and an earlier version
/// of this function listed the common ones and showed `NULL` for the rest
/// -- indistinguishable from a real null, and wrong for `numeric` (money!),
/// arrays, enums, `inet`, `interval`, `bytea`, ranges, PostGIS and so on.
///
/// Two small presentation choices over the raw text: a real SQL null is
/// `NULL`, and booleans read `true`/`false` rather than `psql`'s `t`/`f`.
fn stringify_column(row: &PgRow, index: usize) -> String {
    let raw = row.try_get_raw(index).expect("valid column index");
    if raw.is_null() {
        return "NULL".to_string();
    }
    let text = match raw.as_bytes() {
        Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Err(_) => return "NULL".to_string(),
    };
    if raw.type_info().name() == "BOOL" {
        return match text.as_str() {
            "t" => "true".to_string(),
            "f" => "false".to_string(),
            _ => text,
        };
    }
    text
}

#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
pub use tui::connector;
// The crate's tests drive the TUI path too (`QueryEngine`, `SavedConnection`).
#[cfg(all(test, feature = "tui"))]
#[allow(unused_imports)]
use tui::*;

/// Opens a connected driver for `tradar-server` -- no TUI types involved.
pub fn opener() -> Box<dyn tradar_query::DriverOpener> {
    Box::new(Opener)
}

struct Opener;

#[async_trait]
impl tradar_query::DriverOpener for Opener {
    fn id(&self) -> &'static str {
        "postgres"
    }

    async fn open(&self, target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
        let mut driver = PostgresDriver::new(target);
        tradar_query::with_connect_timeout(target, driver.connect()).await?;
        Ok(Arc::new(driver))
    }
}

#[cfg(test)]
mod tests;
