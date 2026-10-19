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
    self as query_driver, QueryDriver, QueryResult, SchemaInfo, Statement,
};
use tradar_query_workbench::query_engine::QueryEngine;

mod edit;
mod request;
mod response;
mod schema;

use edit::*;
use request::*;
use response::*;
use schema::*;

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
    ///
    /// Both statements carry `?refresh=true`: Elasticsearch makes a write
    /// searchable only on its next refresh (about a second), and the UI
    /// re-runs the original `_search` right after an edit -- without this it
    /// would show the old value for a change that had in fact been applied
    /// (found by editing a hit against a real cluster).
    fn edit_sql(&self, edit: &query_driver::RowEdit) -> Option<String> {
        let index = &edit.table;
        let id = edit.key.iter().find(|(k, _)| k == "_id")?.1.as_str();
        Some(match &edit.change {
            query_driver::RowChange::SetValue { column, value } => {
                let doc = nest_dotted_path(column, es_infer_value(value));
                let body = serde_json::json!({ "doc": doc });
                format!(
                    "POST {index}/_update/{id}?refresh=true\n{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                )
            }
            query_driver::RowChange::SetValues(pairs) => {
                // One partial document with every field, nested paths merged
                // (`a.b` and `a.c` both land inside the same `a` object).
                let mut doc = serde_json::Value::Object(serde_json::Map::new());
                for (column, value) in pairs {
                    merge_json(&mut doc, nest_dotted_path(column, es_infer_value(value)));
                }
                let body = serde_json::json!({ "doc": doc });
                format!(
                    "POST {index}/_update/{id}?refresh=true\n{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                )
            }
            query_driver::RowChange::DeleteRow => {
                format!("DELETE {index}/_doc/{id}?refresh=true")
            }
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

#[cfg(test)]
mod tests;
