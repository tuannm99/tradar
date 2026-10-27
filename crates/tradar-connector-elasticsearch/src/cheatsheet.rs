//! The Elasticsearch cheat sheet (a verb and path, then the JSON body) -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Cluster ──
    CheatEntry::new("Cluster", "Cluster health", r##"GET _cluster/health"##),
    CheatEntry::new(
        "Cluster",
        "Cluster health per index",
        r##"GET _cluster/health?level=indices"##,
    ),
    CheatEntry::new("Cluster", "Version and cluster name", r##"GET /"##),
    CheatEntry::new("Cluster", "Nodes", r##"GET _cat/nodes?format=json"##),
    CheatEntry::new("Cluster", "Master node", r##"GET _cat/master?format=json"##),
    CheatEntry::new(
        "Cluster",
        "Disk allocation per node",
        r##"GET _cat/allocation?format=json"##,
    ),
    CheatEntry::new("Cluster", "JVM heap per node", r##"GET _nodes/stats/jvm"##),
    CheatEntry::new(
        "Cluster",
        "Thread pools (queues and rejections)",
        r##"GET _cat/thread_pool?format=json"##,
    ),
    CheatEntry::new("Cluster", "Plugins", r##"GET _cat/plugins?format=json"##),
    CheatEntry::new("Cluster", "Cluster settings", r##"GET _cluster/settings"##),
    CheatEntry::new(
        "Cluster",
        "Pending cluster tasks",
        r##"GET _cluster/pending_tasks"##,
    ),
    CheatEntry::new(
        "Cluster",
        "Tasks running now",
        r##"GET _tasks?detailed=true"##,
    ),
    CheatEntry::new(
        "Cluster",
        "Why a shard is not allocated",
        r##"GET _cluster/allocation/explain"##,
    ),
    // ── Indices ──
    CheatEntry::new(
        "Indices",
        "Indices by size",
        r##"GET _cat/indices?format=json&s=store.size:desc"##,
    ),
    CheatEntry::new(
        "Indices",
        "Shards (and where they are)",
        r##"GET _cat/shards?format=json"##,
    ),
    CheatEntry::new(
        "Indices",
        "Shards not started, with the reason",
        r##"GET _cat/shards?format=json&h=index,shard,prirep,state,unassigned.reason&s=state"##,
    ),
    CheatEntry::new(
        "Indices",
        "Shard recoveries in progress",
        r##"GET _cat/recovery?format=json&active_only=true"##,
    ),
    CheatEntry::new("Indices", "Aliases", r##"GET _cat/aliases?format=json"##),
    CheatEntry::new(
        "Indices",
        "Mapping of an index",
        r##"GET ${index}/_mapping"##,
    ),
    CheatEntry::new(
        "Indices",
        "Settings of an index",
        r##"GET ${index}/_settings"##,
    ),
    CheatEntry::new(
        "Indices",
        "Document count of an index",
        r##"GET ${index}/_count"##,
    ),
    CheatEntry::new(
        "Indices",
        "Statistics of an index",
        r##"GET ${index}/_stats"##,
    ),
    CheatEntry::new(
        "Indices",
        "Segments of an index",
        r##"GET _cat/segments/${index}?format=json"##,
    ),
    CheatEntry::new(
        "Indices",
        "Fields of an index and their types",
        r##"GET ${index}/_field_caps?fields=*"##,
    ),
    CheatEntry::new("Indices", "Index templates", r##"GET _index_template"##),
    CheatEntry::new(
        "Indices",
        "How a text is split into terms (analyze)",
        r##"POST _analyze
{
  "analyzer": "standard",
  "text": "${text}"
}"##,
    ),
    // ── Search ──
    CheatEntry::new(
        "Search",
        "All documents (first 10)",
        r##"GET ${index}/_search
{
  "query": {"match_all": {}},
  "size": 10
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Full-text match on a field",
        r##"GET ${index}/_search
{
  "query": {"match": {"${field}": "${value}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Exact phrase",
        r##"GET ${index}/_search
{
  "query": {"match_phrase": {"${field}": "${value}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Exact value of a field (term)",
        r##"GET ${index}/_search
{
  "query": {"term": {"${field}": "${value}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Any of several values (terms)",
        r##"GET ${index}/_search
{
  "query": {"terms": {"${field}": ["${a}", "${b}"]}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Search several fields at once",
        r##"GET ${index}/_search
{
  "query": {"multi_match": {"query": "${value}", "fields": ["${field}", "${other_field}"]}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Prefix of a value",
        r##"GET ${index}/_search
{
  "query": {"prefix": {"${field}": "${value}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Wildcard",
        r##"GET ${index}/_search
{
  "query": {"wildcard": {"${field}": "${value}*"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Documents that have a field",
        r##"GET ${index}/_search
{
  "query": {"exists": {"field": "${field}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Documents that miss a field",
        r##"GET ${index}/_search
{
  "query": {"bool": {"must_not": {"exists": {"field": "${field}"}}}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Combine conditions (bool)",
        r##"GET ${index}/_search
{
  "query": {
    "bool": {
      "must": [{"match": {"${field}": "${value}"}}],
      "filter": [{"range": {"@timestamp": {"gte": "now-24h"}}}],
      "must_not": [{"term": {"${other_field}": "${other_value}"}}]
    }
  },
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Last 24 hours, newest first",
        r##"GET ${index}/_search
{
  "query": {"range": {"@timestamp": {"gte": "now-24h"}}},
  "sort": [{"@timestamp": "desc"}],
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "A number within a range",
        r##"GET ${index}/_search
{
  "query": {"range": {"${number_field}": {"gte": 10, "lte": 100}}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Query string (Lucene syntax)",
        r##"GET ${index}/_search
{
  "query": {"query_string": {"query": "${query}"}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Documents by id",
        r##"GET ${index}/_search
{
  "query": {"ids": {"values": ["${id}"]}}
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Only some fields (_source)",
        r##"GET ${index}/_search
{
  "_source": ["${field}", "${other_field}"],
  "query": {"match_all": {}},
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Sort and page (from / size)",
        r##"GET ${index}/_search
{
  "query": {"match_all": {}},
  "sort": [{"${field}": "desc"}],
  "from": 20,
  "size": 20
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Highlight the matches",
        r##"GET ${index}/_search
{
  "query": {"match": {"${field}": "${value}"}},
  "highlight": {"fields": {"${field}": {}}}
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Count the matches of a query",
        r##"POST ${index}/_count
{
  "query": {"match": {"${field}": "${value}"}}
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Check a query without running it (validate)",
        r##"GET ${index}/_validate/query?explain=true
{
  "query": {"match": {"${field}": "${value}"}}
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Where the time goes (profile)",
        r##"GET ${index}/_search
{
  "profile": true,
  "query": {"match": {"${field}": "${value}"}}
}"##,
    ),
    CheatEntry::new(
        "Search",
        "Why a document matches (explain)",
        r##"GET ${index}/_explain/${id}
{
  "query": {"match": {"${field}": "${value}"}}
}"##,
    ),
    // ── Aggregations ──
    CheatEntry::new(
        "Aggregations",
        "Top values of a field (terms)",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"top": {"terms": {"field": "${field}", "size": 10}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Documents per day (date_histogram)",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"per_day": {"date_histogram": {"field": "@timestamp", "calendar_interval": "day"}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Min, max, average and sum of a number (stats)",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"stats": {"stats": {"field": "${number_field}"}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Percentiles of a number",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"p": {"percentiles": {"field": "${number_field}", "percents": [50, 90, 99]}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Number of distinct values (cardinality)",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"unique": {"cardinality": {"field": "${field}"}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Average per group",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"by_group": {"terms": {"field": "${field}"}, "aggs": {"avg_value": {"avg": {"field": "${number_field}"}}}}}
}"##,
    ),
    CheatEntry::new(
        "Aggregations",
        "Latest document per group (top_hits)",
        r##"GET ${index}/_search
{
  "size": 0,
  "aggs": {"by_group": {"terms": {"field": "${field}"}, "aggs": {"latest": {"top_hits": {"size": 1, "sort": [{"@timestamp": "desc"}]}}}}}
}"##,
    ),
    // ── Write ──
    CheatEntry::new(
        "Write",
        "Index a document (id generated)",
        r##"POST ${index}/_doc
{
  "${field}": "${value}"
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Index a document with an id",
        r##"PUT ${index}/_doc/${id}
{
  "${field}": "${value}"
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Get a document by id",
        r##"GET ${index}/_doc/${id}"##,
    ),
    CheatEntry::new(
        "Write",
        "Update fields of a document",
        r##"POST ${index}/_update/${id}
{
  "doc": {"${field}": "${value}"}
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Update with a script",
        r##"POST ${index}/_update/${id}
{
  "script": {"source": "ctx._source.${field} = params.v", "params": {"v": "${value}"}}
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Update every document that matches",
        r##"POST ${index}/_update_by_query
{
  "query": {"term": {"${field}": "${value}"}},
  "script": {"source": "ctx._source.${other_field} = params.v", "params": {"v": "${other_value}"}}
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete a document",
        r##"DELETE ${index}/_doc/${id}"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete every document that matches",
        r##"POST ${index}/_delete_by_query
{
  "query": {"term": {"${field}": "${value}"}}
}"##,
    ),
    CheatEntry::new(
        "Write",
        "Copy an index into another (reindex)",
        r##"POST _reindex
{
  "source": {"index": "${index}"},
  "dest": {"index": "${new_index}"}
}"##,
    ),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "Create an index with a mapping",
        r##"PUT ${new_index}
{
  "settings": {"number_of_shards": 1, "number_of_replicas": 1},
  "mappings": {
    "properties": {
      "name": {"type": "text"},
      "status": {"type": "keyword"},
      "created_at": {"type": "date"}
    }
  }
}"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a field to a mapping",
        r##"PUT ${index}/_mapping
{
  "properties": {"${new_field}": {"type": "keyword"}}
}"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add an alias",
        r##"POST _aliases
{
  "actions": [{"add": {"index": "${index}", "alias": "${alias}"}}]
}"##,
    ),
    CheatEntry::new(
        "Schema",
        "Move an alias to another index",
        r##"POST _aliases
{
  "actions": [
    {"remove": {"index": "${index}", "alias": "${alias}"}},
    {"add": {"index": "${new_index}", "alias": "${alias}"}}
  ]
}"##,
    ),
    CheatEntry::new("Schema", "Close an index", r##"POST ${index}/_close"##),
    CheatEntry::new("Schema", "Open an index", r##"POST ${index}/_open"##),
    CheatEntry::new("Schema", "Delete an index", r##"DELETE ${index}"##),
    // ── Admin ──
    CheatEntry::new(
        "Admin",
        "Make recent writes searchable (refresh)",
        r##"POST ${index}/_refresh"##,
    ),
    CheatEntry::new("Admin", "Flush to disk", r##"POST ${index}/_flush"##),
    CheatEntry::new(
        "Admin",
        "Merge segments (force merge)",
        r##"POST ${index}/_forcemerge?max_num_segments=1"##,
    ),
    CheatEntry::new(
        "Admin",
        "Clear the caches of an index",
        r##"POST ${index}/_cache/clear"##,
    ),
    CheatEntry::new(
        "Admin",
        "Change the number of replicas",
        r##"PUT ${index}/_settings
{
  "index": {"number_of_replicas": 1}
}"##,
    ),
    CheatEntry::new(
        "Admin",
        "Retry shards that failed to allocate",
        r##"POST _cluster/reroute?retry_failed=true"##,
    ),
    CheatEntry::new(
        "Admin",
        "Cancel a task",
        r##"POST _tasks/${task_id}/_cancel"##,
    ),
    CheatEntry::new("Admin", "Snapshot repositories", r##"GET _snapshot"##),
];
