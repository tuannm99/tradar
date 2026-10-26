//! The ClickHouse cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Explore",
        "Tables with size and rows",
        r#"SELECT database, name, engine, total_rows, formatReadableSize(total_bytes) AS size
FROM system.tables
WHERE database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema')
ORDER BY total_bytes DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r#"SELECT name, type, default_kind, comment
FROM system.columns
WHERE table = '${table}'
ORDER BY position;"#,
    ),
    CheatEntry::new(
        "Explore",
        "CREATE TABLE of a table",
        "SHOW CREATE TABLE ${table};",
    ),
    CheatEntry::new("Explore", "Databases", "SHOW DATABASES;"),
    CheatEntry::new("Explore", "Server version", "SELECT version();"),
    // ── Disk ────────────────────────────────────────────────────────────
    CheatEntry::new(
        "Disk",
        "Disk space",
        r#"SELECT name, formatReadableSize(free_space) AS free, formatReadableSize(total_space) AS total
FROM system.disks;"#,
    ),
    CheatEntry::new(
        "Disk",
        "Size on disk per table (active parts)",
        r#"SELECT database, table,
       formatReadableSize(sum(bytes_on_disk)) AS on_disk,
       sum(rows) AS rows,
       count() AS parts
FROM system.parts
WHERE active
GROUP BY database, table
ORDER BY sum(bytes_on_disk) DESC
LIMIT 30;"#,
    ),
    CheatEntry::new(
        "Disk",
        "Compression per column of a table",
        r#"SELECT name,
       formatReadableSize(data_compressed_bytes) AS compressed,
       formatReadableSize(data_uncompressed_bytes) AS uncompressed
FROM system.columns
WHERE table = '${table}'
ORDER BY data_compressed_bytes DESC;"#,
    ),
    // ── Activity ────────────────────────────────────────────────────────
    CheatEntry::new(
        "Activity",
        "Running queries",
        r#"SELECT query_id, user, elapsed, formatReadableSize(memory_usage) AS memory, substring(query, 1, 100) AS query
FROM system.processes;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Slowest finished queries",
        r#"SELECT event_time, query_duration_ms, read_rows, formatReadableSize(memory_usage) AS memory, substring(query, 1, 100) AS query
FROM system.query_log
WHERE type = 'QueryFinish'
ORDER BY query_duration_ms DESC
LIMIT 20;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Recent failed queries",
        r#"SELECT event_time, exception_code, substring(exception, 1, 100) AS exception, substring(query, 1, 100) AS query
FROM system.query_log
WHERE type IN ('ExceptionBeforeStart', 'ExceptionWhileProcessing')
ORDER BY event_time DESC
LIMIT 20;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Merges in progress",
        r#"SELECT database, table, elapsed, progress
FROM system.merges;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Mutations not done",
        r#"SELECT database, table, mutation_id, command, is_done
FROM system.mutations
WHERE NOT is_done;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Errors since start",
        r#"SELECT name, code, value, last_error_time
FROM system.errors
ORDER BY value DESC
LIMIT 20;"#,
    ),
    // ── Settings ────────────────────────────────────────────────────────
    CheatEntry::new(
        "Settings",
        "Settings matching a name",
        r#"SELECT name, value, changed
FROM system.settings
WHERE name LIKE '%${pattern}%';"#,
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
        r#"SELECT toDate(${column}) AS day, count()
FROM ${table}
GROUP BY day
ORDER BY day DESC
LIMIT 30;"#,
    ),
    CheatEntry::new(
        "Query",
        "Explain a query",
        "EXPLAIN\nSELECT * FROM ${table} LIMIT 100;",
    ),
];
