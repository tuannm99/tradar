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

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

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

impl ClickHouseDriver {
    fn new(target: &str) -> Self {
        Self {
            target: target.to_string(),
            base_url: String::new(),
            database: None,
            user: None,
            password: None,
            client: reqwest::Client::new(),
        }
    }

    fn build_request(&self, sql: &str) -> reqwest::RequestBuilder {
        let mut request = self.client.post(&self.base_url).body(sql.to_string());
        let mut params: Vec<(&str, &str)> = Vec::new();
        if let Some(database) = &self.database {
            params.push(("database", database));
        }
        if let Some(user) = &self.user {
            params.push(("user", user));
        }
        if let Some(password) = &self.password {
            params.push(("password", password));
        }
        if !params.is_empty() {
            request = request.query(&params);
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
            anyhow::bail!("{}", format_clickhouse_error(status, &text));
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
/// into a clean base URL plus the pieces ClickHouse's HTTP interface takes
/// separately (`?database=`/`?user=`/`?password=`) -- into `reqwest::Url`
/// rather than adding a `url` dependency of this crate's own, since
/// `reqwest` already re-exports it.
fn parse_target(target: &str) -> anyhow::Result<ParsedTarget> {
    let url = reqwest::Url::parse(target)
        .map_err(|e| anyhow::anyhow!("invalid ClickHouse target '{target}': {e}"))?;
    let user = (!url.username().is_empty()).then(|| url.username().to_string());
    let password = url.password().map(str::to_string);
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

fn format_clickhouse_error(status: reqwest::StatusCode, body: &str) -> String {
    let first_line = body.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        format!("clickhouse returned {status}")
    } else {
        format!("clickhouse returned {status}: {first_line}")
    }
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
    let truncated = data.len() > query_driver::MAX_ROWS;
    let rows: Vec<Vec<String>> = data
        .into_iter()
        .take(query_driver::MAX_ROWS)
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

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "clickhouse",
    display_name: "ClickHouse",
    icon: "📊",
    capabilities: &[Capability::Query, Capability::Schema],
};

struct ClickHouseConnector;

#[async_trait]
impl Connector for ClickHouseConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = ClickHouseDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(ClickHouseConnector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::clickhouse::ClickHouse;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[test]
    fn parse_target_splits_userinfo_path_and_leaves_a_clean_base_url() {
        let parsed = parse_target("http://alice:secret@127.0.0.1:8123/analytics").unwrap();

        assert_eq!(parsed.base_url, "http://127.0.0.1:8123");
        assert_eq!(parsed.database.as_deref(), Some("analytics"));
        assert_eq!(parsed.user.as_deref(), Some("alice"));
        assert_eq!(parsed.password.as_deref(), Some("secret"));
    }

    #[test]
    fn parse_target_with_no_userinfo_or_path_leaves_them_none() {
        let parsed = parse_target("http://127.0.0.1:8123").unwrap();

        assert_eq!(parsed.base_url, "http://127.0.0.1:8123");
        assert_eq!(parsed.database, None);
        assert_eq!(parsed.user, None);
        assert_eq!(parsed.password, None);
    }

    #[test]
    fn parse_target_rejects_an_unparseable_url() {
        assert!(parse_target("not a url").is_err());
    }

    #[test]
    fn has_format_clause_is_case_insensitive_and_whole_word() {
        assert!(has_format_clause("select 1 FORMAT JSON"));
        assert!(has_format_clause("select 1 format CSV"));
        assert!(!has_format_clause("select 1"));
        // "format" only as part of a longer identifier must not count.
        assert!(!has_format_clause("select reformatted from t"));
    }

    #[test]
    fn clickhouse_cell_to_string_renders_each_json_shape() {
        assert_eq!(clickhouse_cell_to_string(&serde_json::Value::Null), "NULL");
        assert_eq!(
            clickhouse_cell_to_string(&serde_json::json!("hello")),
            "hello"
        );
        assert_eq!(clickhouse_cell_to_string(&serde_json::json!(42)), "42");
        assert_eq!(clickhouse_cell_to_string(&serde_json::json!(true)), "true");
        assert_eq!(
            clickhouse_cell_to_string(&serde_json::json!([1, 2, 3])),
            "[1,2,3]"
        );
    }

    #[test]
    fn table_from_json_reads_columns_from_meta_in_order() {
        let json = serde_json::json!({
            "meta": [{"name": "b", "type": "String"}, {"name": "a", "type": "UInt8"}],
            "data": [{"a": 1, "b": "x"}, {"a": 2, "b": null}],
        });

        let result = table_from_json(&json);

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["b".to_string(), "a".to_string()],
                rows: vec![
                    vec!["x".to_string(), "1".to_string()],
                    vec!["NULL".to_string(), "2".to_string()],
                ],
                truncated: false,
            }
        );
    }

    async fn started() -> (
        testcontainers_modules::testcontainers::ContainerAsync<ClickHouse>,
        String,
    ) {
        let container = ClickHouse::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(8123).await.unwrap();
        (container, format!("http://127.0.0.1:{port}"))
    }

    #[tokio::test]
    async fn connect_succeeds_against_a_running_server() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn connect_fails_with_a_clear_message_for_a_database_that_does_not_exist() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&format!("{target}/does_not_exist"));

        let error = driver.connect().await.unwrap_err().to_string();

        assert!(
            error.contains("does_not_exist"),
            "error should name the bad database: {error}"
        );
    }

    #[tokio::test]
    async fn execute_runs_ddl_and_dml_then_reads_the_row_back() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);
        driver.connect().await.unwrap();

        driver
            .execute("CREATE TABLE events (id UInt32, name String) ENGINE = Memory")
            .await
            .unwrap();
        let inserted = driver
            .execute("INSERT INTO events VALUES (1, 'signup')")
            .await
            .unwrap();
        assert_eq!(inserted, QueryResult::Affected { rows: 0 });

        let result = driver.execute("SELECT id, name FROM events").await.unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string(), "name".to_string()],
                rows: vec![vec!["1".to_string(), "signup".to_string()]],
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn execute_reports_a_syntax_error_from_clickhouse_itself() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);
        driver.connect().await.unwrap();

        let error = driver
            .execute("SELECT this is not valid clickhouse sql")
            .await
            .unwrap_err()
            .to_string();

        assert!(!error.is_empty());
    }

    #[tokio::test]
    async fn list_schema_reports_the_table_just_created_grouped_by_database() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);
        driver.connect().await.unwrap();
        driver
            .execute("CREATE TABLE orders (id UInt32, total Float64) ENGINE = Memory")
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        let orders = schema
            .iter()
            .find(|entry| entry.name == "orders")
            .expect("the table we just created should be listed");
        assert_eq!(orders.schema.as_deref(), Some("default"));
        let columns: Vec<&str> = orders.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(columns, vec!["id", "total"]);
    }

    #[tokio::test]
    async fn list_schema_never_reports_clickhouse_s_own_system_tables() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);
        driver.connect().await.unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema
                .iter()
                .all(|entry| entry.schema.as_deref() != Some("system")),
            "system tables leaked into the schema list"
        );
    }

    #[tokio::test]
    async fn ping_succeeds_against_a_running_server() {
        let (_container, target) = started().await;
        let mut driver = ClickHouseDriver::new(&target);
        driver.connect().await.unwrap();

        assert!(driver.ping().await.is_ok());
    }
}
