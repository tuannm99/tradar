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
fn parse_target_decodes_a_percent_encoded_user_and_password() {
    let parsed =
        parse_target("http://me%40corp:p%40ss%3Aw%2Frd%231%20%25x@127.0.0.1:8123/db").unwrap();
    assert_eq!(parsed.user.as_deref(), Some("me@corp"));
    assert_eq!(parsed.password.as_deref(), Some("p@ss:w/rd#1 %x"));
    assert_eq!(parsed.base_url, "http://127.0.0.1:8123");
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
fn clickhouse_error_marker_points_at_the_token_s_position() {
    let body = "Code: 62. DB::Exception: Syntax error: failed at position 18 (is): \
                 is not valid clickhouse sql. Expected one of: ...";
    let sql = "SELECT this is not valid clickhouse sql";

    let marker = clickhouse_error_marker(body, sql).unwrap();

    assert!(marker.contains("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains('^'), "marker was: {marker}");
}

#[test]
fn clickhouse_error_marker_is_none_for_a_message_with_no_position() {
    assert_eq!(
        clickhouse_error_marker(
            "Code: 60. DB::Exception: Table default.t doesn't exist",
            "SELECT * FROM t"
        ),
        None
    );
}

#[test]
fn clickhouse_error_marker_is_none_when_the_token_is_not_actually_in_the_sql() {
    assert_eq!(
        clickhouse_error_marker("failed at position 3 (xyz): ...", "SELECT * FROM t"),
        None
    );
}

#[test]
fn format_clickhouse_error_appends_the_marker_when_one_is_found() {
    let body = "Syntax error: failed at position 18 (is): is not valid clickhouse sql.";
    let sql = "SELECT this is not valid clickhouse sql";

    let message = format_clickhouse_error(reqwest::StatusCode::BAD_REQUEST, body, sql);

    assert!(message.contains("LINE 1:"), "message was: {message}");
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
