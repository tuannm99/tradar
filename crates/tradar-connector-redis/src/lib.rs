//! Redis connector: one command line per execution, naive whitespace
//! parsing. Most replies get a generic RESP-to-JSON conversion; HGETALL and
//! ZRANGE/ZREVRANGE ... WITHSCORES get type-aware formatting so their flat
//! arrays don't lose the field/value or member/score pairing.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_query::{MAX_ROWS, QueryDriver, QueryResult, SchemaInfo, Statement};

mod browse;
mod command;
mod reply;

use browse::*;
use command::*;
use reply::*;

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
        op: tradar_query::CrudOp,
        _columns: &[String],
    ) -> Option<String> {
        let kind = entry.kind.as_deref()?;
        let key = quote_arg(&entry.name);
        let browse_kind = BrowseKind::parse(kind)?;
        Some(match op {
            tradar_query::CrudOp::Read => browse_kind.command(&entry.name),
            tradar_query::CrudOp::Create | tradar_query::CrudOp::Update => match browse_kind {
                BrowseKind::String => format!("SET {key} <value>"),
                BrowseKind::Hash => format!("HSET {key} <field> <value>"),
                BrowseKind::List => format!("RPUSH {key} <value>"),
                BrowseKind::Set => format!("SADD {key} <member>"),
                BrowseKind::Zset => format!("ZADD {key} <score> <member>"),
            },
            tradar_query::CrudOp::Delete => format!("DEL {key}"),
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
        "redis"
    }

    async fn open(&self, target: &str) -> anyhow::Result<Arc<dyn QueryDriver>> {
        let mut driver = RedisDriver::new(target);
        tradar_query::with_connect_timeout(target, driver.connect()).await?;
        Ok(Arc::new(driver))
    }
}

#[cfg(test)]
mod tests;
