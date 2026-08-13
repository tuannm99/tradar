//! Redis connector: one command line per execution, naive whitespace
//! parsing. Most replies get a generic RESP-to-JSON conversion; HGETALL and
//! ZRANGE/ZREVRANGE ... WITHSCORES get type-aware formatting so their flat
//! arrays don't lose the field/value or member/score pairing.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_api::{Connector, ConnectorDescriptor, Session};
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

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let mut connection = self
            .connection
            .clone()
            .expect("connect() must be called first");
        let (_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(0)
            .arg("COUNT")
            .arg(100)
            .query_async(&mut connection)
            .await?;
        Ok(keys.into_iter().map(SchemaInfo::new).collect())
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
        driver.connect().await?;
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
}
