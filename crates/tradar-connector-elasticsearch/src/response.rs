use tradar_query::QueryResult;

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
pub(crate) fn unwrap_search_hits(json: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
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
pub(crate) fn handle_response(
    status: reqwest::StatusCode,
    text: String,
) -> anyhow::Result<QueryResult> {
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
            let cap = tradar_query::max_rows();
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
