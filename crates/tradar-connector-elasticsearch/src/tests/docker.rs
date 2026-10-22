use super::*;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

/// One Elasticsearch node at a time, with a small heap and a JDK that
/// understands cgroup v2: each JVM otherwise
/// sizes its heap from the machine's RAM, and the test harness starting
/// every test's node at once ran a CI runner out of memory (exit 137).
static ONE_NODE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn node() -> testcontainers_modules::testcontainers::ContainerAsync<ElasticSearch> {
    // The module's default 7.16.1 bundles a JDK whose cgroup v2 detection
    // throws on some CI hosts, and the node exits before it is ready; 7.17.x
    // (the same tag `docker-compose.yml` uses for dev) has a fixed JDK.
    ElasticSearch::default()
        .with_tag("7.17.26")
        .with_env_var("ES_JAVA_OPTS", "-Xms512m -Xmx512m")
        .start()
        .await
        .unwrap()
}

#[tokio::test]
async fn connect_succeeds_for_a_running_cluster() {
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
    let port = container.get_host_port_ipv4(9200).await.unwrap();
    let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));

    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn list_schema_reports_index_fields_from_the_mapping() {
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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

#[tokio::test]
async fn list_schema_returns_created_indices() {
    let _turn = ONE_NODE.lock().await;
    let container = node().await;
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
