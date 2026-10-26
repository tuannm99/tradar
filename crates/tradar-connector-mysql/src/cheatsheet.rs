//! The MySQL / MariaDB cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new("Explore", "Databases", "SHOW DATABASES;"),
    CheatEntry::new(
        "Explore",
        "Tables with size and approximate rows",
        r#"SELECT table_name, table_rows AS approx_rows,
       ROUND((data_length + index_length) / 1024 / 1024, 1) AS size_mb
FROM information_schema.tables
WHERE table_schema = DATABASE()
ORDER BY (data_length + index_length) DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r#"SELECT column_name, column_type, is_nullable, column_default, column_key
FROM information_schema.columns
WHERE table_schema = DATABASE() AND table_name = '${table}'
ORDER BY ordinal_position;"#,
    ),
    CheatEntry::new(
        "Explore",
        "CREATE TABLE of a table",
        "SHOW CREATE TABLE ${table};",
    ),
    CheatEntry::new("Explore", "Indexes of a table", "SHOW INDEX FROM ${table};"),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        r#"SELECT constraint_name, column_name, referenced_table_name, referenced_column_name
FROM information_schema.key_column_usage
WHERE table_schema = DATABASE() AND table_name = '${table}' AND referenced_table_name IS NOT NULL;"#,
    ),
    CheatEntry::new("Explore", "Server version", "SELECT VERSION();"),
    // ── Activity ────────────────────────────────────────────────────────
    CheatEntry::new("Activity", "Process list", "SHOW FULL PROCESSLIST;"),
    CheatEntry::new(
        "Activity",
        "Running queries, longest first",
        r#"SELECT id, user, db, time AS seconds, state, LEFT(info, 100) AS query
FROM information_schema.processlist
WHERE command <> 'Sleep'
ORDER BY time DESC;"#,
    ),
    CheatEntry::new(
        "Activity",
        "Connections per user",
        r#"SELECT user, COUNT(*) AS connections
FROM information_schema.processlist
GROUP BY user
ORDER BY connections DESC;"#,
    ),
    CheatEntry::new(
        "Activity",
        "InnoDB lock waits (MySQL 8+)",
        "SELECT * FROM performance_schema.data_lock_waits;",
    ),
    // ── Status ──────────────────────────────────────────────────────────
    CheatEntry::new(
        "Status",
        "Connection counters",
        "SHOW GLOBAL STATUS LIKE 'Threads%';",
    ),
    CheatEntry::new(
        "Status",
        "Variables matching a name",
        "SHOW VARIABLES LIKE '%${pattern}%';",
    ),
    CheatEntry::new(
        "Status",
        "Status counters matching a name",
        "SHOW GLOBAL STATUS LIKE '%${pattern}%';",
    ),
    CheatEntry::new("Status", "InnoDB status", "SHOW ENGINE INNODB STATUS;"),
    // ── Size ────────────────────────────────────────────────────────────
    CheatEntry::new(
        "Size",
        "Databases by size",
        r#"SELECT table_schema AS db,
       ROUND(SUM(data_length + index_length) / 1024 / 1024, 1) AS size_mb
FROM information_schema.tables
GROUP BY table_schema
ORDER BY SUM(data_length + index_length) DESC;"#,
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
        r#"SELECT DATE(${column}) AS day, COUNT(*)
FROM ${table}
GROUP BY day
ORDER BY day DESC
LIMIT 30;"#,
    ),
    CheatEntry::new(
        "Query",
        "Duplicate values of a column",
        r#"SELECT ${column}, COUNT(*)
FROM ${table}
GROUP BY ${column}
HAVING COUNT(*) > 1
ORDER BY 2 DESC
LIMIT 50;"#,
    ),
    CheatEntry::new(
        "Query",
        "Explain a query",
        "EXPLAIN\nSELECT * FROM ${table} LIMIT 100;",
    ),
];
