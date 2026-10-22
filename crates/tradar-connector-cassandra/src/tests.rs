#[test]
fn varint_and_decimal_read_as_the_number_they_hold() {
    // 1250 = 0x04E2, scale 2 -> 12.50 (the bytes a real Cassandra sent).
    assert_eq!(signed_be_to_decimal(&[4, 226]), "1250");
    assert_eq!(format_decimal("1250", 2), "12.50");
    assert_eq!(signed_be_to_decimal(&[0]), "0");
    assert_eq!(signed_be_to_decimal(&[127]), "127");
    assert_eq!(signed_be_to_decimal(&[0, 128]), "128");
    assert_eq!(signed_be_to_decimal(&[255]), "-1");
    assert_eq!(signed_be_to_decimal(&[255, 127]), "-129");
    assert_eq!(signed_be_to_decimal(&[128]), "-128");
    // 2^70 needs more than a machine word.
    assert_eq!(
        signed_be_to_decimal(&[0x40, 0, 0, 0, 0, 0, 0, 0, 0]),
        "1180591620717411303424"
    );
    assert_eq!(format_decimal("5", 3), "0.005");
    assert_eq!(format_decimal("-5", 3), "-0.005");
    assert_eq!(format_decimal("123", 0), "123");
    assert_eq!(format_decimal("12", -3), "12000");
    assert_eq!(format_decimal("0", -2), "0");
}

#[test]
fn timestamps_dates_and_times_read_as_calendar_text() {
    use scylla::value::{CqlDate, CqlTime, CqlTimestamp};
    // The value a real Cassandra returned for '2026-08-04 00:46:57+0000'.
    assert_eq!(
        stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(1_785_804_417_000))),
        "2026-08-04 00:46:57.000+0000"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(0))),
        "1970-01-01 00:00:00.000+0000"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Timestamp(CqlTimestamp(-1))),
        "1969-12-31 23:59:59.999+0000"
    );
    // 2^31 days is the epoch in CQL's `date` encoding.
    assert_eq!(
        stringify_cql_value(&CqlValue::Date(CqlDate((1u32 << 31) + 20_669))),
        "2026-08-04"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Time(CqlTime(3_723_000_000_456))),
        "01:02:03.000000456"
    );
}

#[test]
fn collections_read_the_way_cqlsh_prints_them() {
    let text = |s: &str| CqlValue::Text(s.to_string());
    assert_eq!(
        stringify_cql_value(&CqlValue::Set(vec![text("a"), text("b")])),
        "{'a', 'b'}"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::List(vec![CqlValue::Int(1), CqlValue::Int(2)])),
        "[1, 2]"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Map(vec![(text("k"), CqlValue::Int(1))])),
        "{'k': 1}"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Tuple(vec![Some(CqlValue::Int(1)), None])),
        "(1, NULL)"
    );
    assert_eq!(
        stringify_cql_value(&CqlValue::Set(vec![text("it's")])),
        "{'it''s'}"
    );
}

use super::*;
use scylla::value::{CqlValue, Row};
use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};

#[test]
fn cassandra_error_marker_points_at_the_quoted_token_s_position() {
    let message = "line 1:14 mismatched input 'FRO' expecting K_FROM";
    let query = "SELECT * FRO users";

    let marker = cassandra_error_marker(message, query).unwrap();

    assert!(marker.contains("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains('^'), "marker was: {marker}");
}

#[test]
fn cassandra_error_marker_is_none_for_a_message_with_no_quoted_token() {
    assert_eq!(
        cassandra_error_marker("unconfigured table users", "SELECT * FROM users"),
        None
    );
}

#[test]
fn cassandra_error_marker_is_none_when_the_token_is_not_actually_in_the_query() {
    assert_eq!(
        cassandra_error_marker(
            "no viable alternative at input 'XYZ'",
            "SELECT * FROM users"
        ),
        None
    );
}

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
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let entry = SchemaInfo::new("demo.events");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &[]),
        Some("SELECT * FROM \"demo\".\"events\" LIMIT 100;".to_string())
    );
}

#[test]
fn edit_source_delegates_to_the_shared_single_table_check() {
    let driver = CassandraDriver::new("127.0.0.1:9042");

    assert_eq!(
        driver.edit_source("SELECT * FROM demo.events"),
        Some("demo.events".to_string())
    );
    assert_eq!(
        driver.edit_source("SELECT * FROM demo.events JOIN demo.other ON true"),
        None
    );
}

#[test]
fn cql_value_literal_leaves_numbers_booleans_and_uuids_unquoted() {
    assert_eq!(cql_value_literal("42"), "42");
    assert_eq!(cql_value_literal("-3.5"), "-3.5");
    assert_eq!(cql_value_literal("true"), "true");
    assert_eq!(cql_value_literal("FALSE"), "false");
    assert_eq!(
        cql_value_literal("123e4567-e89b-12d3-a456-426614174000"),
        "123e4567-e89b-12d3-a456-426614174000"
    );
}

#[test]
fn cql_value_literal_quotes_everything_else() {
    assert_eq!(cql_value_literal("hello"), "'hello'");
    assert_eq!(cql_value_literal("it's"), "'it''s'");
    // Not uuid-shaped (wrong group lengths) -- quoted, not passed
    // through as if it were one.
    assert_eq!(cql_value_literal("1234-56-78"), "'1234-56-78'");
}

#[test]
fn edit_sql_sets_several_columns_in_one_update() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let edit = query_driver::RowEdit {
        table: "demo.events".to_string(),
        key: vec![(
            "id".to_string(),
            "123e4567-e89b-12d3-a456-426614174000".to_string(),
        )],
        change: query_driver::RowChange::SetValues(vec![
            ("count".to_string(), "7".to_string()),
            ("label".to_string(), "it's".to_string()),
        ]),
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "UPDATE \"demo\".\"events\" SET \"count\" = 7, \"label\" = 'it''s' WHERE \"id\" = \
             123e4567-e89b-12d3-a456-426614174000"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_builds_an_update_with_cql_typed_literals() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let edit = query_driver::RowEdit {
        table: "demo.events".to_string(),
        key: vec![(
            "id".to_string(),
            "123e4567-e89b-12d3-a456-426614174000".to_string(),
        )],
        change: query_driver::RowChange::SetValue {
            column: "count".to_string(),
            value: "7".to_string(),
        },
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "UPDATE \"demo\".\"events\" SET \"count\" = 7 WHERE \"id\" = \
             123e4567-e89b-12d3-a456-426614174000"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_builds_a_delete_with_a_composite_key_and_quotes_a_text_value() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let edit = query_driver::RowEdit {
        table: "demo.events".to_string(),
        key: vec![
            ("user_id".to_string(), "u1".to_string()),
            ("created_at".to_string(), "100".to_string()),
        ],
        change: query_driver::RowChange::DeleteRow,
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "DELETE FROM \"demo\".\"events\" WHERE \"user_id\" = 'u1' \
             AND \"created_at\" = 100"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_writes_a_null_key_column_without_turning_it_into_a_literal() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let edit = query_driver::RowEdit {
        table: "demo.events".to_string(),
        key: vec![("id".to_string(), "NULL".to_string())],
        change: query_driver::RowChange::DeleteRow,
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some("DELETE FROM \"demo\".\"events\" WHERE \"id\" IS NULL".to_string())
    );
}

#[test]
fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
    let driver = CassandraDriver::new("127.0.0.1:9042");
    let entry = SchemaInfo {
        name: "demo.events".to_string(),
        columns: vec![
            ColumnInfo::new("id", "uuid"),
            ColumnInfo::new("payload", "text"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &["payload".to_string()]),
        Some("SELECT \"payload\" FROM \"demo\".\"events\" LIMIT 100;".to_string())
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

        // `edit_sql` against the real cluster: both key columns are
        // `int` (unquoted per `cql_value_literal`) -- if either were
        // quoted, CQL would reject the statement outright
        // ("Invalid STRING constant ... for int") rather than
        // silently matching nothing, so running it is the real test,
        // not just the generated text.
        let edit = query_driver::RowEdit {
            table: "demo.memberships".to_string(),
            key: vec![
                ("user_id".to_string(), "1".to_string()),
                ("group_id".to_string(), "2".to_string()),
            ],
            change: query_driver::RowChange::SetValue {
                column: "role".to_string(),
                value: "owner".to_string(),
            },
        };
        let update_sql = driver.edit_sql(&edit).expect("Cassandra supports edit_sql");
        driver.execute(&update_sql).await.unwrap();
        let after_update = driver
            .execute("SELECT role FROM demo.memberships WHERE user_id = 1 AND group_id = 2")
            .await
            .unwrap();
        assert_eq!(
            after_update,
            QueryResult::Table {
                columns: vec!["role".to_string()],
                rows: vec![vec!["owner".to_string()]],
                truncated: false,
            }
        );

        let delete = query_driver::RowEdit {
            table: "demo.memberships".to_string(),
            key: vec![
                ("user_id".to_string(), "1".to_string()),
                ("group_id".to_string(), "2".to_string()),
            ],
            change: query_driver::RowChange::DeleteRow,
        };
        let delete_sql = driver
            .edit_sql(&delete)
            .expect("Cassandra supports edit_sql");
        driver.execute(&delete_sql).await.unwrap();
        let after_delete = driver
            .execute("SELECT role FROM demo.memberships WHERE user_id = 1 AND group_id = 2")
            .await
            .unwrap();
        assert_eq!(
            after_delete,
            QueryResult::Table {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
            "the row must actually be gone after DELETE"
        );
    }
}
