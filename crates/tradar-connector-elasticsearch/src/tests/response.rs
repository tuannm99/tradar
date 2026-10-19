use super::*;

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
