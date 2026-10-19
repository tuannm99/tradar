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
use scylla::value::{CqlValue, Row};

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session as ConnectorSession};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

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

/// Formats a CQL error the same `LINE N: ... ^` shape
/// `tradar-connector-postgres`'s `format_pg_error` produces, when there's a
/// token in the message to point at. CQL has no equivalent of Postgres's
/// wire-protocol position, but Cassandra's own parser (ANTLR-based)
/// quotes the offending token in its message the same way MySQL/SQLite's
/// own messages do -- `"... mismatched input 'FRO' expecting K_FROM"` or
/// `"... no viable alternative at input 'FRO'"` -- recovered here via
/// `cassandra_error_marker`, the same trick
/// `tradar-connector-sqlite`/`tradar-connector-mysql` already play for
/// exactly this "no native position" situation. Anything that doesn't
/// match that shape (a missing keyspace/table, a timeout, a dropped
/// connection) falls straight through to `ExecutionError`'s own `Display`
/// unchanged -- there's no token to search for.
fn format_cassandra_error<E>(error: E, query: &str) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    let message = error.to_string();
    let Some(marker) = cassandra_error_marker(&message, query) else {
        return error.into();
    };
    anyhow::anyhow!("{message}\n{marker}")
}

/// The `LINE N: ... ^` marker for the first single-quoted token in
/// `message`, found by that token's first occurrence in `query` --
/// approximate (the real mistake could in principle be a later occurrence
/// of the same token), but still the closest thing to a position
/// Cassandra's own message offers. The explicit `line N:col` its ANTLR
/// parser also reports is deliberately ignored in favor of recomputing a
/// position from the token itself, to stay consistent with every other
/// caller of `line_and_caret`. `None` when the message has no quoted
/// token, or the token it names doesn't actually occur in `query`
/// (nothing to point at either way).
fn cassandra_error_marker(message: &str, query: &str) -> Option<String> {
    let start = message.find('\'')? + 1;
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

/// Turns one cell into display text, the way `cqlsh` prints it: ISO
/// timestamps, plain decimals, `{a, b}` for a set, `[a, b]` for a list,
/// `{k: v}` for a map. An earlier version fell back to Rust's `Debug` for
/// anything composite or exotic, which put `Timestamp(CqlTimestamp(1785804417000))`
/// and `Decimal(CqlDecimal { int_val: CqlVarint([4, 226]), scale: 2 })` in
/// the grid -- found by running against a real Cassandra.
fn stringify_cql_value(value: &CqlValue) -> String {
    match value {
        CqlValue::Ascii(s) | CqlValue::Text(s) => s.clone(),
        CqlValue::Boolean(b) => b.to_string(),
        CqlValue::TinyInt(n) => n.to_string(),
        CqlValue::SmallInt(n) => n.to_string(),
        CqlValue::Int(n) => n.to_string(),
        CqlValue::BigInt(n) => n.to_string(),
        CqlValue::Counter(c) => c.0.to_string(),
        CqlValue::Float(n) => n.to_string(),
        CqlValue::Double(n) => n.to_string(),
        CqlValue::Uuid(u) => u.to_string(),
        CqlValue::Timeuuid(u) => u.to_string(),
        CqlValue::Inet(ip) => ip.to_string(),
        CqlValue::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
        CqlValue::Varint(v) => signed_be_to_decimal(v.as_signed_bytes_be_slice()),
        CqlValue::Decimal(d) => {
            let (bytes, scale) = d.as_signed_be_bytes_slice_and_exponent();
            format_decimal(&signed_be_to_decimal(bytes), scale)
        }
        CqlValue::Timestamp(t) => format_timestamp_ms(t.0),
        CqlValue::Date(d) => {
            let (y, m, day) = civil_from_days(i64::from(d.0) - (1i64 << 31));
            format!("{y:04}-{m:02}-{day:02}")
        }
        CqlValue::Time(t) => {
            let ns = t.0.max(0);
            let secs = ns / 1_000_000_000;
            format!(
                "{:02}:{:02}:{:02}.{:09}",
                secs / 3600,
                secs / 60 % 60,
                secs % 60,
                ns % 1_000_000_000
            )
        }
        CqlValue::Duration(d) => format!("{}mo{}d{}ns", d.months, d.days, d.nanoseconds),
        CqlValue::List(items) | CqlValue::Vector(items) => {
            format!("[{}]", join_nested(items.iter()))
        }
        CqlValue::Set(items) => format!("{{{}}}", join_nested(items.iter())),
        CqlValue::Map(pairs) => {
            let body: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", nested(k), nested(v)))
                .collect();
            format!("{{{}}}", body.join(", "))
        }
        CqlValue::Tuple(items) => {
            let body: Vec<String> = items
                .iter()
                .map(|v| v.as_ref().map_or("NULL".to_string(), nested))
                .collect();
            format!("({})", body.join(", "))
        }
        CqlValue::UserDefinedType { fields, .. } => {
            let body: Vec<String> = fields
                .iter()
                .map(|(name, v)| {
                    format!("{name}: {}", v.as_ref().map_or("NULL".to_string(), nested))
                })
                .collect();
            format!("{{{}}}", body.join(", "))
        }
        CqlValue::Empty => String::new(),
        // `CqlValue` is non-exhaustive across driver releases; a variant
        // added later still shows something rather than failing to build.
        #[allow(unreachable_patterns)]
        other => format!("{other:?}"),
    }
}

/// A value inside a collection: text is single-quoted (`'a'`), as `cqlsh`
/// does, so `{'a, b'}` and `{'a', 'b'}` stay distinguishable.
fn nested(value: &CqlValue) -> String {
    match value {
        CqlValue::Ascii(s) | CqlValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
        other => stringify_cql_value(other),
    }
}

fn join_nested<'a>(items: impl Iterator<Item = &'a CqlValue>) -> String {
    items.map(nested).collect::<Vec<_>>().join(", ")
}

/// A two's-complement big-endian integer of any width as decimal text --
/// what a CQL `varint`, and the unscaled part of a `decimal`, are on the
/// wire. Repeated division by 10^9 over base-2^32 limbs; no bignum crate.
fn signed_be_to_decimal(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".to_string();
    }
    let negative = bytes[0] & 0x80 != 0;
    let mut magnitude: Vec<u8> = bytes.to_vec();
    if negative {
        // Two's complement -> magnitude: invert, then add one.
        for b in magnitude.iter_mut() {
            *b = !*b;
        }
        for b in magnitude.iter_mut().rev() {
            let (sum, carry) = b.overflowing_add(1);
            *b = sum;
            if !carry {
                break;
            }
        }
    }
    // Limbs, most significant first.
    let mut limbs: Vec<u32> = Vec::new();
    for chunk in magnitude.rchunks(4).collect::<Vec<_>>().into_iter().rev() {
        let mut limb = 0u32;
        for &b in chunk {
            limb = (limb << 8) | u32::from(b);
        }
        limbs.push(limb);
    }
    let mut groups: Vec<u32> = Vec::new();
    while limbs.iter().any(|&l| l != 0) {
        let mut remainder = 0u64;
        for limb in limbs.iter_mut() {
            let cur = (remainder << 32) | u64::from(*limb);
            *limb = (cur / 1_000_000_000) as u32;
            remainder = cur % 1_000_000_000;
        }
        groups.push(remainder as u32);
    }
    let mut out = String::new();
    if negative && !groups.is_empty() {
        out.push('-');
    }
    match groups.split_last() {
        None => return "0".to_string(),
        Some((top, rest)) => {
            out.push_str(&top.to_string());
            for g in rest.iter().rev() {
                out.push_str(&format!("{g:09}"));
            }
        }
    }
    out
}

/// `unscaled` (decimal text, maybe negative) with `scale` digits after the
/// point; a negative scale means trailing zeros.
fn format_decimal(unscaled: &str, scale: i32) -> String {
    let (sign, digits) = match unscaled.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", unscaled),
    };
    if scale <= 0 {
        let zeros = "0".repeat(scale.unsigned_abs() as usize);
        return if digits == "0" {
            "0".to_string()
        } else {
            format!("{sign}{digits}{zeros}")
        };
    }
    let scale = scale as usize;
    let padded = if digits.len() <= scale {
        format!("{}{digits}", "0".repeat(scale - digits.len() + 1))
    } else {
        digits.to_string()
    };
    let (int_part, frac_part) = padded.split_at(padded.len() - scale);
    format!("{sign}{int_part}.{frac_part}")
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// A CQL `timestamp` (ms since the epoch) as `cqlsh` prints it, in UTC.
fn format_timestamp_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03}+0000",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1_000 % 60,
        in_day % 1_000
    )
}

fn stringify_row(row: &Row) -> Vec<String> {
    row.columns
        .iter()
        .map(|cell| match cell {
            Some(value) => stringify_cql_value(value),
            None => "NULL".to_string(),
        })
        .collect()
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

/// Whether `value`'s own text reads as a CQL identifier/UUID literal --
/// the hyphenated hex shape (`8-4-4-4-12`) CQL accepts unquoted for both
/// `uuid` and `timeuuid` columns alike (textually identical; which of the
/// two doesn't change how it's written).
fn looks_like_cql_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    [8, 4, 4, 4, 12]
        == groups
            .iter()
            .map(|g| g.len())
            .collect::<Vec<_>>()
            .as_slice()
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A `RowEdit` value, written the way CQL's own literal grammar needs --
/// unlike SQL, CQL does **not** coerce a quoted string to a number/uuid/
/// boolean for comparison (`query_driver::build_sql_edit`'s own doc
/// comment explains why SQL gets away with always quoting); CQL's parser
/// instead rejects the statement outright (`Invalid STRING constant ...
/// for int`). There is no schema to consult here either -- a column's
/// declared CQL type isn't available to this sync method without a round
/// trip -- so, same reason `tradar-connector-mongo`'s `mongo_value_literal`
/// and Elasticsearch's own `edit_sql` already infer from the value's own
/// text instead of a schema lookup, this does too: a bare `true`/`false`,
/// something that parses as a number, or a UUID-shaped string are written
/// unquoted; everything else -- including a collection literal
/// (`list<...>`/`set<...>`/`map<...>`), which CQL forbids inside a
/// `PRIMARY KEY` anyway, so a key column is never one -- is quoted as a
/// plain CQL string. Narrow, known miss: literally typing the text `NaN`/
/// `inf`/`infinity` into a text column's edit prompt is written unquoted
/// (`str::parse::<f64>` itself accepts those spellings) -- valid CQL for
/// `float`/`double`, a type error for `text`; same class of edge case
/// `mongo_value_literal` already has for the literal text `null`.
fn cql_value_literal(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
        return trimmed.to_ascii_lowercase();
    }
    if trimmed.parse::<f64>().is_ok() || looks_like_cql_uuid(trimmed) {
        return trimmed.to_string();
    }
    format!("'{}'", trimmed.replace('\'', "''"))
}

/// `query_driver::build_sql_edit`, with CQL's own literal rules
/// (`cql_value_literal`) in place of SQL's always-quote-it shortcut --
/// otherwise the identical `UPDATE ... SET ... WHERE ...`/`DELETE FROM
/// ... WHERE ...` shape, since CQL's DML syntax is close enough to SQL to
/// share it (same reason this driver already reuses
/// `SQL_KEYWORDS`/`split_sql_statements`). A `WHERE` built from equality
/// on every key column (`edit.key`, which is every `partition_key`/
/// `clustering` column together -- see `list_schema`'s own comment on
/// `ColumnInfo::primary_key`) is valid CQL for addressing exactly one
/// row; the composite-key tuple syntax (`(a, b) = (...)`) is only needed
/// for `IN`/range queries, not plain equality.
fn build_cql_edit(edit: &query_driver::RowEdit) -> String {
    let table = query_driver::quote_identifier(&edit.table);
    let where_clause = edit
        .key
        .iter()
        .map(|(column, value)| {
            if value == "NULL" {
                format!("{} IS NULL", query_driver::quote_identifier(column))
            } else {
                format!(
                    "{} = {}",
                    query_driver::quote_identifier(column),
                    cql_value_literal(value)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    match &edit.change {
        query_driver::RowChange::SetValue { column, value } => format!(
            "UPDATE {table} SET {} = {} WHERE {where_clause}",
            query_driver::quote_identifier(column),
            cql_value_literal(value)
        ),
        query_driver::RowChange::SetValues(pairs) => format!(
            "UPDATE {table} SET {} WHERE {where_clause}",
            pairs
                .iter()
                .map(|(column, value)| format!(
                    "{} = {}",
                    query_driver::quote_identifier(column),
                    cql_value_literal(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        query_driver::RowChange::DeleteRow => format!("DELETE FROM {table} WHERE {where_clause}"),
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
        op: tradar_core::action::CrudOp,
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

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "cassandra",
    display_name: "Cassandra",
    icon: "🌀",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct CassandraConnector;

#[async_trait]
impl Connector for CassandraConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(
        &self,
        connection: SavedConnection,
    ) -> anyhow::Result<Box<dyn ConnectorSession>> {
        let mut driver = CassandraDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(CassandraConnector)
}

#[cfg(test)]
mod tests;
