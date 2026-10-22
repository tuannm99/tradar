//! Cassandra connector: implements `QueryDriver` directly against the
//! `scylla` crate (a pure-Rust CQL binary-protocol client that also speaks
//! to real Apache Cassandra, not just ScyllaDB), and exposes it to
//! `tradar-app` only through `connector()` -- nothing else in this crate is
//! `pub`, so the driver's internals stay this crate's own business, not
//! something the rest of the app can reach into.

use async_trait::async_trait;
use futures_util::StreamExt;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use scylla::value::Row;

use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};

mod edit;
mod error;
mod values;

use edit::*;
use error::*;
use values::*;

struct CassandraDriver {
    /// A bare `host:port` contact point (e.g. `127.0.0.1:9042`) -- not a
    /// URI, since `SessionBuilder::known_node` wants exactly that and
    /// Cassandra has no single default database to fold into the target
    /// the way Postgres/Mongo's connection strings do.
    contact_point: String,
    session: Option<Session>,
}

impl CassandraDriver {
    fn new(contact_point: &str) -> Self {
        Self {
            contact_point: contact_point.to_string(),
            session: None,
        }
    }

    fn session(&self) -> &Session {
        self.session
            .as_ref()
            .expect("connect() must be called first")
    }
}

/// Sort key for `system_schema.columns.kind` -- partition key first, then
/// clustering, then everything else (regular/static columns, where
/// `position` is meaningless anyway).
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "partition_key" => 0,
        "clustering" => 1,
        _ => 2,
    }
}

#[async_trait]
impl QueryDriver for CassandraDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let session = SessionBuilder::new()
            .known_node(&self.contact_point)
            .build()
            .await?;
        self.session = Some(session);
        Ok(())
    }

    fn keywords(&self) -> &'static [&'static str] {
        // CQL syntax is close enough to SQL that the shared vocabulary
        // (SELECT/INSERT/WHERE/...) is still the right suggestion set --
        // same reasoning that already lets Postgres and SQLite share it.
        query_driver::SQL_KEYWORDS
    }

    fn split_statements(&self, text: &str) -> Vec<query_driver::Statement> {
        // CQL also separates statements with `;` and has no dollar-quote
        // syntax of its own, but that branch in the shared splitter only
        // ever triggers on a literal `$`, so it's inert here rather than
        // wrong.
        query_driver::split_sql_statements(text)
    }

    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_query::CrudOp,
        columns: &[String],
    ) -> Option<String> {
        Some(query_driver::build_crud_snippet(entry, op, columns))
    }

    fn edit_source(&self, query: &str) -> Option<String> {
        query_driver::single_table_source(query)
    }

    fn edit_sql(&self, edit: &query_driver::RowEdit) -> Option<String> {
        Some(build_cql_edit(edit))
    }

    async fn ping(&self) -> anyhow::Result<()> {
        // Cassandra has no equivalent of a guaranteed-empty table to
        // `SELECT 1` against; `system.local` always exists and always has
        // exactly one row, so it's the standard health-check query.
        self.session()
            .query_unpaged("SELECT key FROM system.local", &[])
            .await?;
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let session = self.session();
        let tables_result = session
            .query_unpaged(
                "SELECT keyspace_name, table_name FROM system_schema.tables",
                &[],
            )
            .await?
            .into_rows_result()?;
        let tables: Vec<(String, String)> = tables_result
            .rows::<(String, String)>()?
            .collect::<Result<_, _>>()?;

        let mut schema = Vec::new();
        for (keyspace, table) in tables {
            // Cassandra's own system keyspaces aren't user data -- same
            // reason Postgres schema browsing skips `pg_catalog`.
            if keyspace.starts_with("system") {
                continue;
            }

            let columns_result = session
                .query_unpaged(
                    "SELECT column_name, type, kind, position FROM system_schema.columns \
                     WHERE keyspace_name = ? AND table_name = ?",
                    (&keyspace, &table),
                )
                .await?
                .into_rows_result()?;
            let mut columns: Vec<(String, String, String, i32)> = columns_result
                .rows::<(String, String, String, i32)>()?
                .collect::<Result<_, _>>()?;
            // A plain SELECT makes no row-order guarantee, so a composite
            // key can otherwise come back in any order -- `kind` sorts
            // partition-key columns before clustering columns (before
            // everything else), and `position` (0-based *within* each
            // kind, -1 for non-key columns) recovers the declared order
            // inside each: `PRIMARY KEY (user_id, group_id)` must report
            // exactly that order, not "whatever this SELECT happened to
            // return".
            columns.sort_by_key(|(_, _, kind, position)| (kind_rank(kind), *position));

            schema.push(SchemaInfo {
                // Keyspace-qualified, same as Postgres's `public.users` --
                // `build_crud_snippet`'s `quote_identifier` already splits
                // on `.` to quote each part separately.
                name: format!("{keyspace}.{table}"),
                columns: columns
                    .into_iter()
                    .map(|(name, type_name, kind, _)| ColumnInfo {
                        name,
                        type_name,
                        // A row's identity is its partition key plus (if
                        // any) clustering columns together -- there's no
                        // single-column "the primary key" flag the way SQL
                        // has one, but "part of what a WHERE needs to
                        // address one row" is the same concept
                        // `ColumnInfo::primary_key` already means here.
                        primary_key: kind == "partition_key" || kind == "clustering",
                        // CQL has no referential-integrity concept -- no FK
                        // to report, ever, for this driver.
                        foreign_key: None,
                        indexed: false,
                    })
                    .collect(),
                kind: None,
                ttl: None,
                // A keyspace-per-connection level in the navigator, same
                // as Postgres's schema level -- Cassandra has no other
                // object kind besides tables to further group by (no
                // views/functions/procedures the way Postgres does), so
                // `object_kind` stays `None`: `flatten_outline` skips
                // straight from the keyspace folder to the table, no
                // redundant single-kind "Tables" folder in between.
                schema: Some(keyspace),
                object_kind: None,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        // Non-SELECT statements (INSERT/UPDATE/DELETE/DDL) have no row
        // shape to stream -- same `returns_rows` heuristic Postgres/MySQL/
        // SQLite already use to pick between a streamed, capped read and a
        // plain affected-rows execute, rather than this driver's own
        // after-the-fact `IntoRowsResultError::ResultNotRows` check it used
        // to make on an already-fetched response (see below for why that
        // response can no longer be fetched unpaged in the first place).
        if !query_driver::returns_rows(query) {
            self.session()
                .query_unpaged(query, &[])
                .await
                .map_err(|e| format_cassandra_error(e, query))?;
            // CQL's wire protocol has no equivalent of SQL's affected-row
            // count for these (a `Void` result carries nothing else), so
            // this is always 0 -- not a shortcut, an actual protocol
            // limitation.
            return Ok(QueryResult::Affected { rows: 0 });
        }

        // Streamed and capped via `query_iter` rather than `query_unpaged`,
        // which fetches the *entire* result from the cluster in one
        // unpaged response before any truncation could happen -- the same
        // "pulls an unbounded result set into memory" failure mode the
        // `fetch`-based SQL drivers already avoid, just reachable here by
        // an ordinary forgotten-`LIMIT` `SELECT` rather than connect-time
        // schema browsing. Column specs have to be read before
        // `rows_stream` consumes the pager by value.
        let pager = self
            .session()
            .query_iter(query, &[])
            .await
            .map_err(|e| format_cassandra_error(e, query))?;
        let columns: Vec<String> = pager
            .column_specs()
            .as_slice()
            .iter()
            .map(|spec| spec.name().to_string())
            .collect();
        let mut rows_stream = pager
            .rows_stream::<Row>()
            .map_err(|e| format_cassandra_error(e, query))?;
        let mut out_rows = Vec::new();
        let mut truncated = false;
        let mut budget = query_driver::RowBudget::default();
        while let Some(row) = rows_stream.next().await {
            if budget.is_full() {
                truncated = true;
                break;
            }
            let cells = stringify_row(&row.map_err(|e| format_cassandra_error(e, query))?);
            budget.keep(&cells);
            out_rows.push(cells);
        }
        Ok(QueryResult::Table {
            columns,
            rows: out_rows,
            truncated,
        })
    }
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
        "cassandra"
    }

    async fn open(&self, target: &str) -> anyhow::Result<std::sync::Arc<dyn QueryDriver>> {
        let mut driver = CassandraDriver::new(target);
        tradar_query::with_connect_timeout(target, driver.connect()).await?;
        Ok(std::sync::Arc::new(driver))
    }
}

#[cfg(test)]
mod tests;
