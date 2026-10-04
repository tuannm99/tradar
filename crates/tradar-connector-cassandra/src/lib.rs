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
        while let Some(row) = rows_stream.next().await {
            if out_rows.len() == query_driver::MAX_ROWS {
                truncated = true;
                break;
            }
            out_rows.push(stringify_row(
                &row.map_err(|e| format_cassandra_error(e, query))?,
            ));
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
mod tests {
    #[test]
    fn varint_and_decimal_read_as_the_number_they_hold() {
        // 1250 = 0x04E2, scale 2 -> 12.50 (the bytes a real Cassandra sent).
        assert_eq!(signed_be_to_decimal(&[4, 226]), "1250");
        assert_eq!(format_decimal("1250", 2), "12.50");
        assert_eq!(signed_be_to_decimal(&[0]), "0");
        assert_eq!(signed_be_to_decimal(&[127]), "127");
        assert_eq!(signed_be_to_decimal(&[0, 128]), "128");
        assert_eq!(signed_be_to_decimal(&[255]), "-1");
        assert_eq!(signed_be_to_decimal(&[255, 127]), "-129");
        assert_eq!(signed_be_to_decimal(&[128]), "-128");
        // 2^70 needs more than a machine word.
        assert_eq!(
            signed_be_to_decimal(&[0x40, 0, 0, 0, 0, 0, 0, 0, 0]),
            "1180591620717411303424"
        );
        assert_eq!(format_decimal("5", 3), "0.005");
        assert_eq!(format_decimal("-5", 3), "-0.005");
        assert_eq!(format_decimal("123", 0), "123");
        assert_eq!(format_decimal("12", -3), "12000");
        assert_eq!(format_decimal("0", -2), "0");
    }

    #[test]
    fn timestamps_dates_and_times_read_as_calendar_text() {
        use scylla::value::{CqlDate, CqlTime, CqlTimestamp};
        // The value a real Cassandra returned for '2026-08-04 00:46:57+0000'.
        assert_eq!(
            stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(1_785_804_417_000))),
            "2026-08-04 00:46:57.000+0000"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(0))),
            "1970-01-01 00:00:00.000+0000"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(-1))),
            "1969-12-31 23:59:59.999+0000"
        );
        // 2^31 days is the epoch in CQL's `date` encoding.
        assert_eq!(
            stringify_cql_value(&CqlValue::Date(CqlDate((1u32 << 31) + 20_669))),
            "2026-08-04"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Time(CqlTime(3_723_000_000_456))),
            "01:02:03.000000456"
        );
    }

    #[test]
    fn collections_read_the_way_cqlsh_prints_them() {
        let text = |s: &str| CqlValue::Text(s.to_string());
        assert_eq!(
            stringify_cql_value(&CqlValue::Set(vec![text("a"), text("b")])),
            "{'a', 'b'}"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::List(vec![CqlValue::Int(1), CqlValue::Int(2)])),
            "[1, 2]"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Map(vec![(text("k"), CqlValue::Int(1))])),
            "{'k': 1}"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Tuple(vec![Some(CqlValue::Int(1)), None])),
            "(1, NULL)"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Set(vec![text("it's")])),
            "{'it''s'}"
        );
    }

    use super::*;

    #[test]
    fn cassandra_error_marker_points_at_the_quoted_token_s_position() {
        let message = "line 1:14 mismatched input 'FRO' expecting K_FROM";
        let query = "SELECT * FRO users";

        let marker = cassandra_error_marker(message, query).unwrap();

        assert!(marker.contains("LINE 1:"), "marker was: {marker}");
        assert!(marker.contains('^'), "marker was: {marker}");
    }

    #[test]
    fn cassandra_error_marker_is_none_for_a_message_with_no_quoted_token() {
        assert_eq!(
            cassandra_error_marker("unconfigured table users", "SELECT * FROM users"),
            None
        );
    }

    #[test]
    fn cassandra_error_marker_is_none_when_the_token_is_not_actually_in_the_query() {
        assert_eq!(
            cassandra_error_marker(
                "no viable alternative at input 'XYZ'",
                "SELECT * FROM users"
            ),
            None
        );
    }

    #[test]
    fn ascii_and_text_pass_through_verbatim() {
        assert_eq!(
            stringify_cql_value(&CqlValue::Text("hello".to_string())),
            "hello"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Ascii("hi".to_string())),
            "hi"
        );
    }

    #[test]
    fn numeric_and_boolean_variants_stringify_plainly() {
        assert_eq!(stringify_cql_value(&CqlValue::Int(42)), "42");
        assert_eq!(stringify_cql_value(&CqlValue::BigInt(-7)), "-7");
        assert_eq!(stringify_cql_value(&CqlValue::Boolean(true)), "true");
        assert_eq!(stringify_cql_value(&CqlValue::Double(1.5)), "1.5");
    }

    #[test]
    fn a_null_cell_is_reported_as_null_not_empty_string() {
        let row = Row {
            columns: vec![Some(CqlValue::Int(1)), None],
        };

        assert_eq!(
            stringify_row(&row),
            vec!["1".to_string(), "NULL".to_string()]
        );
    }

    #[test]
    fn crud_snippet_delegates_to_the_shared_sql_builder() {
        let driver = CassandraDriver::new("127.0.0.1:9042");
        let entry = SchemaInfo::new("demo.events");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
            Some("SELECT * FROM \"demo\".\"events\" LIMIT 100;".to_string())
        );
    }

    #[test]
    fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
        let driver = CassandraDriver::new("127.0.0.1:9042");
        let entry = SchemaInfo {
            name: "demo.events".to_string(),
            columns: vec![
                ColumnInfo::new("id", "uuid"),
                ColumnInfo::new("payload", "text"),
            ],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        };

        assert_eq!(
            driver.crud_snippet(
                &entry,
                tradar_core::action::CrudOp::Read,
                &["payload".to_string()]
            ),
            Some("SELECT \"payload\" FROM \"demo\".\"events\" LIMIT 100;".to_string())
        );
    }

    #[test]
    fn a_fresh_driver_is_never_in_a_transaction() {
        // No override -- Cassandra has no BEGIN/COMMIT/ROLLBACK, so this
        // must stay on QueryDriver's default rather than something this
        // crate has to maintain.
        let driver = CassandraDriver::new("127.0.0.1:9042");

        assert!(!driver.in_transaction());
    }

    mod docker {
        //! Integration tests against a real Cassandra, via `testcontainers`
        //! directly rather than `testcontainers-modules` -- that crate has
        //! no `cassandra` feature (confirmed via `cargo add --dry-run`
        //! before writing this). Unlike the other connectors' per-test
        //! containers (cheap and fast for Postgres/Redis/Mongo), Cassandra
        //! takes 30-60s just to start accepting CQL connections, so every
        //! assertion here shares a single container instead of spinning up
        //! one per test.

        use std::time::Duration;

        use testcontainers::core::{IntoContainerPort, WaitFor};
        use testcontainers::runners::AsyncRunner;
        use testcontainers::{ContainerAsync, GenericImage, ImageExt};

        use super::*;

        /// Returns the container alongside the driver -- the container
        /// must stay alive (and in scope) for as long as the driver is
        /// used, or it stops the moment this function returns.
        ///
        /// Needs host port 9042 free (not just *a* random one, see the
        /// `with_mapped_port`/`CASSANDRA_BROADCAST_RPC_ADDRESS` comment
        /// below) -- can't run alongside the long-lived dev instance from
        /// `docker compose up cassandra`.
        async fn connected_driver() -> (ContainerAsync<GenericImage>, CassandraDriver) {
            let container = GenericImage::new("cassandra", "5.0")
                .with_wait_for(WaitFor::message_on_stdout(
                    "Starting listening for CQL clients",
                ))
                // Cassandra always advertises ITS OWN port (9042) as part
                // of `broadcast_rpc_address` -- there's no setting for a
                // separate advertised port -- so the docker-assigned host
                // port has to be 9042 too, not whatever a random
                // `with_exposed_port` would pick; otherwise the control
                // connection (opened directly against `known_node`) works
                // but every real query, routed through a second pool
                // connection opened to the *advertised* address, times out.
                // The address side of that same problem is
                // `CASSANDRA_BROADCAST_RPC_ADDRESS` below -- without it
                // Cassandra advertises its Docker-internal bridge IP
                // instead of anything reachable from outside the
                // container. Root-caused by diffing a working `ping()`
                // against `system.local`'s `rpc_address` on a long-lived
                // container -- see the same fix in `docker-compose.yml`.
                .with_mapped_port(9042, 9042.tcp())
                .with_env_var("CASSANDRA_BROADCAST_RPC_ADDRESS", "127.0.0.1")
                // The JVM-based Cassandra image routinely needs 60-90s to
                // reach that log line -- past testcontainers' 60s default
                // startup timeout.
                .with_startup_timeout(Duration::from_secs(150))
                .start()
                .await
                .expect("cassandra container must start");
            let port = container
                .get_host_port_ipv4(9042)
                .await
                .expect("cassandra must expose 9042");

            let mut driver = CassandraDriver::new(&format!("127.0.0.1:{port}"));
            // The log line above means the native transport server has
            // started, not that the whole node (topology/gossip) is ready
            // to actually route a query yet -- `build()` can succeed
            // (control connection up) while the very next real query still
            // fails. A handful of retries covers that last stretch (this
            // is no longer papering over the address-mismatch bug -- see
            // the `with_mapped_port`/`CASSANDRA_BROADCAST_RPC_ADDRESS`
            // comment above -- just genuine "just started" variance).
            let mut last_error = None;
            'ready: for _ in 0..5 {
                match tokio::time::timeout(Duration::from_secs(10), driver.connect()).await {
                    Ok(Ok(())) => {
                        match tokio::time::timeout(Duration::from_secs(10), driver.ping()).await {
                            Ok(Ok(())) => {
                                last_error = None;
                                break 'ready;
                            }
                            Ok(Err(e)) => last_error = Some(e.to_string()),
                            Err(_) => last_error = Some("ping attempt timed out".to_string()),
                        }
                    }
                    Ok(Err(e)) => last_error = Some(e.to_string()),
                    Err(_) => last_error = Some("connect attempt timed out".to_string()),
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            if let Some(error) = last_error {
                panic!("cassandra never became ready to query: {error}");
            }
            (container, driver)
        }

        #[tokio::test]
        async fn ping_succeeds_against_a_running_cassandra() {
            let (_container, driver) =
                tokio::time::timeout(Duration::from_secs(350), connected_driver())
                    .await
                    .expect("container must become ready within 350s");

            let result = driver.ping().await;

            assert!(result.is_ok(), "ping failed: {:?}", result.err());
        }

        #[tokio::test]
        async fn schema_and_execute_round_trip_through_a_real_cluster() {
            let (_container, driver) =
                tokio::time::timeout(Duration::from_secs(350), connected_driver())
                    .await
                    .expect("container must become ready within 350s");

            driver
                .execute(
                    "CREATE KEYSPACE demo WITH replication = \
                     {'class': 'SimpleStrategy', 'replication_factor': 1}",
                )
                .await
                .unwrap();
            driver
                .execute(
                    "CREATE TABLE demo.memberships (\
                     user_id int, group_id int, role text, \
                     PRIMARY KEY (user_id, group_id))",
                )
                .await
                .unwrap();

            let schema = driver.list_schema().await.unwrap();
            let table = schema
                .iter()
                .find(|s| s.name == "demo.memberships")
                .expect("demo.memberships must be in the schema, system keyspaces filtered out");
            let key: Vec<&str> = table
                .columns
                .iter()
                .filter(|c| c.primary_key)
                .map(|c| c.name.as_str())
                .collect();
            assert_eq!(
                key,
                vec!["user_id", "group_id"],
                "partition key + clustering column together must be reported, in order"
            );
            let role = table
                .columns
                .iter()
                .find(|c| c.name == "role")
                .expect("role column must be present");
            assert!(!role.primary_key);
            assert_eq!(role.type_name, "text");

            let inserted = driver
                .execute(
                    "INSERT INTO demo.memberships (user_id, group_id, role) VALUES (1, 2, 'admin')",
                )
                .await
                .unwrap();
            assert_eq!(
                inserted,
                QueryResult::Affected { rows: 0 },
                "CQL never reports an affected-row count, even for a real insert"
            );

            let selected = driver
                .execute("SELECT user_id, group_id, role FROM demo.memberships WHERE user_id = 1")
                .await
                .unwrap();
            match selected {
                QueryResult::Table {
                    columns,
                    rows,
                    truncated,
                } => {
                    assert_eq!(columns, vec!["user_id", "group_id", "role"]);
                    assert_eq!(
                        rows,
                        vec![vec!["1".to_string(), "2".to_string(), "admin".to_string()]]
                    );
                    assert!(!truncated);
                }
                other => panic!("expected a table, got {other:?}"),
            }
        }
    }
}
