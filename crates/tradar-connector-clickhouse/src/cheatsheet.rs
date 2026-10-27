//! The ClickHouse cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new(
        "Explore",
        "Tables with size and rows",
        r##"SELECT database, name, engine, total_rows, formatReadableSize(total_bytes) AS size
FROM system.tables
WHERE database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema')
ORDER BY total_bytes DESC
LIMIT 50;"##,
    ),
    CheatEntry::new("Explore", "Databases", r##"SHOW DATABASES;"##),
    CheatEntry::new(
        "Explore",
        "Tables of a database",
        r##"SHOW TABLES FROM ${database};"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r##"SELECT name, type, default_kind, default_expression, comment
FROM system.columns
WHERE table = '${table}'
ORDER BY position;"##,
    ),
    CheatEntry::new(
        "Explore",
        "CREATE TABLE of a table",
        r##"SHOW CREATE TABLE ${table};"##,
    ),
    CheatEntry::new(
        "Explore",
        "Describe a table",
        r##"DESCRIBE TABLE ${table};"##,
    ),
    CheatEntry::new(
        "Explore",
        "Sorting key, partition key and engine of a table",
        r##"SELECT name, engine, partition_key, sorting_key, primary_key, sampling_key
FROM system.tables
WHERE name = '${table}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Materialized views and their targets",
        r##"SELECT database, name, engine, create_table_query
FROM system.tables
WHERE engine = 'MaterializedView'
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Dictionaries",
        r##"SELECT database, name, status, element_count FROM system.dictionaries;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Functions matching a name",
        r##"SELECT name, is_aggregate
FROM system.functions
WHERE name ILIKE '%${pattern}%'
ORDER BY name;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Server version and uptime",
        r##"SELECT version() AS version, uptime() AS uptime_seconds;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Current user and database",
        r##"SELECT currentUser() AS user, currentDatabase() AS db;"##,
    ),
    // ── Disk ──
    CheatEntry::new(
        "Disk",
        "Disk space",
        r##"SELECT name, formatReadableSize(free_space) AS free, formatReadableSize(total_space) AS total
FROM system.disks;"##,
    ),
    CheatEntry::new(
        "Disk",
        "Size on disk per table (active parts)",
        r##"SELECT database, table,
       formatReadableSize(sum(bytes_on_disk)) AS on_disk,
       sum(rows) AS rows,
       count() AS parts
FROM system.parts
WHERE active
GROUP BY database, table
ORDER BY sum(bytes_on_disk) DESC
LIMIT 30;"##,
    ),
    CheatEntry::new(
        "Disk",
        "Size per partition of a table",
        r##"SELECT partition, formatReadableSize(sum(bytes_on_disk)) AS on_disk, sum(rows) AS rows, count() AS parts
FROM system.parts
WHERE active AND table = '${table}'
GROUP BY partition
ORDER BY partition DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Disk",
        "Compression per column of a table",
        r##"SELECT name,
       formatReadableSize(data_compressed_bytes) AS compressed,
       formatReadableSize(data_uncompressed_bytes) AS uncompressed
FROM system.columns
WHERE table = '${table}'
ORDER BY data_compressed_bytes DESC;"##,
    ),
    CheatEntry::new(
        "Disk",
        "Tables with too many parts",
        r##"SELECT database, table, count() AS parts
FROM system.parts
WHERE active
GROUP BY database, table
ORDER BY parts DESC
LIMIT 20;"##,
    ),
    // ── Activity ──
    CheatEntry::new(
        "Activity",
        "Running queries",
        r##"SELECT query_id, user, elapsed, formatReadableSize(memory_usage) AS memory, substring(query, 1, 100) AS query
FROM system.processes;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Slowest finished queries",
        r##"SELECT event_time, query_duration_ms, read_rows, formatReadableSize(memory_usage) AS memory, substring(query, 1, 100) AS query
FROM system.query_log
WHERE type = 'QueryFinish'
ORDER BY query_duration_ms DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Queries that read the most",
        r##"SELECT event_time, read_rows, formatReadableSize(read_bytes) AS read, substring(query, 1, 100) AS query
FROM system.query_log
WHERE type = 'QueryFinish'
ORDER BY read_bytes DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Recent failed queries",
        r##"SELECT event_time, exception_code, substring(exception, 1, 100) AS exception, substring(query, 1, 100) AS query
FROM system.query_log
WHERE type IN ('ExceptionBeforeStart', 'ExceptionWhileProcessing')
ORDER BY event_time DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Queries per user, last hour",
        r##"SELECT user, count() AS queries, round(avg(query_duration_ms)) AS avg_ms
FROM system.query_log
WHERE type = 'QueryFinish' AND event_time > now() - INTERVAL 1 HOUR
GROUP BY user
ORDER BY queries DESC;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Merges in progress",
        r##"SELECT database, table, elapsed, progress
FROM system.merges;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Mutations not done",
        r##"SELECT database, table, mutation_id, command, is_done, latest_fail_reason
FROM system.mutations
WHERE NOT is_done;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Replication queue",
        r##"SELECT database, table, type, create_time, last_exception
FROM system.replication_queue
ORDER BY create_time
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Errors since start",
        r##"SELECT name, code, value, last_error_time
FROM system.errors
ORDER BY value DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Current metrics",
        r##"SELECT metric, value FROM system.metrics ORDER BY metric;"##,
    ),
    // ── Settings ──
    CheatEntry::new(
        "Settings",
        "Settings matching a name",
        r##"SELECT name, value, changed
FROM system.settings
WHERE name LIKE '%${pattern}%';"##,
    ),
    CheatEntry::new(
        "Settings",
        "Settings changed from the default",
        r##"SELECT name, value FROM system.settings WHERE changed;"##,
    ),
    CheatEntry::new(
        "Settings",
        "MergeTree settings matching a name",
        r##"SELECT name, value
FROM system.merge_tree_settings
WHERE name LIKE '%${pattern}%';"##,
    ),
    // ── Query ──
    CheatEntry::new(
        "Query",
        "First rows of a table",
        r##"SELECT *
FROM ${table}
LIMIT 100;"##,
    ),
    CheatEntry::new(
        "Query",
        "Row count of a table",
        r##"SELECT count() FROM ${table};"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r##"SELECT toDate(${date_column}) AS day, count()
FROM ${table}
GROUP BY day
ORDER BY day DESC
LIMIT 30;"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows per hour, last day",
        r##"SELECT toStartOfHour(${date_column}) AS hour, count()
FROM ${table}
WHERE ${date_column} > now() - INTERVAL 1 DAY
GROUP BY hour
ORDER BY hour;"##,
    ),
    CheatEntry::new(
        "Query",
        "Most frequent values of a column",
        r##"SELECT ${column}, count() AS n
FROM ${table}
GROUP BY ${column}
ORDER BY n DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Duplicate values of a column",
        r##"SELECT ${column}, count() AS n
FROM ${table}
GROUP BY ${column}
HAVING n > 1
ORDER BY n DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Query",
        "Explain a query",
        r##"EXPLAIN
SELECT * FROM ${table} LIMIT 100;"##,
    ),
    CheatEntry::new(
        "Query",
        "Explain with the index used",
        r##"EXPLAIN indexes = 1
SELECT * FROM ${table} WHERE ${column} = '${value}';"##,
    ),
    CheatEntry::new(
        "Query",
        "Explain the pipeline",
        r##"EXPLAIN PIPELINE
SELECT * FROM ${table} LIMIT 100;"##,
    ),
    // ── SQL syntax ──
    CheatEntry::new(
        "SQL syntax",
        "Percentiles (quantiles)",
        r##"SELECT quantiles(0.5, 0.9, 0.99)(${value_column}) AS p50_p90_p99
FROM ${table};"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Unique count (uniq, approximate and fast)",
        r##"SELECT uniq(${column}) AS approx_unique, uniqExact(${column}) AS exact_unique
FROM ${table};"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Collect values into an array (groupArray)",
        r##"SELECT ${group_column}, groupArray(${column}) AS values
FROM ${table}
GROUP BY ${group_column}
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "One row per array element (ARRAY JOIN)",
        r##"SELECT ${group_column}, element
FROM ${table}
ARRAY JOIN ${array_column} AS element
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "N rows per group (LIMIT BY)",
        r##"SELECT *
FROM ${table}
ORDER BY ${column} DESC
LIMIT 3 BY ${group_column};"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Latest row per key (argMax)",
        r##"SELECT ${group_column}, argMax(${value_column}, ${column}) AS latest
FROM ${table}
GROUP BY ${group_column}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Latest version of each row (FINAL)",
        r##"SELECT *
FROM ${table} FINAL
LIMIT 100;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Sample part of a table (needs SAMPLE BY in the table)",
        r##"SELECT count() * 10 AS estimated_rows
FROM ${table} SAMPLE 0.1;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Fixed time buckets",
        r##"SELECT toStartOfInterval(${date_column}, INTERVAL 5 MINUTE) AS bucket, count()
FROM ${table}
GROUP BY bucket
ORDER BY bucket DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Running total (window function)",
        r##"SELECT ${column}, sum(${value_column}) OVER (ORDER BY ${column}) AS running_total
FROM ${table}
ORDER BY ${column}
LIMIT 100;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "CTE (WITH)",
        r##"WITH recent AS (
  SELECT * FROM ${table} ORDER BY ${column} DESC LIMIT 100
)
SELECT count() FROM recent;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Conditional counts (countIf)",
        r##"SELECT countIf(${column} = '${a}') AS a, countIf(${column} = '${b}') AS b
FROM ${table};"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Date arithmetic",
        r##"SELECT now() - INTERVAL 7 DAY AS a_week_ago,
       toStartOfMonth(now()) AS month_start,
       today() AS today;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "JSON field (JSONExtractString)",
        r##"SELECT JSONExtractString(${json_column}, '${json_key}') AS value
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Readable sizes and numbers",
        r##"SELECT formatReadableSize(1234567890) AS size, formatReadableQuantity(1234567) AS quantity;"##,
    ),
    // ── Write ──
    CheatEntry::new(
        "Write",
        "INSERT a row",
        r##"INSERT INTO ${table} (${column})
VALUES ('${value}');"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT rows from a query",
        r##"INSERT INTO ${table} (${column})
SELECT ${column} FROM ${other_table} WHERE ${filter};"##,
    ),
    CheatEntry::new(
        "Write",
        "UPDATE rows (a mutation, runs in the background)",
        r##"ALTER TABLE ${table}
UPDATE ${column} = '${value}'
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE rows (a mutation, runs in the background)",
        r##"ALTER TABLE ${table}
DELETE WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE rows (lightweight)",
        r##"DELETE FROM ${table}
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "Collapse duplicates now (OPTIMIZE ... FINAL)",
        r##"OPTIMIZE TABLE ${table} FINAL;"##,
    ),
    CheatEntry::new(
        "Write",
        "Empty a table (TRUNCATE)",
        r##"TRUNCATE TABLE ${table};"##,
    ),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "CREATE DATABASE",
        r##"CREATE DATABASE IF NOT EXISTS ${new_database};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE TABLE (MergeTree)",
        r##"CREATE TABLE ${new_table} (
  id UInt64,
  name String,
  created_at DateTime DEFAULT now()
) ENGINE = MergeTree
PARTITION BY toYYYYMM(created_at)
ORDER BY (id, created_at);"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE TABLE (ReplacingMergeTree, keeps the latest version)",
        r##"CREATE TABLE ${new_table} (
  id UInt64,
  name String,
  version UInt64
) ENGINE = ReplacingMergeTree(version)
ORDER BY id;"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE TABLE as a copy of another",
        r##"CREATE TABLE ${new_table} AS ${table};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE MATERIALIZED VIEW",
        r##"CREATE MATERIALIZED VIEW ${new_view} ENGINE = SummingMergeTree ORDER BY day AS
SELECT toDate(${date_column}) AS day, count() AS n
FROM ${table}
GROUP BY day;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a column",
        r##"ALTER TABLE ${table} ADD COLUMN ${new_column} String DEFAULT '';"##,
    ),
    CheatEntry::new(
        "Schema",
        "Change the type of a column",
        r##"ALTER TABLE ${table} MODIFY COLUMN ${column} String;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a column",
        r##"ALTER TABLE ${table} RENAME COLUMN ${column} TO ${new_column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a column",
        r##"ALTER TABLE ${table} DROP COLUMN ${column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a data-skipping index",
        r##"ALTER TABLE ${table} ADD INDEX ${index_name} ${column} TYPE bloom_filter GRANULARITY 4;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Set a TTL (delete old rows automatically)",
        r##"ALTER TABLE ${table} MODIFY TTL ${date_column} + INTERVAL 90 DAY;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a partition",
        r##"ALTER TABLE ${table} DROP PARTITION '${partition}';"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a table",
        r##"RENAME TABLE ${table} TO ${new_table};"##,
    ),
    CheatEntry::new("Schema", "DROP TABLE", r##"DROP TABLE ${table};"##),
    // ── Admin ──
    CheatEntry::new(
        "Admin",
        "Kill a query",
        r##"KILL QUERY WHERE query_id = '${query_id}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "Kill a mutation",
        r##"KILL MUTATION WHERE database = '${database}' AND table = '${table}' AND mutation_id = '${mutation_id}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "Write the in-memory logs to disk (so query_log is current)",
        r##"SYSTEM FLUSH LOGS;"##,
    ),
    CheatEntry::new(
        "Admin",
        "Users",
        r##"SELECT name, storage FROM system.users;"##,
    ),
    CheatEntry::new("Admin", "Grants of a user", r##"SHOW GRANTS FOR ${user};"##),
    CheatEntry::new(
        "Admin",
        "CREATE a user",
        r##"CREATE USER ${user} IDENTIFIED BY '${password}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "GRANT SELECT on a database",
        r##"GRANT SELECT ON ${database}.* TO ${user};"##,
    ),
];
