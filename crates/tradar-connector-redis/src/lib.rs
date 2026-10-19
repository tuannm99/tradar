//! Redis connector: one command line per execution, naive whitespace
//! parsing. Most replies get a generic RESP-to-JSON conversion; HGETALL and
//! ZRANGE/ZREVRANGE ... WITHSCORES get type-aware formatting so their flat
//! arrays don't lose the field/value or member/score pairing.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    MAX_ROWS, QueryDriver, QueryResult, SchemaInfo, Statement,
};
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
    /// sidebar lists. SCAN is paginated 100 keys at a time and looped,
    /// stopping once `MAX_ROWS` keys have been collected rather than
    /// looping to completion -- a production keyspace can hold millions of
    /// keys, and without this cap the whole thing loads into memory before
    /// the browse sidebar ever appears (unlike every other connector's
    /// `list_schema`, which only reads small, inherently-bounded metadata --
    /// table/column lists, index mappings -- never data at this scale).
    /// Deliberately silent about the cut (no `truncated` flag reaches the
    /// sidebar): that would mean widening `SchemaInfo`/`list_schema`'s
    /// return type for all twelve connectors just for this one driver's
    /// case, a cost this fix doesn't carry; the remaining per-key `TYPE`+
    /// `TTL` round trip (pipelined together, since both target the same
    /// key) is the same N+1 trade-off already accepted for MongoDB's
    /// per-collection `find_one`, now bounded by the same cap.
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
            if keys.len() >= MAX_ROWS || next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }
        keys.truncate(MAX_ROWS);

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
        let tokens = split_command(query)?;
        let parts: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let (command, args) = parts
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;

        let mut cmd = redis::cmd(command);
        for arg in args {
            cmd.arg(*arg);
        }
        let value: redis::Value = cmd.query_async(&mut connection).await?;

        Ok(QueryResult::Documents {
            items: vec![shape_reply(command, args, &value)],
            truncated: false,
        })
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
    /// `columns` is unused: Redis keys have no columns (`SchemaInfo::columns`
    /// is always empty here), so the navigator's column picker never has
    /// anything to offer for a Redis entry and skips straight to inserting
    /// this, same as it always has.
    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_core::action::CrudOp,
        _columns: &[String],
    ) -> Option<String> {
        let kind = entry.kind.as_deref()?;
        let key = quote_arg(&entry.name);
        let browse_kind = BrowseKind::parse(kind)?;
        Some(match op {
            tradar_core::action::CrudOp::Read => browse_kind.command(&entry.name),
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

/// A command line as `redis-cli` reads it: arguments split on whitespace,
/// except that a token *starting* with `"` or `'` runs to the matching quote
/// and may contain spaces -- `SET greeting "hello world"`. Inside `"..."` the
/// escapes `\n \r \t \b \a \\ \" \xHH` are understood (anything else after a
/// backslash is that character); inside `'...'` only `\'` is. A quote in the
/// middle of an unquoted token is just a character, so `SET k it's` keeps
/// working, and an unterminated quote is an error rather than a silent
/// mis-split.
fn split_command(text: &str) -> anyhow::Result<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut token = String::new();
        match chars[i] {
            quote @ ('"' | '\'') => {
                i += 1;
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    if c == quote {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if c == '\\' && i + 1 < chars.len() {
                        i += 1;
                        let e = chars[i];
                        if quote == '\'' {
                            if e == '\'' {
                                token.push('\'');
                            } else {
                                token.push('\\');
                                token.push(e);
                            }
                        } else {
                            match e {
                                'n' => token.push('\n'),
                                'r' => token.push('\r'),
                                't' => token.push('\t'),
                                'b' => token.push('\u{8}'),
                                'a' => token.push('\u{7}'),
                                'x' if i + 2 < chars.len()
                                    && chars[i + 1].is_ascii_hexdigit()
                                    && chars[i + 2].is_ascii_hexdigit() =>
                                {
                                    let hex: String = chars[i + 1..=i + 2].iter().collect();
                                    token.push(u8::from_str_radix(&hex, 16)? as char);
                                    i += 2;
                                }
                                other => token.push(other),
                            }
                        }
                        i += 1;
                        continue;
                    }
                    token.push(c);
                    i += 1;
                }
                if !closed {
                    anyhow::bail!("unterminated {quote} quote in the command");
                }
                if i < chars.len() && !chars[i].is_whitespace() {
                    anyhow::bail!(
                        "a closing quote must be followed by a space or the end of the command"
                    );
                }
            }
            _ => {
                while i < chars.len() && !chars[i].is_whitespace() {
                    token.push(chars[i]);
                    i += 1;
                }
            }
        }
        out.push(token);
    }
    Ok(out)
}

/// `arg` as one token of a command line: bare when it is plain, otherwise
/// double-quoted with the escapes `split_command` reads back -- a key with
/// a space in it must survive being written into a snippet.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
    {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
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
            Self::String => format!("GET {}", quote_arg(key)),
            Self::Hash => format!("HGETALL {}", quote_arg(key)),
            Self::List => format!("LRANGE {} 0 -1", quote_arg(key)),
            Self::Set => format!("SMEMBERS {}", quote_arg(key)),
            Self::Zset => format!("ZRANGE {} 0 -1 WITHSCORES", quote_arg(key)),
        }
    }
}

/// Converts `execute()`'s `Documents` reply for one of `BrowseKind`'s
/// commands into the `Table` shape the browse view wants (field/value
/// rows for a hash, index/value for a list, ...). Falls back to returning
/// `result` unchanged if it isn't the single-`Documents` shape `execute()`
/// always produces -- defensive, not expected to trigger.
fn reshape_for_browse(kind: BrowseKind, result: QueryResult) -> QueryResult {
    let QueryResult::Documents { items, .. } = &result else {
        return result;
    };
    let Some(value) = items.first() else {
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
mod tests;
