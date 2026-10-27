//! The PostgreSQL cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new(
        "Explore",
        "Tables with size and approximate rows",
        r##"SELECT schemaname, relname AS table_name, n_live_tup AS approx_rows,
       pg_size_pretty(pg_total_relation_size(relid)) AS total_size
FROM pg_stat_user_tables
ORDER BY pg_total_relation_size(relid) DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r##"SELECT column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE table_name = '${table}'
ORDER BY ordinal_position;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Primary key columns of a table",
        r##"SELECT a.attname AS column_name
FROM pg_index i
JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
WHERE i.indrelid = '${table}'::regclass AND i.indisprimary;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a table",
        r##"SELECT indexname, indexdef
FROM pg_indexes
WHERE tablename = '${table}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        r##"SELECT conname AS name, pg_get_constraintdef(oid) AS definition
FROM pg_constraint
WHERE contype = 'f' AND conrelid = '${table}'::regclass;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Tables that reference a table",
        r##"SELECT conrelid::regclass AS referencing_table, conname AS name
FROM pg_constraint
WHERE contype = 'f' AND confrelid = '${table}'::regclass;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Views",
        r##"SELECT schemaname, viewname
FROM pg_views
WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
ORDER BY 1, 2;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Functions of a schema",
        r##"SELECT routine_name, routine_type, data_type AS returns
FROM information_schema.routines
WHERE routine_schema = '${schema}'
ORDER BY routine_name;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Sequences",
        r##"SELECT sequence_schema, sequence_name, data_type
FROM information_schema.sequences
ORDER BY 1, 2;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Schemas",
        r##"SELECT schema_name
FROM information_schema.schemata
ORDER BY schema_name;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Roles",
        r##"SELECT rolname, rolsuper, rolcreatedb, rolcanlogin
FROM pg_roles
ORDER BY rolname;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Privileges on a table",
        r##"SELECT grantee, privilege_type
FROM information_schema.role_table_grants
WHERE table_name = '${table}'
ORDER BY grantee, privilege_type;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Extensions installed",
        r##"SELECT extname, extversion
FROM pg_extension
ORDER BY extname;"##,
    ),
    CheatEntry::new("Explore", "Server version", r##"SELECT version();"##),
    // ── Activity ──
    CheatEntry::new(
        "Activity",
        "Running queries",
        r##"SELECT pid, usename, state, now() - query_start AS running_for, left(query, 100) AS query
FROM pg_stat_activity
WHERE state <> 'idle' AND pid <> pg_backend_pid()
ORDER BY query_start;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Connections per database and state",
        r##"SELECT datname, state, count(*) AS connections
FROM pg_stat_activity
GROUP BY datname, state
ORDER BY connections DESC;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Idle in transaction",
        r##"SELECT pid, usename, now() - state_change AS idle_for, left(query, 100) AS last_query
FROM pg_stat_activity
WHERE state = 'idle in transaction'
ORDER BY state_change;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Who blocks whom",
        r##"SELECT blocked.pid AS blocked_pid,
       blocking.pid AS blocking_pid,
       left(blocked.query, 80) AS blocked_query,
       left(blocking.query, 80) AS blocking_query
FROM pg_stat_activity blocked
JOIN pg_stat_activity blocking ON blocking.pid = ANY (pg_blocking_pids(blocked.pid));"##,
    ),
    CheatEntry::new(
        "Activity",
        "Locks held on tables",
        r##"SELECT relation::regclass AS table_name, mode, granted, pid
FROM pg_locks
WHERE relation IS NOT NULL
ORDER BY relation;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Transactions open for a long time",
        r##"SELECT pid, usename, xact_start, now() - xact_start AS open_for, state, left(query, 100) AS query
FROM pg_stat_activity
WHERE xact_start IS NOT NULL
ORDER BY xact_start
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Replication status",
        r##"SELECT client_addr, state, sent_lsn, replay_lsn, replay_lag
FROM pg_stat_replication;"##,
    ),
    // ── Size ──
    CheatEntry::new(
        "Size",
        "Databases by size",
        r##"SELECT datname, pg_size_pretty(pg_database_size(datname)) AS size
FROM pg_database
ORDER BY pg_database_size(datname) DESC;"##,
    ),
    CheatEntry::new(
        "Size",
        "Size of one table, its indexes and the total",
        r##"SELECT pg_size_pretty(pg_table_size('${table}')) AS table_size,
       pg_size_pretty(pg_indexes_size('${table}')) AS index_size,
       pg_size_pretty(pg_total_relation_size('${table}')) AS total;"##,
    ),
    CheatEntry::new(
        "Size",
        "Largest indexes",
        r##"SELECT schemaname, relname AS table_name, indexrelname AS index_name,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
FROM pg_stat_user_indexes
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 20;"##,
    ),
    // ── Health ──
    CheatEntry::new(
        "Health",
        "Indexes never used",
        r##"SELECT schemaname, relname AS table_name, indexrelname AS index_name, idx_scan,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
FROM pg_stat_user_indexes
WHERE idx_scan = 0
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Health",
        "Duplicate indexes",
        r##"SELECT indrelid::regclass AS table_name, array_agg(indexrelid::regclass) AS duplicate_indexes
FROM pg_index
GROUP BY indrelid, indkey
HAVING count(*) > 1;"##,
    ),
    CheatEntry::new(
        "Health",
        "Tables with the most dead rows",
        r##"SELECT relname AS table_name, n_dead_tup, n_live_tup, last_autovacuum
FROM pg_stat_user_tables
ORDER BY n_dead_tup DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Health",
        "Tables read by sequential scan",
        r##"SELECT relname AS table_name, seq_scan, seq_tup_read, idx_scan
FROM pg_stat_user_tables
WHERE seq_scan > 0
ORDER BY seq_tup_read DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Health",
        "Cache hit ratio",
        r##"SELECT sum(blks_hit)::float / nullif(sum(blks_hit) + sum(blks_read), 0) AS hit_ratio
FROM pg_stat_database;"##,
    ),
    CheatEntry::new(
        "Health",
        "Slowest statements (needs pg_stat_statements)",
        r##"SELECT round(total_exec_time::numeric, 1) AS total_ms, calls,
       round(mean_exec_time::numeric, 2) AS mean_ms, left(query, 100) AS query
FROM pg_stat_statements
ORDER BY total_exec_time DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Health",
        "Settings matching a name",
        r##"SELECT name, setting, unit, short_desc
FROM pg_settings
WHERE name ILIKE '%${pattern}%'
ORDER BY name;"##,
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
        r##"SELECT count(*) FROM ${table};"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r##"SELECT date_trunc('day', ${date_column}) AS day, count(*)
FROM ${table}
GROUP BY 1
ORDER BY 1 DESC
LIMIT 30;"##,
    ),
    CheatEntry::new(
        "Query",
        "Duplicate values of a column",
        r##"SELECT ${column}, count(*)
FROM ${table}
GROUP BY ${column}
HAVING count(*) > 1
ORDER BY 2 DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Query",
        "Most frequent values of a column",
        r##"SELECT ${column}, count(*) AS n
FROM ${table}
GROUP BY ${column}
ORDER BY n DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows with a NULL column",
        r##"SELECT *
FROM ${table}
WHERE ${column} IS NULL
LIMIT 100;"##,
    ),
    CheatEntry::new(
        "Query",
        "Explain a query (with timing)",
        r##"EXPLAIN (ANALYZE, BUFFERS)
SELECT * FROM ${table} LIMIT 100;"##,
    ),
    // ── SQL syntax ──
    CheatEntry::new(
        "SQL syntax",
        "CTE (WITH)",
        r##"WITH recent AS (
  SELECT * FROM ${table} ORDER BY ${column} DESC LIMIT 100
)
SELECT count(*) FROM recent;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Top N per group (window function)",
        r##"SELECT *
FROM (
  SELECT t.*, row_number() OVER (PARTITION BY ${group_column} ORDER BY ${column} DESC) AS rn
  FROM ${table} t
) ranked
WHERE rn <= 3;"##,
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
        "Rank rows (window function)",
        r##"SELECT *, rank() OVER (ORDER BY ${column} DESC) AS rank
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Rows with no match in another table (LEFT JOIN)",
        r##"SELECT a.*
FROM ${table} a
LEFT JOIN ${other_table} b ON b.${join_column} = a.${join_column}
WHERE b.${join_column} IS NULL;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Rows that have a match (EXISTS)",
        r##"SELECT *
FROM ${table} a
WHERE EXISTS (SELECT 1 FROM ${other_table} b WHERE b.${join_column} = a.${join_column});"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "CASE expression",
        r##"SELECT ${column},
       CASE WHEN ${column} IS NULL THEN 'missing' ELSE 'present' END AS state
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Count by condition (FILTER)",
        r##"SELECT count(*) FILTER (WHERE ${column} = '${a}') AS a,
       count(*) FILTER (WHERE ${column} = '${b}') AS b
FROM ${table};"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Keyset pagination",
        r##"SELECT *
FROM ${table}
WHERE ${column} > '${last_value}'
ORDER BY ${column}
LIMIT 100;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Date arithmetic",
        r##"SELECT now() - interval '7 days' AS a_week_ago,
       date_trunc('month', now()) AS month_start,
       now()::date AS today;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "JSON field (->>)",
        r##"SELECT ${json_column}->>'${json_key}' AS value
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Join values into a string (string_agg)",
        r##"SELECT ${group_column}, string_agg(${column}::text, ', ') AS joined
FROM ${table}
GROUP BY ${group_column}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Series of numbers (generate_series)",
        r##"SELECT generate_series(1, 10) AS n;"##,
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
        "INSERT and return the row",
        r##"INSERT INTO ${table} (${column})
VALUES ('${value}')
RETURNING *;"##,
    ),
    CheatEntry::new(
        "Write",
        "Upsert (ON CONFLICT)",
        r##"INSERT INTO ${table} (${key_column}, ${column})
VALUES ('${key}', '${value}')
ON CONFLICT (${key_column}) DO UPDATE SET ${column} = EXCLUDED.${column};"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT rows from a query",
        r##"INSERT INTO ${table} (${column})
SELECT ${column} FROM ${other_table} WHERE ${filter};"##,
    ),
    CheatEntry::new(
        "Write",
        "UPDATE rows",
        r##"UPDATE ${table}
SET ${column} = '${value}'
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "UPDATE from another table",
        r##"UPDATE ${table} t
SET ${column} = o.${column}
FROM ${other_table} o
WHERE o.${join_column} = t.${join_column};"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE rows",
        r##"DELETE FROM ${table}
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE and return the rows",
        r##"DELETE FROM ${table}
WHERE ${key_column} = '${key}'
RETURNING *;"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete duplicates, keeping one",
        r##"DELETE FROM ${table} a
USING ${table} b
WHERE a.ctid < b.ctid AND a.${column} = b.${column};"##,
    ),
    CheatEntry::new(
        "Write",
        "Empty a table (TRUNCATE)",
        r##"TRUNCATE TABLE ${table};"##,
    ),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "CREATE TABLE",
        r##"CREATE TABLE ${new_table} (
  id bigserial PRIMARY KEY,
  name text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a column",
        r##"ALTER TABLE ${table} ADD COLUMN ${new_column} text;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a column",
        r##"ALTER TABLE ${table} RENAME COLUMN ${column} TO ${new_column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Change the type of a column",
        r##"ALTER TABLE ${table}
ALTER COLUMN ${number_column} TYPE bigint USING ${number_column}::bigint;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Make a column NOT NULL",
        r##"ALTER TABLE ${table} ALTER COLUMN ${column} SET NOT NULL;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a column",
        r##"ALTER TABLE ${table} DROP COLUMN ${column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a table",
        r##"ALTER TABLE ${table} RENAME TO ${new_table};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a foreign key",
        r##"ALTER TABLE ${table}
ADD CONSTRAINT ${constraint} FOREIGN KEY (${fk_column}) REFERENCES ${other_table} (${join_column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE INDEX (without locking writes)",
        r##"CREATE INDEX CONCURRENTLY ${index_name} ON ${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE UNIQUE INDEX",
        r##"CREATE UNIQUE INDEX CONCURRENTLY ${index_name} ON ${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "DROP INDEX",
        r##"DROP INDEX CONCURRENTLY ${index_name};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE VIEW",
        r##"CREATE VIEW ${view} AS
SELECT * FROM ${table};"##,
    ),
    CheatEntry::new("Schema", "DROP TABLE", r##"DROP TABLE ${table};"##),
    // ── Admin ──
    CheatEntry::new(
        "Admin",
        "Cancel a running query",
        r##"SELECT pg_cancel_backend(${pid});"##,
    ),
    CheatEntry::new(
        "Admin",
        "Terminate a connection",
        r##"SELECT pg_terminate_backend(${pid});"##,
    ),
    CheatEntry::new(
        "Admin",
        "VACUUM ANALYZE a table",
        r##"VACUUM (ANALYZE, VERBOSE) ${table};"##,
    ),
    CheatEntry::new("Admin", "ANALYZE a table", r##"ANALYZE ${table};"##),
    CheatEntry::new(
        "Admin",
        "Rebuild the indexes of a table",
        r##"REINDEX TABLE CONCURRENTLY ${table};"##,
    ),
    CheatEntry::new(
        "Admin",
        "CREATE a login role",
        r##"CREATE ROLE ${role} LOGIN PASSWORD '${password}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "GRANT SELECT on every table of public",
        r##"GRANT SELECT ON ALL TABLES IN SCHEMA public TO ${role};"##,
    ),
    CheatEntry::new(
        "Admin",
        "REVOKE all privileges on a table",
        r##"REVOKE ALL ON ${table} FROM ${role};"##,
    ),
    CheatEntry::new(
        "Admin",
        "Reload the configuration",
        r##"SELECT pg_reload_conf();"##,
    ),
    CheatEntry::new(
        "Admin",
        "Statement timeout for this session",
        r##"SET statement_timeout = '30s';"##,
    ),
    // ── Transaction ──
    CheatEntry::new("Transaction", "BEGIN", r##"BEGIN;"##),
    CheatEntry::new("Transaction", "COMMIT", r##"COMMIT;"##),
    CheatEntry::new("Transaction", "ROLLBACK", r##"ROLLBACK;"##),
    CheatEntry::new("Transaction", "SAVEPOINT", r##"SAVEPOINT ${name};"##),
    CheatEntry::new(
        "Transaction",
        "ROLLBACK TO a savepoint",
        r##"ROLLBACK TO SAVEPOINT ${name};"##,
    ),
];
