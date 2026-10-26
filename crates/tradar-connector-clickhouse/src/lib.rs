//! ClickHouse connector: real SQL (`SQL_KEYWORDS`/`split_sql_statements`,
//! same as Postgres/SQLite/Cassandra), but talked to over its HTTP
//! interface via `reqwest` rather than a native driver -- there is no
//! `sqlx` support for ClickHouse, and its HTTP interface already returns a
//! query's column names *and* types alongside the rows when asked for
//! `FORMAT JSON`, which is exactly the dynamic shape `QueryResult::Table`
//! needs for arbitrary user-typed SQL against an unknown schema. Exposes
//! only `connector()` -- everything else here is this crate's own business.
//!
//! Row-edit (`QueryDriver::edit_sql`/`edit_source`) is deliberately not
//! implemented: ClickHouse has no ANSI `UPDATE`/`DELETE`, only
//! `ALTER TABLE ... UPDATE/DELETE` mutations, which run asynchronously in
//! the background rather than taking effect immediately -- the results
//! grid's "y" then re-read-the-row flow would show stale data right after
//! confirming, which is a worse experience than not offering the edit at
//! all. See `docs/backlog/clickhouse-connector-2026-09-29.md`.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};

/// ClickHouse's own system databases -- never worth browsing, same idea as
/// Postgres excluding `pg_catalog`/`information_schema` from `list_schema`.
/// ClickHouse exposes both a real `system` database and an
/// `INFORMATION_SCHEMA`/`information_schema` compatibility view of it.
const SYSTEM_DATABASES: [&str; 3] = ["system", "information_schema", "INFORMATION_SCHEMA"];

struct ClickHouseDriver {
    /// The saved connection's target, as typed -- parsed lazily in
    /// `connect()` rather than here, same reason `PostgresDriver` fills its
    /// `pool` there instead of at construction: a bad target should fail as
    /// a connect error, not a panic from a constructor nothing can report
    /// through.
    target: String,
    base_url: String,
    /// `?database=` on every request, when the target named one -- `None`
    /// leaves ClickHouse to apply its own default (`"default"`) rather than
    /// this driver hardcoding that name.
    database: Option<String>,
    user: Option<String>,
    password: Option<String>,
    client: reqwest::Client,
}

/// The HTTP client, with a `User-Agent` of its own. `reqwest` sends none by
/// default, and a proxy or WAF in front of ClickHouse (nginx rules, Cloudflare,
/// an API gateway) commonly answers a request without one -- while `curl`, which
/// has one, gets through -- with a bare `403 Forbidden`.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("tradar/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl ClickHouseDriver {
    fn new(target: &str) -> Self {
        Self {
            target: target.to_string(),
            base_url: String::new(),
            database: None,
            user: None,
            password: None,
            client: http_client(),
        }
    }

    fn build_request(&self, sql: &str) -> reqwest::RequestBuilder {
        let mut request = self.client.post(&self.base_url).body(sql.to_string());
        if let Some(database) = &self.database {
            request = request.query(&[("database", database)]);
        }
        // HTTP Basic rather than `?user=&password=`: the password stays out
        // of URLs (server logs, proxies) and needs no query escaping.
        if let Some(user) = &self.user {
            request = request.basic_auth(user, self.password.as_deref());
        }
        request
    }

    /// Runs `sql` and returns the raw response body, or a clean error built
    /// from ClickHouse's own error text on a non-2xx status.
    async fn run_sql(&self, sql: &str) -> anyhow::Result<String> {
        let response = self.build_request(sql).send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("{}", format_clickhouse_error(status, &text, sql));
        }
        Ok(text)
    }
}

/// What `parse_target` pulls out of a connection target -- a plain struct
/// rather than a 4-tuple so each piece is named at both ends, not just
/// positional (and so clippy's `type_complexity` lint has nothing to flag).
struct ParsedTarget {
    base_url: String,
    database: Option<String>,
    user: Option<String>,
    password: Option<String>,
}

/// Parses a target of the form `http://[user[:password]@]host[:port][/database]`
/// (percent-encode `@ : / # ?` and spaces in the user/password: `p@ss` -> `p%40ss`)
/// into a clean base URL plus the pieces ClickHouse's HTTP interface takes
/// separately (`?database=`/`?user=`/`?password=`) -- into `reqwest::Url`
/// rather than adding a `url` dependency of this crate's own, since
/// `reqwest` already re-exports it.
fn parse_target(target: &str) -> anyhow::Result<ParsedTarget> {
    let url = reqwest::Url::parse(target)
        .map_err(|e| anyhow::anyhow!("invalid ClickHouse target '{target}': {e}"))?;
    // `Url::username()`/`password()` hand back the userinfo still
    // percent-encoded (`p%40ss` for `p@ss`); sending that as-is is a wrong
    // password, which ClickHouse answers with a 403. Decode it.
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    let user = (!url.username().is_empty()).then(|| decode(url.username()));
    let password = url.password().map(decode);
    let database = url.path().trim_matches('/');
    let database = (!database.is_empty()).then(|| database.to_string());

    let mut base = url;
    // Errors from these setters only fire for schemes that can't carry the
    // piece being cleared (e.g. no host at all) -- `http(s)` always can, and
    // this URL just parsed successfully as one, so there is nothing
    // meaningful to do with the `Err` here.
    let _ = base.set_username("");
    let _ = base.set_password(None);
    base.set_path("");
    base.set_query(None);
    let base_url = base.as_str().trim_end_matches('/').to_string();

    Ok(ParsedTarget {
        base_url,
        database,
        user,
        password,
    })
}

/// Formats a ClickHouse error, adding a `psql`-style `LINE N: ... ^` marker
/// (`query_driver::line_and_caret`, shared with Postgres/MySQL/SQLite) when
/// one can be recovered -- see `clickhouse_error_marker`'s own doc comment
/// for why that's a token search, not ClickHouse's own numeric position.
fn format_clickhouse_error(status: reqwest::StatusCode, body: &str, sql: &str) -> String {
    let first_line = body.lines().next().unwrap_or("").trim();
    let message = if first_line.is_empty() {
        let hint = if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::UNAUTHORIZED
        {
            " (no message from ClickHouse -- a proxy or firewall in front of it may be refusing the request)"
        } else {
            ""
        };
        format!("clickhouse returned {status}{hint}")
    } else {
        format!("clickhouse returned {status}: {first_line}")
    };
    match clickhouse_error_marker(body, sql) {
        Some(marker) => format!("{message}\n{marker}"),
        None => message,
    }
}

/// The `LINE N: ... ^` marker for a ClickHouse syntax error, when its
/// message is shaped like `Syntax error: failed at position N (<token>):
/// ...`. `N` is ClickHouse's own count of the request body it actually
/// received (`sql`, which is `query` plus a possibly-appended ` FORMAT
/// JSON` -- never an issue in practice, since a syntax error always lands
/// before that suffix) -- but this deliberately **doesn't** trust `N`
/// directly, since whether it's 0- or 1-based, byte- or char-counted isn't
/// confirmed against a live server anywhere in this codebase (no Docker in
/// the sandbox this was written in -- see
/// `docs/backlog/connector-ux-gaps-2026-10-02.md`). Instead it recovers the
/// token quoted in `(...)` right after the position and finds *that* by
/// substring search in `sql`, the same robust trick
/// `tradar-connector-sqlite`'s `near_token_marker` and
/// `tradar-connector-mysql`'s own `near_token_marker` already use for
/// exactly this "no confirmed native position" situation. `None` when the
/// message isn't shaped that way, or the token it names doesn't actually
/// occur in `sql`.
fn clickhouse_error_marker(body: &str, sql: &str) -> Option<String> {
    let prefix = "failed at position ";
    let after_prefix = &body[body.find(prefix)? + prefix.len()..];
    let open = after_prefix.find('(')?;
    let close = after_prefix[open..].find(')')?;
    let token = after_prefix[open + 1..open + close].trim();
    if token.is_empty() {
        return None;
    }
    let byte_index = sql.find(token)?;
    // `line_and_caret` wants a 1-based *character* index, matching
    // Postgres's own convention -- counting chars up to the byte offset
    // `find` returns, so multi-byte UTF-8 ahead of the token doesn't throw
    // the caret off.
    let char_index = sql[..byte_index].chars().count() + 1;
    query_driver::line_and_caret(sql, char_index)
}

/// Whether `sql` already names its own output format, so `execute` doesn't
/// double up (`... FORMAT JSON FORMAT JSON`) or override a format the user
/// explicitly asked for (`... FORMAT CSV`). A whole-word, case-insensitive
/// scan rather than a parser -- same heuristic style as `returns_rows`/
/// `transaction_control` in `query_driver`, and good enough since `FORMAT`
/// isn't a word ClickHouse SQL uses any other way.
fn has_format_clause(sql: &str) -> bool {
    sql.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word.eq_ignore_ascii_case("format"))
}

/// One result cell as display text -- `NULL` for JSON `null` (matching
/// every other driver's convention for a missing value), a string's own
/// text unquoted, and everything else (numbers, bools, and ClickHouse's
/// `Array`/`Tuple`/`Map`/nested types, which `FORMAT JSON` renders as JSON
/// arrays/objects) as compact JSON -- there's no single scalar type to
/// decode into the way `sqlx` gives Postgres, so this is the one rendering
/// that already covers every ClickHouse type without a type-by-type match.
fn clickhouse_cell_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Turns a ClickHouse `FORMAT JSON` response body (`{"meta": [...], "data":
/// [...], ...}`) into a `QueryResult::Table` -- column order comes from
/// `meta`, not from sorting each row's own JSON object keys, so it matches
/// what the query actually selected even if a client reorders object keys.
fn table_from_json(json: &serde_json::Value) -> QueryResult {
    let columns: Vec<String> = json
        .get("meta")
        .and_then(|m| m.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let cap = query_driver::max_rows();
    let truncated = data.len() > cap;
    let rows: Vec<Vec<String>> = data
        .into_iter()
        .take(cap)
        .map(|row| {
            columns
                .iter()
                .map(|column| {
                    row.get(column)
                        .map(clickhouse_cell_to_string)
                        .unwrap_or_else(|| "NULL".to_string())
                })
                .collect()
        })
        .collect();
    QueryResult::Table {
        columns,
        rows,
        truncated,
    }
}

mod cheatsheet;

#[async_trait]
impl QueryDriver for ClickHouseDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let parsed = parse_target(&self.target)?;
        self.base_url = parsed.base_url;
        self.database = parsed.database;
        self.user = parsed.user;
        self.password = parsed.password;
        // Round-trips a real query rather than just `GET /ping` (which
        // ClickHouse answers with no auth at all) so a wrong user/password
        // or a database that doesn't exist fails right here, the same way
        // every other connector's `connect()` proves real credentials work
        // rather than just "something answered".
        self.run_sql("SELECT 1").await?;
        Ok(())
    }

    fn cheatsheet(&self) -> &'static [tradar_query::CheatEntry] {
        crate::cheatsheet::ENTRIES
    }

    fn keywords(&self) -> &'static [&'static str] {
        // ClickHouse's SQL is close enough to ANSI (SELECT/WHERE/JOIN/GROUP
        // BY/...) that the shared vocabulary is still the right suggestion
        // set -- same reasoning that already lets Postgres, SQLite and
        // Cassandra share it rather than each carrying a dialect-specific
        // extra list.
        query_driver::SQL_KEYWORDS
    }

    fn split_statements(&self, text: &str) -> Vec<query_driver::Statement> {
        query_driver::split_sql_statements(text)
    }

    /// Cheapest possible liveness check: ClickHouse's `/ping` endpoint
    /// answers `Ok.` with no auth and no query parsing at all, unlike
    /// `connect()`'s `SELECT 1` -- exactly the "cheap round trip" this
    /// method's own contract asks for, run every 15s in the background.
    async fn ping(&self) -> anyhow::Result<()> {
        let response = self
            .client
            .get(format!("{}/ping", self.base_url))
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("clickhouse ping failed with status {}", response.status());
        }
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let excluded = SYSTEM_DATABASES
            .iter()
            .map(|db| format!("'{db}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT database, table, name, type, is_in_primary_key \
             FROM system.columns \
             WHERE database NOT IN ({excluded}) \
             ORDER BY database, table, position \
             FORMAT JSON"
        );
        let body = self.run_sql(&sql).await?;
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("could not parse ClickHouse JSON response: {e}"))?;
        let QueryResult::Table { rows, .. } = table_from_json(&json) else {
            unreachable!("table_from_json always returns Table");
        };

        let mut schema: Vec<SchemaInfo> = Vec::new();
        for row in rows {
            let [database, table, column, type_name, is_primary_key]: [String; 5] =
                row.try_into().map_err(|row: Vec<String>| {
                    anyhow::anyhow!("expected 5 columns from system.columns, got {}", row.len())
                })?;
            let same_as_last = schema
                .last()
                .is_some_and(|s| s.schema.as_deref() == Some(database.as_str()) && s.name == table);
            if !same_as_last {
                schema.push(SchemaInfo {
                    schema: Some(database),
                    ..SchemaInfo::new(table)
                });
            }
            let entry = schema.last_mut().expect("just pushed");
            entry.columns.push(ColumnInfo {
                name: column,
                type_name,
                // ClickHouse's "primary key" is a sorting/index expression,
                // not a uniqueness constraint the way Postgres/SQLite mean
                // it -- reported as-is (`system.columns.is_in_primary_key`)
                // since it's still the closest thing this backend has to
                // one, but nothing here relies on it addressing a single
                // row the way row-edit would (not implemented -- see the
                // module doc comment).
                primary_key: is_primary_key == "1",
                foreign_key: None,
                indexed: false,
            });
        }
        query_driver::qualify_colliding_names(&mut schema);
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        if !query_driver::returns_rows(query) {
            self.run_sql(query).await?;
            // ClickHouse's HTTP interface doesn't report how many rows an
            // `INSERT`/`ALTER`/... touched -- `0` is the same "ran, nothing
            // tabular to show" placeholder Postgres already uses for its
            // own transaction-control statements, not a real count.
            return Ok(QueryResult::Affected { rows: 0 });
        }
        let sql = if has_format_clause(query) {
            query.to_string()
        } else {
            format!(
                "{} FORMAT JSON",
                query.trim().trim_end_matches(';').trim_end()
            )
        };
        let body = self.run_sql(&sql).await?;
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("could not parse ClickHouse JSON response: {e}"))?;
        Ok(table_from_json(&json))
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
        "clickhouse"
    }

    async fn open(&self, target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
        let mut driver = ClickHouseDriver::new(target);
        tradar_query::with_connect_timeout(target, driver.connect()).await?;
        Ok(Arc::new(driver))
    }
}

#[cfg(test)]
mod tests;
