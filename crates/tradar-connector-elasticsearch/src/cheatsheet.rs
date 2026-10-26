//! The Elasticsearch cheat sheet -- see `QueryDriver::cheatsheet`. A verb and
//! path, then the JSON body, as this connector reads a request.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Cluster ─────────────────────────────────────────────────────────
    CheatEntry::new("Cluster", "Cluster health", "GET _cluster/health"),
    CheatEntry::new("Cluster", "Nodes", "GET _cat/nodes?format=json"),
    CheatEntry::new(
        "Cluster",
        "Disk allocation per node",
        "GET _cat/allocation?format=json",
    ),
    CheatEntry::new(
        "Cluster",
        "Shards (and where they are)",
        "GET _cat/shards?format=json",
    ),
    CheatEntry::new("Cluster", "Cluster settings", "GET _cluster/settings"),
    CheatEntry::new("Cluster", "Tasks running now", "GET _tasks?detailed=true"),
    // ── Indices ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Indices",
        "Indices by size",
        "GET _cat/indices?format=json&s=store.size:desc",
    ),
    CheatEntry::new("Indices", "Aliases", "GET _cat/aliases?format=json"),
    CheatEntry::new("Indices", "Mapping of an index", "GET ${index}/_mapping"),
    CheatEntry::new("Indices", "Settings of an index", "GET ${index}/_settings"),
    CheatEntry::new(
        "Indices",
        "Document count of an index",
        "GET ${index}/_count",
    ),
    // ── Search ──────────────────────────────────────────────────────────
    CheatEntry::new(
        "Search",
        "All documents (first 10)",
        "GET ${index}/_search\n{\n  \"query\": {\"match_all\": {}},\n  \"size\": 10\n}",
    ),
    CheatEntry::new(
        "Search",
        "Full-text match on a field",
        "GET ${index}/_search\n{\n  \"query\": {\"match\": {\"${field}\": \"${value}\"}},\n  \"size\": 20\n}",
    ),
    CheatEntry::new(
        "Search",
        "Exact value of a field",
        "GET ${index}/_search\n{\n  \"query\": {\"term\": {\"${field}\": \"${value}\"}},\n  \"size\": 20\n}",
    ),
    CheatEntry::new(
        "Search",
        "Last 24 hours, newest first",
        "GET ${index}/_search\n{\n  \"query\": {\"range\": {\"@timestamp\": {\"gte\": \"now-24h\"}}},\n  \"sort\": [{\"@timestamp\": \"desc\"}],\n  \"size\": 20\n}",
    ),
    CheatEntry::new(
        "Search",
        "Top values of a field (terms aggregation)",
        "GET ${index}/_search\n{\n  \"size\": 0,\n  \"aggs\": {\"top\": {\"terms\": {\"field\": \"${field}\", \"size\": 10}}}\n}",
    ),
    CheatEntry::new(
        "Search",
        "Why a query matches (explain a document)",
        "GET ${index}/_explain/${id}\n{\n  \"query\": {\"match\": {\"${field}\": \"${value}\"}}\n}",
    ),
];
