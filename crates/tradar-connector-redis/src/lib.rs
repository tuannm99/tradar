//! Redis connector: one command line per execution, naive whitespace
//! parsing. Most replies get a generic RESP-to-JSON conversion; HGETALL and
//! ZRANGE/ZREVRANGE ... WITHSCORES get type-aware formatting so their flat
//! arrays don't lose the field/value or member/score pairing.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{QueryDriver, QueryResult, SchemaInfo, Statement};
use tradar_query_workbench::query_engine::QueryEngine;

struct RedisDriver {
    url: String,
    connection: Option<redis::aio::MultiplexedConnection>,
}

impl RedisDriver {
    fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            connection: None,
        }
    }

    /// `entry`'s browse type and the command that fetches its full value --
    /// the one piece of logic `browse_entry` (runs it) and `browse_command`
    /// (echoes it) both need, kept in one place so they can't drift apart.
    fn browse_kind_and_command(&self, entry: &SchemaInfo) -> Option<(BrowseKind, String)> {
        let kind = entry.kind.as_deref()?;
        let browse_kind = BrowseKind::parse(kind)?;
        let command = browse_kind.command(&entry.name);
        Some((browse_kind, command))
    }
}

#[async_trait]
impl QueryDriver for RedisDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = redis::Client::open(self.url.as_str())?;
        self.connection = Some(client.get_multiplexed_async_connection().await?);
        Ok(())
    }

    /// Commands worth completing. Redis has hundreds; these are the ones
    /// you type by hand, plus the two this driver formats specially.
    fn keywords(&self) -> &'static [&'static str] {
        &[
            "GET",
            "SET",
            "DEL",
            "EXISTS",
            "EXPIRE",
            "TTL",
            "KEYS",
            "SCAN",
            "TYPE",
            "INCR",
            "DECR",
            "MGET",
            "MSET",
            "HGET",
            "HSET",
            "HGETALL",
            "HDEL",
            "HKEYS",
            "HVALS",
            "LPUSH",
            "RPUSH",
            "LPOP",
            "RPOP",
            "LRANGE",
            "LLEN",
            "SADD",
            "SREM",
            "SMEMBERS",
            "SCARD",
            "ZADD",
            "ZRANGE",
            "ZREVRANGE",
            "ZSCORE",
            "ZCARD",
            "WITHSCORES",
            "INFO",
            "DBSIZE",
            "FLUSHDB",
            "PING",
        ]
    }

    /// One command per line -- this driver runs a single whitespace-split
    /// command, so a line is exactly a statement.
    fn split_statements(&self, text: &str) -> Vec<Statement> {
        let mut statements = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let start = offset + (line.len() - line.trim_start().len());
                statements.push(Statement {
                    text: trimmed.to_string(),
                    start,
                    end: start + trimmed.len(),
                });
            }
            offset += line.len();
        }
        statements
    }

    async fn ping(&self) -> anyhow::Result<()> {
        let mut connection = self
            .connection
            .clone()
            .expect("connect() must be called first");
        let _: String = redis::cmd("PING").query_async(&mut connection).await?;
        Ok(())
    }

    /// Every key, each with its Redis type and TTL -- what the browse
    /// sidebar lists. SCAN is paginated 100 keys at a time and looped to
    /// completion, then every key gets one `TYPE`+`TTL` round trip
    /// (pipelined together, since both target the same key): N+1 round
    /// trips for N keys, the same trade-off already accepted for MongoDB's
    /// per-collection `find_one` in `list_schema`. Fine for a keyspace of
    /// ordinary size; worth pipelining across keys too if a very large one
    /// ever makes this slow in practice.
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let mut connection = self
            .connection
            .clone()
            .expect("connect() must be called first");

        let mut keys = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut connection)
                .await?;
            keys.extend(batch);
            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        let mut schema = Vec::with_capacity(keys.len());
        for name in keys {
            let (kind, ttl): (String, i64) = redis::pipe()
                .cmd("TYPE")
                .arg(&name)
                .cmd("TTL")
                .arg(&name)
                .query_async(&mut connection)
                .await?;
            schema.push(SchemaInfo {
                name,
                columns: Vec::new(),
                kind: Some(kind),
                // -1 = no expiry set, -2 = key gone between SCAN and here
                // (a race, not an error) -- neither is a duration to show.
                ttl: (ttl >= 0).then_some(ttl),
                // Redis has neither a schema/database level (`SELECT 0-15`
                // is out of scope, see docs/roadmap.md) nor more than one
                // object kind -- `flatten_outline` skips both grouping
                // levels whenever every entry leaves these `None`.
                schema: None,
                object_kind: None,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let mut connection = self
            .connection
            .clone()
            .expect("connect() must be called first");
        let parts: Vec<&str> = query.split_whitespace().collect();
        let (command, args) = parts
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;

        let mut cmd = redis::cmd(command);
        for arg in args {
            cmd.arg(*arg);
        }
        let value: redis::Value = cmd.query_async(&mut connection).await?;

        Ok(QueryResult::Documents(vec![shape_reply(
            command, args, &value,
        )]))
    }

    /// The browse sidebar's Enter action: run the command that shows
    /// `entry`'s full value, shaped as a `Table` specific to its Redis
    /// type -- see "Redis: key browser" in `docs/backlog/mockup-ui-2026-08-15.md`. Delegates to
    /// `execute()` for the actual round trip (same RESP-to-JSON handling
    /// console mode uses), then reshapes that into rows/columns.
    async fn browse_entry(&self, entry: &SchemaInfo) -> anyhow::Result<QueryResult> {
        let kind = entry.kind.as_deref().unwrap_or_default();
        let (browse_kind, command) = self
            .browse_kind_and_command(entry)
            .ok_or_else(|| anyhow::anyhow!("no browse view for Redis type '{kind}'"))?;
        let result = self.execute(&command).await?;
        Ok(reshape_for_browse(browse_kind, result))
    }

    /// The full echo line for `entry`'s browse command -- `<target>>
    /// <command>`, e.g. `127.0.0.1:6379> HGETALL user:1`. Formats the
    /// target itself (stripping the `redis://` scheme) rather than handing
    /// back a bare command for the caller to prefix: `tradar-query-
    /// workbench` must stay generic across connectors, so it only ever
    /// gets an opaque string to print verbatim, the same way
    /// `format_pg_error` keeps all its Postgres-specific formatting inside
    /// that connector.
    fn browse_command(&self, entry: &SchemaInfo) -> Option<String> {
        let (_, command) = self.browse_kind_and_command(entry)?;
        let target = self.url.strip_prefix("redis://").unwrap_or(&self.url);
        Some(format!("{target}> {command}"))
    }

    /// Reuses `BrowseKind` (see "Redis: key browser" in `docs/backlog/mockup-ui-2026-08-15.md`)
    /// for the Read op -- the browse view's command already *is* "show me
    /// this key's full value". Create/Update share a command per type
    /// (Redis's own `SET`/`HSET`/... already overwrite rather than
    /// distinguishing "new" from "changed"); Delete is always `DEL`.
    fn crud_snippet(&self, entry: &SchemaInfo, op: tradar_core::action::CrudOp) -> Option<String> {
        let kind = entry.kind.as_deref()?;
        let key = &entry.name;
        let browse_kind = BrowseKind::parse(kind)?;
        Some(match op {
            tradar_core::action::CrudOp::Read => browse_kind.command(key),
            tradar_core::action::CrudOp::Create | tradar_core::action::CrudOp::Update => {
                match browse_kind {
                    BrowseKind::String => format!("SET {key} <value>"),
                    BrowseKind::Hash => format!("HSET {key} <field> <value>"),
                    BrowseKind::List => format!("RPUSH {key} <value>"),
                    BrowseKind::Set => format!("SADD {key} <member>"),
                    BrowseKind::Zset => format!("ZADD {key} <score> <member>"),
                }
            }
            tradar_core::action::CrudOp::Delete => format!("DEL {key}"),
        })
    }
}

/// The five Redis types the browse sidebar has a specialized view for --
/// see item 4 ("Redis: key browser") in `docs/backlog/mockup-ui-2026-08-15.md`. Streams and
/// other types aren't in scope yet: `parse` returns `None` for them, which
/// `browse_entry` turns into a clear "not supported" error rather than a
/// silent fallback to something misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowseKind {
    String,
    Hash,
    List,
    Set,
    Zset,
}

impl BrowseKind {
    fn parse(kind: &str) -> Option<Self> {
        Some(match kind {
            "string" => Self::String,
            "hash" => Self::Hash,
            "list" => Self::List,
            "set" => Self::Set,
            "zset" => Self::Zset,
            _ => return None,
        })
    }

    /// The command that fetches `key`'s full value for this type.
    fn command(self, key: &str) -> String {
        match self {
            Self::String => format!("GET {key}"),
            Self::Hash => format!("HGETALL {key}"),
            Self::List => format!("LRANGE {key} 0 -1"),
            Self::Set => format!("SMEMBERS {key}"),
            Self::Zset => format!("ZRANGE {key} 0 -1 WITHSCORES"),
        }
    }
}

/// Converts `execute()`'s `Documents` reply for one of `BrowseKind`'s
/// commands into the `Table` shape the browse view wants (field/value
/// rows for a hash, index/value for a list, ...). Falls back to returning
/// `result` unchanged if it isn't the single-`Documents` shape `execute()`
/// always produces -- defensive, not expected to trigger.
fn reshape_for_browse(kind: BrowseKind, result: QueryResult) -> QueryResult {
    let QueryResult::Documents(docs) = &result else {
        return result;
    };
    let Some(value) = docs.first() else {
        return result;
    };

    let (columns, rows): (Vec<String>, Vec<Vec<String>>) = match kind {
        BrowseKind::String => (vec!["value".to_string()], vec![vec![json_to_cell(value)]]),
        BrowseKind::Hash => (
            vec!["field".to_string(), "value".to_string()],
            value
                .as_object()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(field, v)| vec![field.clone(), json_to_cell(v)])
                        .collect()
                })
                .unwrap_or_default(),
        ),
        BrowseKind::List => (
            vec!["index".to_string(), "value".to_string()],
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, v)| vec![index.to_string(), json_to_cell(v)])
                        .collect()
                })
                .unwrap_or_default(),
        ),
        BrowseKind::Set => (
            vec!["member".to_string()],
            value
                .as_array()
                .map(|items| items.iter().map(|v| vec![json_to_cell(v)]).collect())
                .unwrap_or_default(),
        ),
        BrowseKind::Zset => (
            vec!["member".to_string(), "score".to_string()],
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|pair| {
                            let member = pair.get("member").map(json_to_cell).unwrap_or_default();
                            let score = pair.get("score").map(json_to_cell).unwrap_or_default();
                            vec![member, score]
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
    };

    QueryResult::Table {
        columns,
        rows,
        truncated: false,
    }
}

/// A JSON leaf as a plain grid cell: a string as-is, null as empty, and
/// anything else (number, bool) via its JSON text -- these only ever come
/// from `value_to_json`'s output, never nested objects/arrays.
fn json_to_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn shape_reply(command: &str, args: &[&str], value: &redis::Value) -> serde_json::Value {
    match command.to_ascii_uppercase().as_str() {
        "HGETALL" => hgetall_to_object(value).unwrap_or_else(|| value_to_json(value)),
        "ZRANGE" | "ZREVRANGE" if args.iter().any(|a| a.eq_ignore_ascii_case("withscores")) => {
            zrange_withscores_to_pairs(value).unwrap_or_else(|| value_to_json(value))
        }
        _ => value_to_json(value),
    }
}

fn hgetall_to_object(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut object = serde_json::Map::new();
    for pair in items.chunks(2) {
        let [field, val] = pair else { return None };
        object.insert(value_to_string(field)?, value_to_json(val));
    }
    Some(serde_json::Value::Object(object))
}

fn zrange_withscores_to_pairs(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut pairs = Vec::new();
    for pair in items.chunks(2) {
        let [member, score] = pair else { return None };
        let mut entry = serde_json::Map::new();
        entry.insert(
            "member".to_string(),
            serde_json::Value::String(value_to_string(member)?),
        );
        entry.insert("score".to_string(), value_to_json(score));
        pairs.push(serde_json::Value::Object(entry));
    }
    Some(serde_json::Value::Array(pairs))
}

fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        redis::Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

fn value_to_json(value: &redis::Value) -> serde_json::Value {
    match value {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(i) => serde_json::Value::Number((*i).into()),
        redis::Value::BulkString(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).to_string())
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s.clone()),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
        redis::Value::Array(items) | redis::Value::Set(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        redis::Value::Map(pairs) => {
            let mut object = serde_json::Map::new();
            for (key, val) in pairs {
                if let Some(key) = value_to_string(key) {
                    object.insert(key, value_to_json(val));
                }
            }
            serde_json::Value::Object(object)
        }
        redis::Value::Double(d) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        redis::Value::Boolean(b) => serde_json::Value::Bool(*b),
        redis::Value::VerbatimString { text, .. } => serde_json::Value::String(text.clone()),
        _ => serde_json::Value::Null,
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "redis",
    display_name: "Redis",
    icon: "📕",
    capabilities: &[Capability::Query, Capability::Schema],
};

struct RedisConnector;

#[async_trait]
impl Connector for RedisConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = RedisDriver::new(&connection.target);
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(RedisConnector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::redis::{REDIS_PORT, Redis};
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[test]
    fn crud_snippet_covers_all_four_ops_for_a_hash() {
        let driver = RedisDriver::new("redis://127.0.0.1:1");
        let entry = SchemaInfo {
            name: "user:1".to_string(),
            columns: Vec::new(),
            kind: Some("hash".to_string()),
            ttl: None,
            schema: None,
            object_kind: None,
        };

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            Some("HGETALL user:1".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Create),
            Some("HSET user:1 <field> <value>".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Update),
            Some("HSET user:1 <field> <value>".to_string())
        );
        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Delete),
            Some("DEL user:1".to_string())
        );
    }

    #[test]
    fn browse_command_echoes_the_target_and_the_literal_command_browse_entry_would_run() {
        let driver = RedisDriver::new("redis://127.0.0.1:1");
        let entry = SchemaInfo {
            name: "user:1".to_string(),
            columns: Vec::new(),
            kind: Some("hash".to_string()),
            ttl: None,
            schema: None,
            object_kind: None,
        };

        assert_eq!(
            driver.browse_command(&entry),
            Some("127.0.0.1:1> HGETALL user:1".to_string())
        );
    }

    #[test]
    fn browse_command_is_none_for_an_unknown_type() {
        let driver = RedisDriver::new("redis://127.0.0.1:1");
        let entry = SchemaInfo {
            name: "events".to_string(),
            columns: Vec::new(),
            kind: Some("stream".to_string()),
            ttl: None,
            schema: None,
            object_kind: None,
        };

        assert_eq!(driver.browse_command(&entry), None);
    }

    #[test]
    fn crud_snippet_is_none_for_an_unknown_type() {
        let driver = RedisDriver::new("redis://127.0.0.1:1");
        let entry = SchemaInfo {
            name: "events".to_string(),
            columns: Vec::new(),
            kind: Some("stream".to_string()),
            ttl: None,
            schema: None,
            object_kind: None,
        };

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            None
        );
    }

    #[tokio::test]
    async fn connect_succeeds_for_a_running_redis() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_hgetall_returns_a_json_object() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("HSET user:1 name Ada age 36").await.unwrap();

        let result = driver.execute("HGETALL user:1").await.unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs[0]["name"], "Ada");
                assert_eq!(docs[0]["age"], "36");
            }
            other => panic!("expected Documents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_zrange_withscores_returns_member_score_pairs() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver
            .execute("ZADD leaderboard 10 alice 20 bob")
            .await
            .unwrap();

        let result = driver
            .execute("ZRANGE leaderboard 0 -1 WITHSCORES")
            .await
            .unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(
                    docs[0],
                    serde_json::json!([
                        {"member": "alice", "score": "10"},
                        {"member": "bob", "score": "20"}
                    ])
                );
            }
            other => panic!("expected Documents, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_schema_returns_existing_keys() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("SET greeting hello").await.unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema.iter().any(|entry| entry.name == "greeting"),
            "schema was: {:?}",
            schema.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn list_schema_reports_each_key_s_type() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("SET greeting hello").await.unwrap();
        driver.execute("HSET user:1 name Ada").await.unwrap();

        let schema = driver.list_schema().await.unwrap();

        let kind_of = |name: &str| {
            schema
                .iter()
                .find(|entry| entry.name == name)
                .and_then(|entry| entry.kind.clone())
        };
        assert_eq!(kind_of("greeting"), Some("string".to_string()));
        assert_eq!(kind_of("user:1"), Some("hash".to_string()));
    }

    #[tokio::test]
    async fn list_schema_pages_past_the_first_scan_batch() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        for i in 0..250 {
            driver.execute(&format!("SET key:{i} v")).await.unwrap();
        }

        let schema = driver.list_schema().await.unwrap();

        assert_eq!(
            schema.len(),
            250,
            "a single 100-key SCAN batch must not be the whole answer"
        );
    }

    #[tokio::test]
    async fn browse_entry_shapes_a_string_as_a_one_row_table() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("SET greeting hello").await.unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "greeting".to_string(),
                columns: Vec::new(),
                kind: Some("string".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await
            .unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["value".to_string()],
                rows: vec![vec!["hello".to_string()]],
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn browse_entry_shapes_a_hash_as_field_value_rows() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("HSET user:1 name Ada age 36").await.unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "user:1".to_string(),
                columns: Vec::new(),
                kind: Some("hash".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await
            .unwrap();

        match result {
            QueryResult::Table {
                columns,
                rows,
                truncated,
            } => {
                assert_eq!(columns, vec!["field".to_string(), "value".to_string()]);
                assert!(!truncated);
                assert!(rows.contains(&vec!["name".to_string(), "Ada".to_string()]));
                assert!(rows.contains(&vec!["age".to_string(), "36".to_string()]));
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn browse_entry_shapes_a_list_as_index_value_rows() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("RPUSH queue a b c").await.unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "queue".to_string(),
                columns: Vec::new(),
                kind: Some("list".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await
            .unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["index".to_string(), "value".to_string()],
                rows: vec![
                    vec!["0".to_string(), "a".to_string()],
                    vec!["1".to_string(), "b".to_string()],
                    vec!["2".to_string(), "c".to_string()],
                ],
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn browse_entry_shapes_a_set_as_member_rows() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("SADD tags vip early-adopter").await.unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "tags".to_string(),
                columns: Vec::new(),
                kind: Some("set".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await
            .unwrap();

        match result {
            QueryResult::Table {
                columns,
                rows,
                truncated,
            } => {
                assert_eq!(columns, vec!["member".to_string()]);
                assert!(!truncated);
                assert!(rows.contains(&vec!["vip".to_string()]));
                assert!(rows.contains(&vec!["early-adopter".to_string()]));
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn browse_entry_shapes_a_zset_as_member_score_rows() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver
            .execute("ZADD leaderboard 10 alice 20 bob")
            .await
            .unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "leaderboard".to_string(),
                columns: Vec::new(),
                kind: Some("zset".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await
            .unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["member".to_string(), "score".to_string()],
                rows: vec![
                    vec!["alice".to_string(), "10".to_string()],
                    vec!["bob".to_string(), "20".to_string()],
                ],
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn browse_entry_on_an_unsupported_type_fails_clearly() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("XADD events * field value").await.unwrap();

        let result = driver
            .browse_entry(&SchemaInfo {
                name: "events".to_string(),
                columns: Vec::new(),
                kind: Some("stream".to_string()),
                ttl: None,
                schema: None,
                object_kind: None,
            })
            .await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("stream"),
            "error should name the unsupported type: {error}"
        );
    }
}
