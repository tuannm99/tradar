//! The SQLite cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Explore",
        "Tables and views",
        r#"SELECT type, name
FROM sqlite_master
WHERE type IN ('table', 'view')
ORDER BY type, name;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        "SELECT * FROM pragma_table_info('${table}');",
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a table",
        "SELECT * FROM pragma_index_list('${table}');",
    ),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        "SELECT * FROM pragma_foreign_key_list('${table}');",
    ),
    CheatEntry::new(
        "Explore",
        "CREATE statement of a table or index",
        "SELECT sql FROM sqlite_master WHERE name = '${name}';",
    ),
    CheatEntry::new("Explore", "SQLite version", "SELECT sqlite_version();"),
    // ── Health ──────────────────────────────────────────────────────────
    CheatEntry::new("Health", "Integrity check", "PRAGMA integrity_check;"),
    CheatEntry::new(
        "Health",
        "Database size in bytes",
        r#"SELECT page_count * page_size AS bytes
FROM pragma_page_count(), pragma_page_size();"#,
    ),
    CheatEntry::new("Health", "Journal mode", "PRAGMA journal_mode;"),
    // ── Query ───────────────────────────────────────────────────────────
    CheatEntry::new(
        "Query",
        "First rows of a table",
        "SELECT *\nFROM ${table}\nLIMIT 100;",
    ),
    CheatEntry::new(
        "Query",
        "Row count of a table",
        "SELECT COUNT(*) FROM ${table};",
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r#"SELECT date(${column}) AS day, COUNT(*)
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
        "Explain a query plan",
        "EXPLAIN QUERY PLAN\nSELECT * FROM ${table};",
    ),
];
