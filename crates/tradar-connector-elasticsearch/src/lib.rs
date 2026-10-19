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

/// Kibana Dev Tools lets a request carry comments -- `# line` anywhere,
/// `// line` and `/* block */` inside the body -- and a commented-out clause
/// is the most ordinary thing to leave in a query buffer, so they are
/// dropped before the request is read (a body with one would otherwise fail
/// the local JSON check). `#` only counts at the start of a line (a JSON
/// string cannot span lines, so no body line legitimately starts with it);
/// `//` and `/* */` are only looked for in the body, string-aware, so a
/// path or a `"http://..."` value is never mistaken for one.
fn strip_comments(query: &str) -> String {
    let query: String = query
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let query = query.trim_start();
    let (request_line, body) = match query.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => return query.to_string(),
    };
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i = (i + 2).min(chars.len());
                out.push(' ');
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    format!("{request_line}\n{out}")
}

fn parse_query(query: &str) -> Option<(String, String, Option<String>)> {
    let query = strip_comments(query);
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
    // A search over several indices (`logs-*`, `a,b`) mixes hits whose `_id`
    // alone does not say where they live, so `_index` is kept then. A
    // single-index search leaves it out -- every row would repeat it.
    let mut indices = hits.iter().filter_map(|hit| hit.get("_index"));
    let first = indices.next();
    let several_indices = indices.any(|index| Some(index) != first);
    Some(
        hits.iter()
            .map(|hit| {
                let mut doc = serde_json::Map::new();
                if let Some(id) = hit.get("_id") {
                    doc.insert("_id".to_string(), id.clone());
                }
                if several_indices && let Some(index) = hit.get("_index") {
                    doc.insert("_index".to_string(), index.clone());
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
    //
    // Except when the response also carries `aggregations` or `suggest`:
    // those are the answer (often with `"size": 0`, so no hits at all), and
    // unwrapping would throw them away and leave an empty grid. The whole
    // response is shown then, hits included, as JSON.
    let keeps_whole_response = json.get("aggregations").is_some() || json.get("suggest").is_some();
    match unwrap_search_hits(&json).filter(|_| !keeps_whole_response) {
        Some(mut docs) => {
            // A `size` of thousands is legal for ES; the row cap still holds
            // for what we keep (and the stored copy).
            let cap = tradar_query_workbench::query_driver::max_rows();
            let truncated = docs.len() > cap;
            docs.truncate(cap);
            Ok(QueryResult::Documents {
                items: docs,
                truncated,
            })
        }
        None => Ok(QueryResult::Documents {
            items: vec![json],
            truncated: false,
        }),
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
/// Deep-merges `patch` into `target` (objects merge key by key, anything else
/// is replaced) -- how two dotted paths under one parent share its object.
fn merge_json(target: &mut serde_json::Value, patch: serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(into), serde_json::Value::Object(from)) => {
            for (key, value) in from {
                match into.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        into.insert(key, value);
                    }
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

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
mod tests;
