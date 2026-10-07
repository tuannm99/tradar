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
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|e| format_mysql_error(e, query))?
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
        run(pool, query).await
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
mod tests {
    use super::*;
    use testcontainers_modules::mysql::Mysql;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[test]
    fn near_token_marker_points_at_the_quoted_token_s_position() {
        let message = "You have an error in your SQL syntax; ... right syntax to use near 'FRO users' at line 1";
        let query = "SELECT * FRO users";

        let marker = near_token_marker(message, query).unwrap();

        assert!(marker.contains("LINE 1:"), "marker was: {marker}");
        assert!(marker.contains('^'), "marker was: {marker}");
    }

    #[test]
    fn near_token_marker_is_none_for_a_message_with_no_quoted_token() {
        assert_eq!(
            near_token_marker("Table 'mydb.users' doesn't exist", "SELECT * FROM users"),
            None
        );
    }

    #[test]
    fn near_token_marker_is_none_when_the_token_is_not_actually_in_the_query() {
        assert_eq!(
            near_token_marker("... near 'XYZ' at line 1", "SELECT * FROM users"),
            None
        );
    }

    #[test]
    fn crud_snippet_delegates_to_the_shared_sql_builder() {
        let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");
        let entry = SchemaInfo::new("users");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
            Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
        );
    }

    #[test]
    fn table_ddl_and_migrations_are_not_supported_at_v1() {
        // Unlike Postgres, this driver doesn't override `table_ddl` or
        // `supports_migrations` -- both features generate dialect-specific
        // DDL/tracking SQL that hasn't been built for MySQL/MariaDB yet,
        // so they fall back to the `QueryDriver` trait's own defaults.
        let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");

        assert_eq!(
            driver.table_ddl(&query_driver::TableDesignerOp::RenameTable {
                table: "users".to_string(),
                new_name: "accounts".to_string(),
            }),
            None
        );
        assert!(!driver.supports_migrations());
    }

    #[tokio::test]
    async fn connect_fails_quickly_against_an_unreachable_host() {
        // Port 1 is reserved and never has a MySQL server listening -- no
        // Docker/testcontainers needed, connection is refused immediately at
        // the OS level. This is a regression test for the pool's default
        // 30s acquire_timeout, which made a bad connection target look
        // identical to a hung UI.
        let mut driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), driver.connect())
            .await
            .expect("connect() should fail well within 10s, not hang");

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_succeeds_for_a_running_mysql() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");

        let mut driver = MySqlDriver::new(&conn_string);
        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn list_schema_returns_created_tables() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(driver.pool.as_ref().unwrap())
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "users");
        assert!(schema[0].columns[0].primary_key);
    }

    #[tokio::test]
    async fn list_schema_reports_a_foreign_key_reference() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();
        let pool = driver.pool.as_ref().unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, \
             FOREIGN KEY (user_id) REFERENCES users (id))",
        )
        .execute(pool)
        .await
        .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let orders = schema.iter().find(|s| s.name == "orders").unwrap();
        let user_id = orders.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = user_id
            .foreign_key
            .as_ref()
            .expect("user_id references users(id)");
        assert_eq!(fk.table, "users");
        assert_eq!(fk.column, "id");
    }

    #[tokio::test]
    async fn list_schema_reports_views_and_hides_system_schemas() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();
        let pool = driver.pool.as_ref().unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("CREATE VIEW user_ids AS SELECT id FROM users")
            .execute(pool)
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let table = schema.iter().find(|s| s.name == "users").unwrap();
        assert_eq!(table.object_kind.as_deref(), Some("table"));
        let view = schema.iter().find(|s| s.name == "user_ids").unwrap();
        assert_eq!(view.object_kind.as_deref(), Some("view"));
        assert!(
            schema.iter().all(|s| !matches!(
                s.schema.as_deref(),
                Some("information_schema" | "mysql" | "performance_schema" | "sys")
            )),
            "system schemas must not show up in the navigator: {:?}",
            schema.iter().map(|s| &s.schema).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn execute_returns_columns_and_rows_for_a_select() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
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

    /// Regression test: `stringify_column`'s old fallback (a plain `String`
    /// decode for anything it didn't special-case) fails outright for
    /// these types under sqlx's binary wire protocol -- and would otherwise
    /// get swallowed into a plain "NULL", indistinguishable from an actual
    /// null and wrong for every row that had one.
    #[tokio::test]
    async fn json_and_datetime_columns_decode_to_real_text_not_null() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query(
            "CREATE TABLE events (
                data JSON NOT NULL,
                at DATETIME NOT NULL,
                on_day DATE NOT NULL,
                nothing JSON
            )",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events (data, at, on_day, nothing) VALUES
             ('{\"code\": \"ok\"}', '2026-08-04 00:46:57', '2026-08-04', NULL)",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

        let result = driver
            .execute("SELECT data, at, on_day, nothing FROM events")
            .await
            .unwrap();

        let QueryResult::Table { rows, .. } = result else {
            panic!("expected a Table result");
        };
        let row = &rows[0];
        assert_eq!(row[0], "{\"code\": \"ok\"}");
        assert_eq!(row[1], "2026-08-04 00:46:57");
        assert_eq!(row[2], "2026-08-04");
        assert_eq!(
            row[3], "NULL",
            "an actual null must still say NULL, distinct from a decode failure"
        );
    }

    /// Found by running against a real server: `DECIMAL` (the type money
    /// lives in), `ENUM`, `SET` and `BLOB` showed `NULL` for every row,
    /// because the binary-protocol decoder list never covered them.
    #[tokio::test]
    async fn decimal_enum_set_and_blob_columns_show_their_text_not_null() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();
        sqlx::query(
            "CREATE TABLE exotic (
                price DECIMAL(8,2) NOT NULL,
                mood ENUM('sad','ok','happy') NOT NULL,
                perms SET('r','w','x') NOT NULL,
                blob_col BLOB NOT NULL,
                text_blob BLOB NOT NULL,
                flag BOOLEAN NOT NULL,
                nothing DECIMAL(8,2)
            )",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO exotic VALUES
             (12.50, 'happy', 'r,w', X'DEADBEEF', 'plain text', TRUE, NULL)",
        )
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

        let result = driver
            .execute("SELECT price, mood, perms, blob_col, text_blob, flag, nothing FROM exotic")
            .await
            .unwrap();

        let QueryResult::Table { rows, .. } = result else {
            panic!("expected a Table result");
        };
        let row = &rows[0];
        assert_eq!(row[0], "12.50");
        assert_eq!(row[1], "happy");
        assert_eq!(row[2], "r,w");
        assert_eq!(row[3], "0xdeadbeef");
        assert_eq!(row[4], "plain text");
        assert_eq!(row[5], "true");
        assert_eq!(row[6], "NULL", "a real null still says NULL");
    }

    #[tokio::test]
    async fn a_fresh_driver_is_not_in_a_transaction() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();

        assert!(!driver.in_transaction());
    }

    #[tokio::test]
    async fn commit_closes_the_transaction_and_keeps_the_change() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
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
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut writer = MySqlDriver::new(&conn_string);
        writer.connect().await.unwrap();
        writer
            .execute("CREATE TABLE users (id INTEGER) ENGINE=InnoDB")
            .await
            .unwrap();
        let mut reader = MySqlDriver::new(&conn_string);
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

    #[tokio::test]
    async fn a_syntax_error_reports_the_offending_line_with_a_caret() {
        let container = Mysql::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(3306).await.unwrap();
        let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
        let mut driver = MySqlDriver::new(&conn_string);
        driver.connect().await.unwrap();

        let error = driver
            .execute("SELECT * FRO users")
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("LINE 1:"), "error was: {error}");
        assert!(error.contains('^'), "error was: {error}");
    }
}
