//! Cassandra connector: implements `QueryDriver` directly against the
//! `scylla` crate (a pure-Rust CQL binary-protocol client that also speaks
//! to real Apache Cassandra, not just ScyllaDB), and exposes it to
//! `tradar-app` only through `connector()` -- nothing else in this crate is
//! `pub`, so the driver's internals stay this crate's own business, not
//! something the rest of the app can reach into.

use async_trait::async_trait;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use scylla::response::query_result::IntoRowsResultError;
use scylla::value::{CqlValue, Row};

use tradar_connector_api::{Connector, ConnectorDescriptor, Session as ConnectorSession};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
use tradar_query_workbench::query_engine::QueryEngine;

struct CassandraDriver {
    /// A bare `host:port` contact point (e.g. `127.0.0.1:9042`) -- not a
    /// URI, since `SessionBuilder::known_node` wants exactly that and
    /// Cassandra has no single default database to fold into the target
    /// the way Postgres/Mongo's connection strings do.
    contact_point: String,
    session: Option<Session>,
}

impl CassandraDriver {
    fn new(contact_point: &str) -> Self {
        Self {
            contact_point: contact_point.to_string(),
            session: None,
        }
    }

    fn session(&self) -> &Session {
        self.session
            .as_ref()
            .expect("connect() must be called first")
    }
}

/// Turns one cell into display text. Cassandra's CQL types are a closed,
/// well-known set (unlike Mongo's arbitrary BSON), so the common scalars
/// get a real conversion; the handful of composite/exotic types (list,
/// set, map, tuple, UDT, vector, duration, decimal, varint, counter) fall
/// back to `Debug` -- passthrough rather than inventing a bespoke
/// serialization, same choice made for Mongo's `Bson::element_type()`.
fn stringify_cql_value(value: &CqlValue) -> String {
    match value {
        CqlValue::Ascii(s) | CqlValue::Text(s) => s.clone(),
        CqlValue::Boolean(b) => b.to_string(),
        CqlValue::TinyInt(n) => n.to_string(),
        CqlValue::SmallInt(n) => n.to_string(),
        CqlValue::Int(n) => n.to_string(),
        CqlValue::BigInt(n) => n.to_string(),
        CqlValue::Float(n) => n.to_string(),
        CqlValue::Double(n) => n.to_string(),
        CqlValue::Uuid(u) => u.to_string(),
        CqlValue::Timeuuid(u) => u.to_string(),
        CqlValue::Inet(ip) => ip.to_string(),
        CqlValue::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
        CqlValue::Empty => String::new(),
        other => format!("{other:?}"),
    }
}

fn stringify_row(row: &Row) -> Vec<String> {
    row.columns
        .iter()
        .map(|cell| match cell {
            Some(value) => stringify_cql_value(value),
            None => "NULL".to_string(),
        })
        .collect()
}

/// Sort key for `system_schema.columns.kind` -- partition key first, then
/// clustering, then everything else (regular/static columns, where
/// `position` is meaningless anyway).
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "partition_key" => 0,
        "clustering" => 1,
        _ => 2,
    }
}

#[async_trait]
impl QueryDriver for CassandraDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let session = SessionBuilder::new()
            .known_node(&self.contact_point)
            .build()
            .await?;
        self.session = Some(session);
        Ok(())
    }

    fn keywords(&self) -> &'static [&'static str] {
        // CQL syntax is close enough to SQL that the shared vocabulary
        // (SELECT/INSERT/WHERE/...) is still the right suggestion set --
        // same reasoning that already lets Postgres and SQLite share it.
        query_driver::SQL_KEYWORDS
    }

    fn split_statements(&self, text: &str) -> Vec<query_driver::Statement> {
        // CQL also separates statements with `;` and has no dollar-quote
        // syntax of its own, but that branch in the shared splitter only
        // ever triggers on a literal `$`, so it's inert here rather than
        // wrong.
        query_driver::split_sql_statements(text)
    }

    fn crud_snippet(&self, entry: &SchemaInfo, op: tradar_core::action::CrudOp) -> Option<String> {
        Some(query_driver::build_crud_snippet(entry, op))
    }

    async fn ping(&self) -> anyhow::Result<()> {
        // Cassandra has no equivalent of a guaranteed-empty table to
        // `SELECT 1` against; `system.local` always exists and always has
        // exactly one row, so it's the standard health-check query.
        self.session()
            .query_unpaged("SELECT key FROM system.local", &[])
            .await?;
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let session = self.session();
        let tables_result = session
            .query_unpaged(
                "SELECT keyspace_name, table_name FROM system_schema.tables",
                &[],
            )
            .await?
            .into_rows_result()?;
        let tables: Vec<(String, String)> = tables_result
            .rows::<(String, String)>()?
            .collect::<Result<_, _>>()?;

        let mut schema = Vec::new();
        for (keyspace, table) in tables {
            // Cassandra's own system keyspaces aren't user data -- same
            // reason Postgres schema browsing skips `pg_catalog`.
            if keyspace.starts_with("system") {
                continue;
            }

            let columns_result = session
                .query_unpaged(
                    "SELECT column_name, type, kind, position FROM system_schema.columns \
                     WHERE keyspace_name = ? AND table_name = ?",
                    (&keyspace, &table),
                )
                .await?
                .into_rows_result()?;
            let mut columns: Vec<(String, String, String, i32)> = columns_result
                .rows::<(String, String, String, i32)>()?
                .collect::<Result<_, _>>()?;
            // A plain SELECT makes no row-order guarantee, so a composite
            // key can otherwise come back in any order -- `kind` sorts
            // partition-key columns before clustering columns (before
            // everything else), and `position` (0-based *within* each
            // kind, -1 for non-key columns) recovers the declared order
            // inside each: `PRIMARY KEY (user_id, group_id)` must report
            // exactly that order, not "whatever this SELECT happened to
            // return".
            columns.sort_by_key(|(_, _, kind, position)| (kind_rank(kind), *position));

            schema.push(SchemaInfo {
                // Keyspace-qualified, same as Postgres's `public.users` --
                // `build_crud_snippet`'s `quote_identifier` already splits
                // on `.` to quote each part separately.
                name: format!("{keyspace}.{table}"),
                columns: columns
                    .into_iter()
                    .map(|(name, type_name, kind, _)| ColumnInfo {
                        name,
                        type_name,
                        // A row's identity is its partition key plus (if
                        // any) clustering columns together -- there's no
                        // single-column "the primary key" flag the way SQL
                        // has one, but "part of what a WHERE needs to
                        // address one row" is the same concept
                        // `ColumnInfo::primary_key` already means here.
                        primary_key: kind == "partition_key" || kind == "clustering",
                    })
                    .collect(),
                kind: None,
                ttl: None,
            });
        }
        Ok(schema)
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let result = self.session().query_unpaged(query, &[]).await?;
        match result.into_rows_result() {
            Ok(rows_result) => {
                let columns: Vec<String> = rows_result
                    .column_specs()
                    .iter()
                    .map(|spec| spec.name().to_string())
                    .collect();
                let mut out_rows = Vec::new();
                let mut truncated = false;
                for row in rows_result.rows::<Row>()? {
                    if out_rows.len() == query_driver::MAX_ROWS {
                        truncated = true;
                        break;
                    }
                    out_rows.push(stringify_row(&row?));
                }
                Ok(QueryResult::Table {
                    columns,
                    rows: out_rows,
                    truncated,
                })
            }
            // Not a Rows response -- a write or DDL statement. CQL's wire
            // protocol has no equivalent of SQL's affected-row count for
            // these (a `Void` result carries nothing else), so this is
            // always 0 -- not a shortcut, an actual protocol limitation.
            Err(IntoRowsResultError::ResultNotRows(_)) => Ok(QueryResult::Affected { rows: 0 }),
            Err(e) => Err(e.into()),
        }
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "cassandra",
    display_name: "Cassandra",
    icon: "🌀",
    capabilities: &[Capability::Query, Capability::Schema, Capability::Export],
};

struct CassandraConnector;

#[async_trait]
impl Connector for CassandraConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(
        &self,
        connection: SavedConnection,
    ) -> anyhow::Result<Box<dyn ConnectorSession>> {
        let mut driver = CassandraDriver::new(&connection.target);
        tradar_connector_api::with_connect_timeout(&connection.target, driver.connect()).await?;
        let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
        let schema = driver.list_schema().await.map_err(|e| e.to_string());
        Ok(Box::new(QueryEngine::new(driver, connection, schema)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(CassandraConnector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_text_pass_through_verbatim() {
        assert_eq!(
            stringify_cql_value(&CqlValue::Text("hello".to_string())),
            "hello"
        );
        assert_eq!(
            stringify_cql_value(&CqlValue::Ascii("hi".to_string())),
            "hi"
        );
    }

    #[test]
    fn numeric_and_boolean_variants_stringify_plainly() {
        assert_eq!(stringify_cql_value(&CqlValue::Int(42)), "42");
        assert_eq!(stringify_cql_value(&CqlValue::BigInt(-7)), "-7");
        assert_eq!(stringify_cql_value(&CqlValue::Boolean(true)), "true");
        assert_eq!(stringify_cql_value(&CqlValue::Double(1.5)), "1.5");
    }

    #[test]
    fn a_null_cell_is_reported_as_null_not_empty_string() {
        let row = Row {
            columns: vec![Some(CqlValue::Int(1)), None],
        };

        assert_eq!(
            stringify_row(&row),
            vec!["1".to_string(), "NULL".to_string()]
        );
    }

    #[test]
    fn an_exotic_variant_falls_back_to_debug_rather_than_erroring() {
        let value = CqlValue::List(vec![CqlValue::Int(1), CqlValue::Int(2)]);

        // Not asserting the exact Debug text (that's scylla's concern, not
        // ours) -- just that it doesn't panic and produces *something*.
        assert!(!stringify_cql_value(&value).is_empty());
    }

    #[test]
    fn crud_snippet_delegates_to_the_shared_sql_builder() {
        let driver = CassandraDriver::new("127.0.0.1:9042");
        let entry = SchemaInfo::new("demo.events");

        assert_eq!(
            driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read),
            Some("SELECT * FROM \"demo\".\"events\" LIMIT 100;".to_string())
        );
    }

    #[test]
    fn a_fresh_driver_is_never_in_a_transaction() {
        // No override -- Cassandra has no BEGIN/COMMIT/ROLLBACK, so this
        // must stay on QueryDriver's default rather than something this
        // crate has to maintain.
        let driver = CassandraDriver::new("127.0.0.1:9042");

        assert!(!driver.in_transaction());
    }

    mod docker {
        //! Integration tests against a real Cassandra, via `testcontainers`
        //! directly rather than `testcontainers-modules` -- that crate has
        //! no `cassandra` feature (confirmed via `cargo add --dry-run`
        //! before writing this). Unlike the other connectors' per-test
        //! containers (cheap and fast for Postgres/Redis/Mongo), Cassandra
        //! takes 30-60s just to start accepting CQL connections, so every
        //! assertion here shares a single container instead of spinning up
        //! one per test.

        use std::time::Duration;

        use testcontainers::core::{IntoContainerPort, WaitFor};
        use testcontainers::runners::AsyncRunner;
        use testcontainers::{ContainerAsync, GenericImage, ImageExt};

        use super::*;

        /// Returns the container alongside the driver -- the container
        /// must stay alive (and in scope) for as long as the driver is
        /// used, or it stops the moment this function returns.
        ///
        /// Needs host port 9042 free (not just *a* random one, see the
        /// `with_mapped_port`/`CASSANDRA_BROADCAST_RPC_ADDRESS` comment
        /// below) -- can't run alongside the long-lived dev instance from
        /// `docker compose up cassandra`.
        async fn connected_driver() -> (ContainerAsync<GenericImage>, CassandraDriver) {
            let container = GenericImage::new("cassandra", "5.0")
                .with_wait_for(WaitFor::message_on_stdout(
                    "Starting listening for CQL clients",
                ))
                // Cassandra always advertises ITS OWN port (9042) as part
                // of `broadcast_rpc_address` -- there's no setting for a
                // separate advertised port -- so the docker-assigned host
                // port has to be 9042 too, not whatever a random
                // `with_exposed_port` would pick; otherwise the control
                // connection (opened directly against `known_node`) works
                // but every real query, routed through a second pool
                // connection opened to the *advertised* address, times out.
                // The address side of that same problem is
                // `CASSANDRA_BROADCAST_RPC_ADDRESS` below -- without it
                // Cassandra advertises its Docker-internal bridge IP
                // instead of anything reachable from outside the
                // container. Root-caused by diffing a working `ping()`
                // against `system.local`'s `rpc_address` on a long-lived
                // container -- see the same fix in `docker-compose.yml`.
                .with_mapped_port(9042, 9042.tcp())
                .with_env_var("CASSANDRA_BROADCAST_RPC_ADDRESS", "127.0.0.1")
                // The JVM-based Cassandra image routinely needs 60-90s to
                // reach that log line -- past testcontainers' 60s default
                // startup timeout.
                .with_startup_timeout(Duration::from_secs(150))
                .start()
                .await
                .expect("cassandra container must start");
            let port = container
                .get_host_port_ipv4(9042)
                .await
                .expect("cassandra must expose 9042");

            let mut driver = CassandraDriver::new(&format!("127.0.0.1:{port}"));
            // The log line above means the native transport server has
            // started, not that the whole node (topology/gossip) is ready
            // to actually route a query yet -- `build()` can succeed
            // (control connection up) while the very next real query still
            // fails. A handful of retries covers that last stretch (this
            // is no longer papering over the address-mismatch bug -- see
            // the `with_mapped_port`/`CASSANDRA_BROADCAST_RPC_ADDRESS`
            // comment above -- just genuine "just started" variance).
            let mut last_error = None;
            'ready: for _ in 0..5 {
                match tokio::time::timeout(Duration::from_secs(10), driver.connect()).await {
                    Ok(Ok(())) => {
                        match tokio::time::timeout(Duration::from_secs(10), driver.ping()).await {
                            Ok(Ok(())) => {
                                last_error = None;
                                break 'ready;
                            }
                            Ok(Err(e)) => last_error = Some(e.to_string()),
                            Err(_) => last_error = Some("ping attempt timed out".to_string()),
                        }
                    }
                    Ok(Err(e)) => last_error = Some(e.to_string()),
                    Err(_) => last_error = Some("connect attempt timed out".to_string()),
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            if let Some(error) = last_error {
                panic!("cassandra never became ready to query: {error}");
            }
            (container, driver)
        }

        #[tokio::test]
        async fn ping_succeeds_against_a_running_cassandra() {
            let (_container, driver) =
                tokio::time::timeout(Duration::from_secs(350), connected_driver())
                    .await
                    .expect("container must become ready within 350s");

            let result = driver.ping().await;

            assert!(result.is_ok(), "ping failed: {:?}", result.err());
        }

        #[tokio::test]
        async fn schema_and_execute_round_trip_through_a_real_cluster() {
            let (_container, driver) =
                tokio::time::timeout(Duration::from_secs(350), connected_driver())
                    .await
                    .expect("container must become ready within 350s");

            driver
                .execute(
                    "CREATE KEYSPACE demo WITH replication = \
                     {'class': 'SimpleStrategy', 'replication_factor': 1}",
                )
                .await
                .unwrap();
            driver
                .execute(
                    "CREATE TABLE demo.memberships (\
                     user_id int, group_id int, role text, \
                     PRIMARY KEY (user_id, group_id))",
                )
                .await
                .unwrap();

            let schema = driver.list_schema().await.unwrap();
            let table = schema
                .iter()
                .find(|s| s.name == "demo.memberships")
                .expect("demo.memberships must be in the schema, system keyspaces filtered out");
            let key: Vec<&str> = table
                .columns
                .iter()
                .filter(|c| c.primary_key)
                .map(|c| c.name.as_str())
                .collect();
            assert_eq!(
                key,
                vec!["user_id", "group_id"],
                "partition key + clustering column together must be reported, in order"
            );
            let role = table
                .columns
                .iter()
                .find(|c| c.name == "role")
                .expect("role column must be present");
            assert!(!role.primary_key);
            assert_eq!(role.type_name, "text");

            let inserted = driver
                .execute(
                    "INSERT INTO demo.memberships (user_id, group_id, role) VALUES (1, 2, 'admin')",
                )
                .await
                .unwrap();
            assert_eq!(
                inserted,
                QueryResult::Affected { rows: 0 },
                "CQL never reports an affected-row count, even for a real insert"
            );

            let selected = driver
                .execute("SELECT user_id, group_id, role FROM demo.memberships WHERE user_id = 1")
                .await
                .unwrap();
            match selected {
                QueryResult::Table {
                    columns,
                    rows,
                    truncated,
                } => {
                    assert_eq!(columns, vec!["user_id", "group_id", "role"]);
                    assert_eq!(
                        rows,
                        vec![vec!["1".to_string(), "2".to_string(), "admin".to_string()]]
                    );
                    assert!(!truncated);
                }
                other => panic!("expected a table, got {other:?}"),
            }
        }
    }
}
