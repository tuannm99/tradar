//! The SQLite cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new(
        "Explore",
        "Tables and views",
        r##"SELECT type, name
FROM sqlite_master
WHERE type IN ('table', 'view')
ORDER BY type, name;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r##"SELECT * FROM pragma_table_info('${table}');"##,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a table",
        r##"SELECT * FROM pragma_index_list('${table}');"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of an index",
        r##"SELECT * FROM pragma_index_info('${index_name}');"##,
    ),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        r##"SELECT * FROM pragma_foreign_key_list('${table}');"##,
    ),
    CheatEntry::new(
        "Explore",
        "CREATE statement of a table, view or index",
        r##"SELECT sql FROM sqlite_master WHERE name = '${name}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Triggers",
        r##"SELECT name, tbl_name FROM sqlite_master WHERE type = 'trigger';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Attached databases",
        r##"PRAGMA database_list;"##,
    ),
    CheatEntry::new("Explore", "SQLite version", r##"SELECT sqlite_version();"##),
    // ── Health ──
    CheatEntry::new("Health", "Integrity check", r##"PRAGMA integrity_check;"##),
    CheatEntry::new(
        "Health",
        "Foreign key violations",
        r##"PRAGMA foreign_key_check;"##,
    ),
    CheatEntry::new(
        "Health",
        "Database size in bytes",
        r##"SELECT page_count * page_size AS bytes
FROM pragma_page_count(), pragma_page_size();"##,
    ),
    CheatEntry::new(
        "Health",
        "Free pages (space VACUUM would give back)",
        r##"PRAGMA freelist_count;"##,
    ),
    CheatEntry::new("Health", "Journal mode", r##"PRAGMA journal_mode;"##),
    CheatEntry::new(
        "Health",
        "Are foreign keys enforced?",
        r##"PRAGMA foreign_keys;"##,
    ),
    CheatEntry::new("Health", "Compile options", r##"PRAGMA compile_options;"##),
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
        r##"SELECT COUNT(*) FROM ${table};"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r##"SELECT date(${column}) AS day, COUNT(*)
FROM ${table}
GROUP BY day
ORDER BY day DESC
LIMIT 30;"##,
    ),
    CheatEntry::new(
        "Query",
        "Duplicate values of a column",
        r##"SELECT ${column}, COUNT(*)
FROM ${table}
GROUP BY ${column}
HAVING COUNT(*) > 1
ORDER BY 2 DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Query",
        "Most frequent values of a column",
        r##"SELECT ${column}, COUNT(*) AS n
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
        "Explain a query plan",
        r##"EXPLAIN QUERY PLAN
SELECT * FROM ${table};"##,
    ),
    // ── SQL syntax ──
    CheatEntry::new(
        "SQL syntax",
        "CTE (WITH)",
        r##"WITH recent AS (
  SELECT * FROM ${table} ORDER BY ${column} DESC LIMIT 100
)
SELECT COUNT(*) FROM recent;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Recursive CTE (numbers 1 to 10)",
        r##"WITH RECURSIVE n(i) AS (
  SELECT 1
  UNION ALL
  SELECT i + 1 FROM n WHERE i < 10
)
SELECT i FROM n;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Top N per group (window function)",
        r##"SELECT *
FROM (
  SELECT t.*, ROW_NUMBER() OVER (PARTITION BY ${group_column} ORDER BY ${column} DESC) AS rn
  FROM ${table} t
) ranked
WHERE rn <= 3;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Running total (window function)",
        r##"SELECT ${column}, SUM(${value_column}) OVER (ORDER BY ${column}) AS running_total
FROM ${table}
ORDER BY ${column}
LIMIT 100;"##,
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
        r##"SELECT COUNT(*) FILTER (WHERE ${column} = '${a}') AS a,
       COUNT(*) FILTER (WHERE ${column} = '${b}') AS b
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
        r##"SELECT date('now', '-7 days') AS a_week_ago,
       date('now', 'start of month') AS month_start,
       date('now') AS today;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "JSON field (json_extract)",
        r##"SELECT json_extract(${json_column}, '$.${json_key}') AS value
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Join values into a string (group_concat)",
        r##"SELECT ${group_column}, group_concat(${column}, ', ') AS joined
FROM ${table}
GROUP BY ${group_column}
LIMIT 50;"##,
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
ON CONFLICT (${key_column}) DO UPDATE SET ${column} = excluded.${column};"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT OR REPLACE a row",
        r##"INSERT OR REPLACE INTO ${table} (${key_column}, ${column})
VALUES ('${key}', '${value}');"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT OR IGNORE (skip rows that already exist)",
        r##"INSERT OR IGNORE INTO ${table} (${key_column}, ${column})
VALUES ('${key}', '${value}');"##,
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
        r##"UPDATE ${table}
SET ${column} = o.${column}
FROM ${other_table} o
WHERE o.${join_column} = ${table}.${join_column};"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE rows",
        r##"DELETE FROM ${table}
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "Delete duplicates, keeping one",
        r##"DELETE FROM ${table}
WHERE rowid NOT IN (SELECT MIN(rowid) FROM ${table} GROUP BY ${column});"##,
    ),
    CheatEntry::new("Write", "Empty a table", r##"DELETE FROM ${table};"##),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "CREATE TABLE",
        r##"CREATE TABLE ${new_table} (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a column",
        r##"ALTER TABLE ${table} ADD COLUMN ${new_column} TEXT;"##,
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
        "Rename a table",
        r##"ALTER TABLE ${table} RENAME TO ${new_table};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE INDEX",
        r##"CREATE INDEX ${new_index} ON ${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE UNIQUE INDEX",
        r##"CREATE UNIQUE INDEX ${new_index} ON ${table} (${column});"##,
    ),
    CheatEntry::new("Schema", "DROP INDEX", r##"DROP INDEX ${index_name};"##),
    CheatEntry::new(
        "Schema",
        "CREATE VIEW",
        r##"CREATE VIEW ${new_view} AS
SELECT * FROM ${table};"##,
    ),
    CheatEntry::new("Schema", "DROP VIEW", r##"DROP VIEW ${view};"##),
    CheatEntry::new("Schema", "DROP TABLE", r##"DROP TABLE ${table};"##),
    // ── Admin ──
    CheatEntry::new(
        "Admin",
        "VACUUM (rebuild the file, give space back)",
        r##"VACUUM;"##,
    ),
    CheatEntry::new("Admin", "Update the planner statistics", r##"ANALYZE;"##),
    CheatEntry::new(
        "Admin",
        "Let SQLite optimise itself",
        r##"PRAGMA optimize;"##,
    ),
    CheatEntry::new(
        "Admin",
        "Rebuild the indexes of a table",
        r##"REINDEX ${table};"##,
    ),
    CheatEntry::new(
        "Admin",
        "Turn foreign keys on for this connection",
        r##"PRAGMA foreign_keys = ON;"##,
    ),
    CheatEntry::new(
        "Admin",
        "Switch to write-ahead logging",
        r##"PRAGMA journal_mode = WAL;"##,
    ),
    CheatEntry::new(
        "Admin",
        "Attach another database file",
        r##"ATTACH DATABASE '${path}' AS other;"##,
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
    CheatEntry::new(
        "Transaction",
        "RELEASE a savepoint",
        r##"RELEASE SAVEPOINT ${name};"##,
    ),
];
