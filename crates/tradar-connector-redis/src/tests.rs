use super::*;
use tradar_query::{MAX_ROWS, QueryDriver, QueryResult, SchemaInfo};

#[test]
fn split_command_splits_on_whitespace_like_before() {
    assert_eq!(
        split_command("  GET   key:1 ").unwrap(),
        vec!["GET", "key:1"]
    );
    assert!(split_command("   ").unwrap().is_empty());
}

#[test]
fn split_command_keeps_a_quoted_value_with_spaces_as_one_argument() {
    assert_eq!(
        split_command("SET greeting \"hello world\"").unwrap(),
        vec!["SET", "greeting", "hello world"]
    );
    assert_eq!(
        split_command("SET k 'it\\'s a  test'").unwrap(),
        vec!["SET", "k", "it's a  test"]
    );
    assert_eq!(split_command("SET k \"\"").unwrap(), vec!["SET", "k", ""]);
}

#[test]
fn split_command_reads_double_quote_escapes() {
    assert_eq!(
        split_command("SET k \"a\\nb\\t\\\"q\\\"\\x41\"").unwrap(),
        vec!["SET", "k", "a\nb\t\"q\"A"]
    );
}

#[test]
fn split_command_treats_a_mid_token_quote_as_a_plain_character() {
    assert_eq!(
        split_command("SET k it's").unwrap(),
        vec!["SET", "k", "it's"]
    );
}

#[test]
fn split_command_rejects_an_unterminated_quote_and_junk_after_a_closing_one() {
    assert!(split_command("SET k \"oops").is_err());
    assert!(split_command("SET k \"a\"b").is_err());
}

#[test]
fn quote_arg_round_trips_through_split_command() {
    for key in [
        "plain:key",
        "has space",
        "quo\"te",
        "back\\slash",
        "tab\there",
        "",
    ] {
        let line = format!("GET {}", quote_arg(key));
        assert_eq!(
            split_command(&line).unwrap(),
            vec!["GET".to_string(), key.to_string()],
            "{line}"
        );
    }
}
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
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &[]),
        Some("HGETALL user:1".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Create, &[]),
        Some("HSET user:1 <field> <value>".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Update, &[]),
        Some("HSET user:1 <field> <value>".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Delete, &[]),
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
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &[]),
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
        QueryResult::Documents { items: docs, .. } => {
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
        QueryResult::Documents { items: docs, .. } => {
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
async fn list_schema_stops_scanning_at_max_rows() {
    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
    driver.connect().await.unwrap();
    // One pipelined MSET rather than MAX_ROWS+1 round trips through
    // `execute()` -- same reasoning as `list_schema`'s own SCAN/pipeline
    // use, just applied to seeding the test data quickly.
    let mut connection = driver.connection.clone().unwrap();
    let mut mset = redis::cmd("MSET");
    for i in 0..=MAX_ROWS {
        mset.arg(format!("key:{i}")).arg("v");
    }
    let _: () = mset.query_async(&mut connection).await.unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert_eq!(
        schema.len(),
        MAX_ROWS,
        "an unbounded keyspace must not load past MAX_ROWS keys"
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

#[test]
fn cheatsheet_is_well_formed() {
    let problems = tradar_query::cheatsheet_problems(crate::cheatsheet::ENTRIES);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// An entry's text with every `${name}` filled with `x`.
fn filled(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        match rest[at..].find('}') {
            Some(end) => rest = &rest[at + end + 1..],
            None => break,
        }
        out.push('x');
    }
    out.push_str(rest);
    out
}

#[test]
fn every_cheatsheet_entry_is_one_command() {
    let driver = RedisDriver::new("redis://127.0.0.1:1");
    for entry in crate::cheatsheet::ENTRIES {
        let text = filled(entry.text);
        assert_eq!(driver.split_statements(&text).len(), 1, "`{}`", entry.title);
        let words = crate::command::split_command(&text).unwrap();
        assert!(
            !words.is_empty() && words[0] == words[0].to_uppercase(),
            "`{}`",
            entry.title
        );
    }
}

/// Every cheat sheet entry, run against a real Redis (Docker).
const CHEAT_MAY_FAIL: &[&str] = &[
    // No such client / the allocator may not support it.
    "Close a client by id",
    "Free unused memory",
];

fn cheat_value(name: &str, text: &str) -> Option<String> {
    let command = text.split_whitespace().next().unwrap_or("");
    let first = command.to_uppercase();
    let first = first.as_str();
    Some(
        match name {
            "key" => match first {
                "HGETALL" | "HGET" | "HKEYS" | "HLEN" | "HSET" | "HDEL" | "HINCRBY" => "k_hash",
                "LRANGE" | "LLEN" | "RPUSH" | "LPUSH" | "LPOP" | "RPOP" => "k_list",
                "SMEMBERS" | "SCARD" | "SISMEMBER" | "SRANDMEMBER" | "SADD" | "SREM" => "k_set",
                "ZRANGE" | "ZRANGEBYSCORE" | "ZSCORE" | "ZRANK" | "ZCARD" | "ZADD" | "ZREM"
                | "ZINCRBY" => "k_zset",
                "XRANGE" | "XREVRANGE" | "XLEN" | "XINFO" | "XADD" | "XDEL" => "k_stream",
                "PFCOUNT" => "k_hll",
                "INCR" | "INCRBY" => "k_num",
                _ => "k_str",
            },
            "other_key" => "k_str2",
            "new_key" => "k_new",
            "other_value" => "v2",
            "value" if first == "CONFIG" => "10",
            "value" => "v",
            "parameter" => "hz",
            "pattern" if first == "CONFIG" => "hz",
            "pattern" if first == "PUBSUB" => "*",
            "pattern" => "k_*",
            "field" if first == "HINCRBY" => "f_num",
            "field" if first == "XADD" => "f",
            "field" => "f_str",
            "member" => "m1",
            "score" | "amount" => "1",
            "seconds" => "100",
            "type" => "string",
            "entry_id" => "1-1",
            "channel" => "ch",
            "message" => "hello",
            "id" => "999999",
            _ => return None,
        }
        .to_string(),
    )
}

const CHEAT_SEED: &[&str] = &[
    "FLUSHALL",
    "SET k_str v",
    "SET k_str2 v2",
    "SET k_num 1",
    "HSET k_hash f_str v f_num 1",
    "RPUSH k_list a b c",
    "SADD k_set m1 m2",
    "ZADD k_zset 1 m1 2 m2",
    "XADD k_stream 1-1 f v",
    "PFADD k_hll a b c",
];

#[tokio::test]
async fn every_cheatsheet_entry_runs_on_a_real_server() {
    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
    driver.connect().await.unwrap();
    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in crate::cheatsheet::ENTRIES {
        for command in CHEAT_SEED {
            driver
                .execute(command)
                .await
                .unwrap_or_else(|e| panic!("fixture `{command}`: {e}"));
        }
        let text = match tradar_query::fill_placeholders(entry.text, |n| cheat_value(n, entry.text))
        {
            Ok(text) => text,
            Err(name) => {
                failures.push(format!("`{}`: no value for `${{{name}}}`", entry.title));
                continue;
            }
        };
        ran += 1;
        if let Err(e) = driver.execute(&text).await
            && !CHEAT_MAY_FAIL.contains(&entry.title)
        {
            failures.push(format!("`{}`: {e}\n{text}", entry.title));
        }
    }
    assert!(ran > 60, "only {ran} entries ran");
    assert!(
        failures.is_empty(),
        "{} of {ran} cheat sheet entries failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
