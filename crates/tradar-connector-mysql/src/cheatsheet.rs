//! The MySQL / MariaDB cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new("Explore", "Databases", r##"SHOW DATABASES;"##),
    CheatEntry::new(
        "Explore",
        "Tables with size and approximate rows",
        r##"SELECT table_name, table_rows AS approx_rows,
       ROUND((data_length + index_length) / 1024 / 1024, 1) AS size_mb
FROM information_schema.tables
WHERE table_schema = DATABASE()
ORDER BY (data_length + index_length) DESC
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r##"SELECT column_name, column_type, is_nullable, column_default, column_key
FROM information_schema.columns
WHERE table_schema = DATABASE() AND table_name = '${table}'
ORDER BY ordinal_position;"##,
    ),
    CheatEntry::new(
        "Explore",
        "CREATE TABLE of a table",
        r##"SHOW CREATE TABLE ${table};"##,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a table",
        r##"SHOW INDEX FROM ${table};"##,
    ),
    CheatEntry::new(
        "Explore",
        "Foreign keys of a table",
        r##"SELECT constraint_name, column_name, referenced_table_name, referenced_column_name
FROM information_schema.key_column_usage
WHERE table_schema = DATABASE() AND table_name = '${table}' AND referenced_table_name IS NOT NULL;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Tables that reference a table",
        r##"SELECT table_name, column_name, constraint_name
FROM information_schema.key_column_usage
WHERE table_schema = DATABASE() AND referenced_table_name = '${table}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Views",
        r##"SELECT table_name
FROM information_schema.views
WHERE table_schema = DATABASE()
ORDER BY table_name;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Stored routines",
        r##"SELECT routine_name, routine_type
FROM information_schema.routines
WHERE routine_schema = DATABASE()
ORDER BY routine_name;"##,
    ),
    CheatEntry::new("Explore", "Triggers", r##"SHOW TRIGGERS;"##),
    CheatEntry::new(
        "Explore",
        "Table status (engine, rows, collation)",
        r##"SHOW TABLE STATUS LIKE '${table}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Users",
        r##"SELECT user, host FROM mysql.user ORDER BY user, host;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Grants of a user",
        r##"SHOW GRANTS FOR '${user}'@'${host}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Current user and database",
        r##"SELECT CURRENT_USER() AS user, DATABASE() AS db;"##,
    ),
    CheatEntry::new("Explore", "Server version", r##"SELECT VERSION();"##),
    // ── Activity ──
    CheatEntry::new("Activity", "Process list", r##"SHOW FULL PROCESSLIST;"##),
    CheatEntry::new(
        "Activity",
        "Running queries, longest first",
        r##"SELECT id, user, db, time AS seconds, state, LEFT(info, 100) AS query
FROM information_schema.processlist
WHERE command <> 'Sleep'
ORDER BY time DESC;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Connections per user",
        r##"SELECT user, COUNT(*) AS connections
FROM information_schema.processlist
GROUP BY user
ORDER BY connections DESC;"##,
    ),
    CheatEntry::new(
        "Activity",
        "InnoDB lock waits (MySQL 8+)",
        r##"SELECT * FROM performance_schema.data_lock_waits;"##,
    ),
    CheatEntry::new(
        "Activity",
        "Open transactions (InnoDB)",
        r##"SELECT trx_id, trx_state, trx_started, trx_mysql_thread_id, LEFT(trx_query, 100) AS query
FROM information_schema.innodb_trx
ORDER BY trx_started;"##,
    ),
    CheatEntry::new("Activity", "Replica status", r##"SHOW REPLICA STATUS;"##),
    // ── Status ──
    CheatEntry::new(
        "Status",
        "Connection counters",
        r##"SHOW GLOBAL STATUS LIKE 'Threads%';"##,
    ),
    CheatEntry::new(
        "Status",
        "Status counters matching a name",
        r##"SHOW GLOBAL STATUS LIKE '%${pattern}%';"##,
    ),
    CheatEntry::new(
        "Status",
        "Variables matching a name",
        r##"SHOW VARIABLES LIKE '%${pattern}%';"##,
    ),
    CheatEntry::new("Status", "InnoDB status", r##"SHOW ENGINE INNODB STATUS;"##),
    CheatEntry::new(
        "Status",
        "Slow query log settings",
        r##"SHOW VARIABLES LIKE 'slow_query%';"##,
    ),
    CheatEntry::new(
        "Status",
        "Maximum connections and how many are used",
        r##"SELECT @@max_connections AS max_connections,
       (SELECT COUNT(*) FROM information_schema.processlist) AS in_use;"##,
    ),
    // ── Size ──
    CheatEntry::new(
        "Size",
        "Databases by size",
        r##"SELECT table_schema AS db,
       ROUND(SUM(data_length + index_length) / 1024 / 1024, 1) AS size_mb
FROM information_schema.tables
GROUP BY table_schema
ORDER BY SUM(data_length + index_length) DESC;"##,
    ),
    CheatEntry::new(
        "Size",
        "Index size per table",
        r##"SELECT table_name, ROUND(index_length / 1024 / 1024, 1) AS index_mb, ROUND(data_length / 1024 / 1024, 1) AS data_mb
FROM information_schema.tables
WHERE table_schema = DATABASE()
ORDER BY index_length DESC
LIMIT 30;"##,
    ),
    // ── Health ──
    CheatEntry::new(
        "Health",
        "Indexes never used (MySQL 8 / sys schema)",
        r##"SELECT * FROM sys.schema_unused_indexes;"##,
    ),
    CheatEntry::new(
        "Health",
        "Tables without a primary key",
        r##"SELECT t.table_name
FROM information_schema.tables t
LEFT JOIN information_schema.table_constraints c
  ON c.table_schema = t.table_schema AND c.table_name = t.table_name AND c.constraint_type = 'PRIMARY KEY'
WHERE t.table_schema = DATABASE() AND t.table_type = 'BASE TABLE' AND c.constraint_name IS NULL;"##,
    ),
    CheatEntry::new(
        "Health",
        "Statements by total time (sys schema)",
        r##"SELECT query, exec_count, total_latency, avg_latency
FROM sys.statement_analysis
ORDER BY total_latency DESC
LIMIT 20;"##,
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
        r##"SELECT COUNT(*) FROM ${table};"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows per day",
        r##"SELECT DATE(${date_column}) AS day, COUNT(*)
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
        "Explain a query",
        r##"EXPLAIN
SELECT * FROM ${table} LIMIT 100;"##,
    ),
    CheatEntry::new(
        "Query",
        "Explain with timing (MySQL 8.0.18+)",
        r##"EXPLAIN ANALYZE
SELECT * FROM ${table} LIMIT 100;"##,
    ),
    // ── SQL syntax ──
    CheatEntry::new(
        "SQL syntax",
        "CTE (WITH, MySQL 8+)",
        r##"WITH recent AS (
  SELECT * FROM ${table} ORDER BY ${column} DESC LIMIT 100
)
SELECT COUNT(*) FROM recent;"##,
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
        "Count by condition",
        r##"SELECT SUM(${column} = '${a}') AS a,
       SUM(${column} = '${b}') AS b
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
        r##"SELECT NOW() - INTERVAL 7 DAY AS a_week_ago,
       DATE_FORMAT(NOW(), '%Y-%m-01') AS month_start,
       CURDATE() AS today;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "JSON field (JSON_EXTRACT)",
        r##"SELECT JSON_UNQUOTE(JSON_EXTRACT(${json_column}, '$.${json_key}')) AS value
FROM ${table}
LIMIT 50;"##,
    ),
    CheatEntry::new(
        "SQL syntax",
        "Join values into a string (GROUP_CONCAT)",
        r##"SELECT ${group_column}, GROUP_CONCAT(${column} SEPARATOR ', ') AS joined
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
        "Upsert (ON DUPLICATE KEY UPDATE)",
        r##"INSERT INTO ${table} (${key_column}, ${column})
VALUES ('${key}', '${value}')
ON DUPLICATE KEY UPDATE ${column} = VALUES(${column});"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT IGNORE (skip rows that already exist)",
        r##"INSERT IGNORE INTO ${table} (${key_column}, ${column})
VALUES ('${key}', '${value}');"##,
    ),
    CheatEntry::new(
        "Write",
        "REPLACE a row",
        r##"REPLACE INTO ${table} (${key_column}, ${column})
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
        r##"UPDATE ${table} t
JOIN ${other_table} o ON o.${join_column} = t.${join_column}
SET t.${column} = o.${column};"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE rows",
        r##"DELETE FROM ${table}
WHERE ${key_column} = '${key}';"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE in batches",
        r##"DELETE FROM ${table}
WHERE ${filter}
LIMIT 1000;"##,
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
  id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a column",
        r##"ALTER TABLE ${table} ADD COLUMN ${new_column} VARCHAR(255) NULL;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a column",
        r##"ALTER TABLE ${table} RENAME COLUMN ${column} TO ${new_column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Change the type of a column",
        r##"ALTER TABLE ${table} MODIFY COLUMN ${number_column} BIGINT NOT NULL;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a column",
        r##"ALTER TABLE ${table} DROP COLUMN ${column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Rename a table",
        r##"RENAME TABLE ${table} TO ${new_table};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a foreign key",
        r##"ALTER TABLE ${table}
ADD CONSTRAINT ${constraint} FOREIGN KEY (${fk_column}) REFERENCES ${other_table} (${join_column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE INDEX",
        r##"CREATE INDEX ${index_name} ON ${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE UNIQUE INDEX",
        r##"CREATE UNIQUE INDEX ${index_name} ON ${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "DROP INDEX",
        r##"DROP INDEX ${index_name} ON ${table};"##,
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
        "Kill a query (keep the connection)",
        r##"KILL QUERY ${id};"##,
    ),
    CheatEntry::new("Admin", "Kill a connection", r##"KILL CONNECTION ${id};"##),
    CheatEntry::new(
        "Admin",
        "Rebuild a table and update its statistics",
        r##"OPTIMIZE TABLE ${table};"##,
    ),
    CheatEntry::new(
        "Admin",
        "Update the statistics of a table",
        r##"ANALYZE TABLE ${table};"##,
    ),
    CheatEntry::new(
        "Admin",
        "CREATE a user",
        r##"CREATE USER '${user}'@'${host}' IDENTIFIED BY '${password}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "GRANT SELECT on a database",
        r##"GRANT SELECT ON ${database}.* TO '${user}'@'${host}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "REVOKE all privileges on a database",
        r##"REVOKE ALL PRIVILEGES ON ${database}.* FROM '${user}'@'${host}';"##,
    ),
    CheatEntry::new(
        "Admin",
        "Set a variable for this session",
        r##"SET SESSION max_execution_time = 30000;"##,
    ),
    // ── Transaction ──
    CheatEntry::new(
        "Transaction",
        "START TRANSACTION",
        r##"START TRANSACTION;"##,
    ),
    CheatEntry::new("Transaction", "COMMIT", r##"COMMIT;"##),
    CheatEntry::new("Transaction", "ROLLBACK", r##"ROLLBACK;"##),
    CheatEntry::new("Transaction", "SAVEPOINT", r##"SAVEPOINT ${name};"##),
    CheatEntry::new(
        "Transaction",
        "ROLLBACK TO a savepoint",
        r##"ROLLBACK TO SAVEPOINT ${name};"##,
    ),
];
