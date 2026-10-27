//! The Cassandra (CQL) cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ──
    CheatEntry::new(
        "Explore",
        "Keyspaces and their replication",
        r##"SELECT keyspace_name, replication, durable_writes
FROM system_schema.keyspaces;"##,
    ),
    CheatEntry::new(
        "Explore",
        "Tables of a keyspace",
        r##"SELECT keyspace_name, table_name, default_time_to_live
FROM system_schema.tables
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table (partition and clustering keys first)",
        r##"SELECT column_name, kind, clustering_order, type
FROM system_schema.columns
WHERE keyspace_name = '${keyspace}' AND table_name = '${table}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a keyspace",
        r##"SELECT table_name, index_name, kind, options
FROM system_schema.indexes
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Materialized views of a keyspace",
        r##"SELECT view_name, base_table_name
FROM system_schema.views
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "User-defined types of a keyspace",
        r##"SELECT type_name, field_names, field_types
FROM system_schema.types
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Functions of a keyspace",
        r##"SELECT function_name, argument_types, return_type
FROM system_schema.functions
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new(
        "Explore",
        "Table options (compaction, TTL, caching)",
        r##"SELECT compaction, default_time_to_live, gc_grace_seconds, caching
FROM system_schema.tables
WHERE keyspace_name = '${keyspace}' AND table_name = '${table}';"##,
    ),
    // ── Cluster ──
    CheatEntry::new(
        "Cluster",
        "This node",
        r##"SELECT cluster_name, release_version, data_center, rack, partitioner
FROM system.local;"##,
    ),
    CheatEntry::new(
        "Cluster",
        "Other nodes",
        r##"SELECT peer, data_center, rack, release_version
FROM system.peers;"##,
    ),
    CheatEntry::new(
        "Cluster",
        "Partition size estimates of a keyspace",
        r##"SELECT table_name, partitions_count, mean_partition_size
FROM system.size_estimates
WHERE keyspace_name = '${keyspace}';"##,
    ),
    CheatEntry::new("Cluster", "Roles", r##"LIST ROLES;"##),
    CheatEntry::new(
        "Cluster",
        "Permissions of a role",
        r##"LIST ALL PERMISSIONS OF ${role};"##,
    ),
    // ── Query ──
    CheatEntry::new(
        "Query",
        "First rows of a table",
        r##"SELECT *
FROM ${keyspace}.${table}
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows of one partition",
        r##"SELECT *
FROM ${keyspace}.${table}
WHERE ${key} = ${value}
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows of one partition, newest first (clustering order)",
        r##"SELECT *
FROM ${keyspace}.${series_table}
WHERE ${key} = ${value}
ORDER BY ${clustering_column} DESC
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Several partitions (IN)",
        r##"SELECT *
FROM ${keyspace}.${table}
WHERE ${key} IN (${value}, ${second_value});"##,
    ),
    CheatEntry::new(
        "Query",
        "A range of a clustering column",
        r##"SELECT *
FROM ${keyspace}.${series_table}
WHERE ${key} = ${value} AND ${clustering_column} >= ${from} AND ${clustering_column} < ${to};"##,
    ),
    CheatEntry::new(
        "Query",
        "Filter on a non-key column (ALLOW FILTERING, scans)",
        r##"SELECT *
FROM ${keyspace}.${table}
WHERE ${column} = '${other_value}'
LIMIT 20
ALLOW FILTERING;"##,
    ),
    CheatEntry::new(
        "Query",
        "Walk the whole table in token order",
        r##"SELECT token(${key}), ${key}
FROM ${keyspace}.${table}
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Row count (scans the table -- slow on a big one)",
        r##"SELECT COUNT(*)
FROM ${keyspace}.${table};"##,
    ),
    CheatEntry::new(
        "Query",
        "Remaining time to live and write time of a column",
        r##"SELECT ${column}, TTL(${column}), WRITETIME(${column})
FROM ${keyspace}.${table}
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Query",
        "Rows as JSON",
        r##"SELECT JSON *
FROM ${keyspace}.${table}
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "Query",
        "Distinct partition keys",
        r##"SELECT DISTINCT ${key}
FROM ${keyspace}.${table}
LIMIT 20;"##,
    ),
    // ── CQL syntax ──
    CheatEntry::new(
        "CQL syntax",
        "Time functions",
        r##"SELECT now() AS now_timeuuid, toTimestamp(now()) AS now_timestamp, toUnixTimestamp(now()) AS now_millis
FROM system.local;"##,
    ),
    CheatEntry::new(
        "CQL syntax",
        "Rows of a time window (timeuuid column)",
        r##"SELECT *
FROM ${keyspace}.${series_table}
WHERE ${key} = ${value} AND ${clustering_column} > minTimeuuid('2024-01-01 00:00:00+0000')
LIMIT 20;"##,
    ),
    CheatEntry::new(
        "CQL syntax",
        "A value in a collection column (CONTAINS)",
        r##"SELECT *
FROM ${keyspace}.${table}
WHERE ${set_column} CONTAINS '${value}'
LIMIT 20
ALLOW FILTERING;"##,
    ),
    // ── Write ──
    CheatEntry::new(
        "Write",
        "INSERT a row",
        r##"INSERT INTO ${keyspace}.${table} (${key}, ${column})
VALUES (${value}, '${other_value}');"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT only if the row is new",
        r##"INSERT INTO ${keyspace}.${table} (${key}, ${column})
VALUES (${value}, '${other_value}')
IF NOT EXISTS;"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT a row that expires (TTL in seconds)",
        r##"INSERT INTO ${keyspace}.${table} (${key}, ${column})
VALUES (${value}, '${other_value}')
USING TTL 86400;"##,
    ),
    CheatEntry::new(
        "Write",
        "INSERT a row from JSON",
        r##"INSERT INTO ${keyspace}.${table}
JSON '{"${key}": ${value}, "${column}": "${other_value}"}';"##,
    ),
    CheatEntry::new(
        "Write",
        "UPDATE a column",
        r##"UPDATE ${keyspace}.${table}
SET ${column} = '${other_value}'
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Write",
        "UPDATE only if a column has a value (lightweight transaction)",
        r##"UPDATE ${keyspace}.${table}
SET ${column} = '${other_value}'
WHERE ${key} = ${value}
IF ${column} = '${old_value}';"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to a set column",
        r##"UPDATE ${keyspace}.${table}
SET ${set_column} = ${set_column} + {'${other_value}'}
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Write",
        "Add to a counter column (a counter table)",
        r##"UPDATE ${keyspace}.${counter_table}
SET ${counter_column} = ${counter_column} + 1
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE a row",
        r##"DELETE FROM ${keyspace}.${table}
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Write",
        "DELETE one column of a row",
        r##"DELETE ${column} FROM ${keyspace}.${table}
WHERE ${key} = ${value};"##,
    ),
    CheatEntry::new(
        "Write",
        "Empty a table",
        r##"TRUNCATE ${keyspace}.${table};"##,
    ),
    // ── Schema ──
    CheatEntry::new(
        "Schema",
        "CREATE KEYSPACE",
        r##"CREATE KEYSPACE ${new_keyspace}
WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 1};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE KEYSPACE (several data centers)",
        r##"CREATE KEYSPACE ${new_keyspace}
WITH replication = {'class': 'NetworkTopologyStrategy', '${datacenter}': 3};"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE TABLE",
        r##"CREATE TABLE ${keyspace}.${new_table} (
  id uuid,
  created_at timestamp,
  name text,
  PRIMARY KEY (id, created_at)
) WITH CLUSTERING ORDER BY (created_at DESC);"##,
    ),
    CheatEntry::new(
        "Schema",
        "Add a column",
        r##"ALTER TABLE ${keyspace}.${table} ADD ${new_column} text;"##,
    ),
    CheatEntry::new(
        "Schema",
        "Drop a column",
        r##"ALTER TABLE ${keyspace}.${table} DROP ${column};"##,
    ),
    CheatEntry::new(
        "Schema",
        "Set a default TTL on a table",
        r##"ALTER TABLE ${keyspace}.${table} WITH default_time_to_live = 86400;"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE INDEX",
        r##"CREATE INDEX ${index_name} ON ${keyspace}.${table} (${column});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE MATERIALIZED VIEW",
        r##"CREATE MATERIALIZED VIEW ${keyspace}.${new_view} AS
SELECT * FROM ${keyspace}.${table}
WHERE ${column} IS NOT NULL AND ${key} IS NOT NULL
PRIMARY KEY (${column}, ${key});"##,
    ),
    CheatEntry::new(
        "Schema",
        "CREATE TYPE",
        r##"CREATE TYPE ${keyspace}.${new_type} (street text, city text, zip text);"##,
    ),
    CheatEntry::new(
        "Schema",
        "DROP INDEX",
        r##"DROP INDEX ${keyspace}.${index_name};"##,
    ),
    CheatEntry::new(
        "Schema",
        "DROP TABLE",
        r##"DROP TABLE ${keyspace}.${table};"##,
    ),
    CheatEntry::new("Schema", "DROP KEYSPACE", r##"DROP KEYSPACE ${keyspace};"##),
    // ── Admin ──
    CheatEntry::new(
        "Admin",
        "CREATE a role",
        r##"CREATE ROLE ${role} WITH PASSWORD = '${password}' AND LOGIN = true;"##,
    ),
    CheatEntry::new(
        "Admin",
        "GRANT SELECT on a keyspace",
        r##"GRANT SELECT ON KEYSPACE ${keyspace} TO ${role};"##,
    ),
    CheatEntry::new(
        "Admin",
        "REVOKE all permissions on a keyspace",
        r##"REVOKE ALL PERMISSIONS ON KEYSPACE ${keyspace} FROM ${role};"##,
    ),
];
