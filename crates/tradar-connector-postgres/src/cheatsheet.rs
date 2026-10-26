//! The PostgreSQL cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Explore",
        "Tables with size and approximate rows",
        r#"SELECT schemaname, relname AS table_name, n_live_tup AS approx_rows,
       pg_size_pretty(pg_total_relation_size(relid)) AS total_size
FROM pg_stat_user_tables
ORDER BY pg_total_relation_size(relid) DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r#"SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE table_name = '${table}'
ORDER BY ordinal_position;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a table",
        r#"SELECT indexname, indexdef
FROM pg_indexes
WHERE tablename = '${table}';"#,
    ),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        r#"SELECT conname AS name, pg_get_constraintdef(oid) AS definition
FROM pg_constraint
WHERE contype = 'f' AND conrelid = '${table}'::regclass;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Schemas",
        r#"SELECT schema_name
FROM information_schema.schemata
ORDER BY schema_name;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Extensions installed",
        r#"SELECT extname, extversion
FROM pg_extension
ORDER BY extname;"#,
    ),
    CheatEntry::new("Explore", "Server version", "SELECT version();"),
    // ── Activity ────────────────────────────────────────────────────────
    CheatEntry::new(
        "Activity",
        "Running queries",
        r#"SELECT pid, usename, state, now() - query_start AS running_for, left(query, 100) AS query
FROM pg_stat_activity
WHERE state <> 'idle' AND pid <> pg_backend_pid()
ORDER BY query_start;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Connections per database and state",
        r#"SELECT datname, state, count(*) AS connections
FROM pg_stat_activity
GROUP BY datname, state
ORDER BY connections DESC;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Who blocks whom",
        r#"SELECT blocked.pid AS blocked_pid,
       blocking.pid AS blocking_pid,
       left(blocked.query, 80) AS blocked_query,
       left(blocking.query, 80) AS blocking_query
FROM pg_stat_activity blocked
JOIN pg_stat_activity blocking ON blocking.pid = ANY (pg_blocking_pids(blocked.pid));"#,
    ),
    CheatEntry::new(
        "Activity",
        "Transactions open for a long time",
        r#"SELECT pid, usename, xact_start, now() - xact_start AS open_for, state, left(query, 100) AS query
FROM pg_stat_activity
WHERE xact_start IS NOT NULL
ORDER BY xact_start
LIMIT 20;"#,
    ),
    // ── Size ────────────────────────────────────────────────────────────
    CheatEntry::new(
        "Size",
        "Databases by size",
        r#"SELECT datname, pg_size_pretty(pg_database_size(datname)) AS size
FROM pg_database
ORDER BY pg_database_size(datname) DESC;"#,
    ),
    CheatEntry::new(
        "Size",
        "Largest indexes",
        r#"SELECT schemaname, relname AS table_name, indexrelname AS index_name,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
FROM pg_stat_user_indexes
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 20;"#,
    ),
    // ── Health ──────────────────────────────────────────────────────────
    CheatEntry::new(
        "Health",
        "Indexes never used",
        r#"SELECT schemaname, relname AS table_name, indexrelname AS index_name, idx_scan,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
FROM pg_stat_user_indexes
WHERE idx_scan = 0
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Health",
        "Tables with the most dead rows",
        r#"SELECT relname AS table_name, n_dead_tup, n_live_tup, last_autovacuum
FROM pg_stat_user_tables
ORDER BY n_dead_tup DESC
LIMIT 20;"#,
    ),
    CheatEntry::new(
        "Health",
        "Cache hit ratio",
        r#"SELECT sum(blks_hit)::float / nullif(sum(blks_hit) + sum(blks_read), 0) AS hit_ratio
FROM pg_stat_database;"#,
    ),
    CheatEntry::new(
        "Health",
        "Settings matching a name",
        r#"SELECT name, setting, unit, short_desc
FROM pg_settings
WHERE name ILIKE '%${pattern}%'
ORDER BY name;"#,
    ),
    // ── Query ───────────────────────────────────────────────────────────
    CheatEntry::new(
        "Query",
        "First rows of a table",
        "SELECT *\nFROM ${table}\nLIMIT 100;",
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r#"SELECT date_trunc('day', ${column}) AS day, count(*)
FROM ${table}
GROUP BY 1
ORDER BY 1 DESC
LIMIT 30;"#,
    ),
    CheatEntry::new(
        "Query",
        "Duplicate values of a column",
        r#"SELECT ${column}, count(*)
FROM ${table}
GROUP BY ${column}
HAVING count(*) > 1
ORDER BY 2 DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Query",
        "Explain a query (with timing)",
        "EXPLAIN (ANALYZE, BUFFERS)\nSELECT * FROM ${table} LIMIT 100;",
    ),
];
