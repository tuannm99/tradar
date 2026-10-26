//! The Cassandra (CQL) cheat sheet -- see `QueryDriver::cheatsheet`.
use tradar_query::CheatEntry;

pub(crate) const ENTRIES: &[CheatEntry] = &[
    // ── Explore ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Explore",
        "Keyspaces and their replication",
        r#"SELECT keyspace_name, replication
FROM system_schema.keyspaces;"#,
    ),
    CheatEntry::new(
        "Explore",
        "Tables of a keyspace",
        r#"SELECT keyspace_name, table_name
FROM system_schema.tables
WHERE keyspace_name = '${keyspace}';"#,
    ),
    CheatEntry::new(
        "Explore",
        "Columns of a table",
        r#"SELECT column_name, kind, type
FROM system_schema.columns
WHERE keyspace_name = '${keyspace}' AND table_name = '${table}';"#,
    ),
    CheatEntry::new(
        "Explore",
        "Indexes of a keyspace",
        r#"SELECT table_name, index_name, kind
FROM system_schema.indexes
WHERE keyspace_name = '${keyspace}';"#,
    ),
    CheatEntry::new(
        "Explore",
        "Materialized views of a keyspace",
        r#"SELECT view_name, base_table_name
FROM system_schema.views
WHERE keyspace_name = '${keyspace}';"#,
    ),
    // ── Cluster ─────────────────────────────────────────────────────────
    CheatEntry::new(
        "Cluster",
        "This node",
        r#"SELECT cluster_name, release_version, data_center, rack
FROM system.local;"#,
    ),
    CheatEntry::new(
        "Cluster",
        "Other nodes",
        r#"SELECT peer, data_center, rack, release_version
FROM system.peers;"#,
    ),
    CheatEntry::new(
        "Cluster",
        "Partition size estimates of a keyspace",
        r#"SELECT table_name, partitions_count, mean_partition_size
FROM system.size_estimates
WHERE keyspace_name = '${keyspace}';"#,
    ),
    // ── Query ───────────────────────────────────────────────────────────
    CheatEntry::new(
        "Query",
        "First rows of a table",
        "SELECT *\nFROM ${keyspace}.${table}\nLIMIT 20;",
    ),
    CheatEntry::new(
        "Query",
        "Rows of one partition",
        r#"SELECT *
FROM ${keyspace}.${table}
WHERE ${key} = ${value}
LIMIT 20;"#,
    ),
    CheatEntry::new(
        "Query",
        "Row count (scans the table -- slow on a big one)",
        "SELECT COUNT(*)\nFROM ${keyspace}.${table};",
    ),
];
