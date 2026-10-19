use super::*;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn table_ddl_delegates_to_the_shared_ddl_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

    assert_eq!(
        driver.table_ddl(&query_driver::TableDesignerOp::RenameTable {
            table: "users".to_string(),
            new_name: "accounts".to_string(),
        }),
        Some("ALTER TABLE \"users\" RENAME TO \"accounts\"".to_string())
    );
}

#[test]
fn supports_migrations_is_true() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

    assert!(driver.supports_migrations());
}

#[test]
fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");
    let entry = SchemaInfo {
        name: "users".to_string(),
        columns: vec![
            ColumnInfo::new("id", "INTEGER"),
            ColumnInfo::new("email", "TEXT"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    assert_eq!(
        driver.crud_snippet(
            &entry,
            tradar_core::action::CrudOp::Read,
            &["email".to_string()]
        ),
        Some("SELECT \"email\" FROM \"users\" LIMIT 100;".to_string())
    );
}

#[tokio::test]
async fn connect_fails_quickly_against_an_unreachable_host() {
    // Port 1 is reserved and never has a Postgres server listening -- no
    // Docker/testcontainers needed, connection is refused immediately at
    // the OS level. This is a regression test for PgPool::connect()'s
    // default 30s acquire_timeout, which made a bad connection target
    // look identical to a hung UI.
    let mut driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), driver.connect())
        .await
        .expect("connect() should fail well within 10s, not hang");

    assert!(result.is_err());
}

#[tokio::test]
async fn connect_succeeds_for_a_running_postgres() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let mut driver = PostgresDriver::new(&conn_string);
    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn list_schema_returns_created_tables() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert_eq!(schema.len(), 1);
    assert_eq!(schema[0].name, "users");
}

#[tokio::test]
async fn list_schema_reports_a_foreign_key_reference() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER \
         REFERENCES users (id))",
    )
    .execute(pool)
    .await
    .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let orders = schema.iter().find(|s| s.name == "orders").unwrap();
    let user_id = orders.columns.iter().find(|c| c.name == "user_id").unwrap();
    let fk = user_id
        .foreign_key
        .as_ref()
        .expect("user_id references users(id)");
    assert_eq!(fk.table, "users");
    assert_eq!(fk.column, "id");

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    let id = users.columns.iter().find(|c| c.name == "id").unwrap();
    assert!(
        id.foreign_key.is_none(),
        "users.id is a primary key, not itself a foreign key"
    );
}

#[tokio::test]
async fn list_schema_groups_multiple_schemas_and_hides_system_ones() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA reporting")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE reporting.summaries (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(users.schema.as_deref(), Some("public"));
    let summaries = schema.iter().find(|s| s.name == "summaries").unwrap();
    assert_eq!(summaries.schema.as_deref(), Some("reporting"));
    assert!(
        schema.iter().all(|s| !matches!(
            s.schema.as_deref(),
            Some("pg_catalog" | "information_schema")
        )),
        "system schemas must not show up in the navigator: {:?}",
        schema.iter().map(|s| &s.schema).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn list_schema_reports_views_functions_and_procedures_by_kind() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE VIEW user_ids AS SELECT id FROM users")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION greet() RETURNS int AS $$ SELECT 1 $$ LANGUAGE sql")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE PROCEDURE do_nothing() LANGUAGE sql AS $$ SELECT 1 $$")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let table = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(table.object_kind.as_deref(), Some("table"));
    let view = schema.iter().find(|s| s.name == "user_ids").unwrap();
    assert_eq!(view.object_kind.as_deref(), Some("view"));
    let function = schema.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(function.object_kind.as_deref(), Some("function"));
    let procedure = schema.iter().find(|s| s.name == "do_nothing").unwrap();
    assert_eq!(procedure.object_kind.as_deref(), Some("procedure"));
}

#[tokio::test]
async fn list_schema_qualifies_only_a_table_name_that_collides_across_schemas() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA reporting")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE reporting.users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert!(
        schema.iter().any(|s| s.name == "public.users"),
        "schema was: {:?}",
        schema.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert!(schema.iter().any(|s| s.name == "reporting.users"));
    assert!(
        schema.iter().any(|s| s.name == "orders"),
        "a non-colliding name must stay bare"
    );
}

#[tokio::test]
async fn execute_returns_columns_and_rows_for_a_select() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER, name TEXT)")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, name) VALUES (1, 'Ada')")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

    let result = driver.execute("SELECT id, name FROM users").await.unwrap();

    assert_eq!(
        result,
        QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![vec!["1".to_string(), "Ada".to_string()]],
            truncated: false,
        }
    );
}

/// Regression test: `stringify_column`'s old fallback (a plain
/// `String` decode for anything it didn't special-case) fails outright
/// for these types under sqlx's binary wire protocol -- and used to
/// get swallowed into a plain "NULL", indistinguishable from an
/// actual null and wrong for every row that had one. Now text protocol,
/// so JSON reads as Postgres formats it (`{"code": "ok"}`).
#[tokio::test]
async fn uuid_jsonb_and_timestamp_columns_decode_to_real_text_not_null() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query(
        "CREATE TABLE events (
            id UUID NOT NULL DEFAULT '3fa85f64-5717-4562-b3fc-2c963f66afa6',
            data JSONB NOT NULL,
            at TIMESTAMP NOT NULL,
            at_tz TIMESTAMPTZ NOT NULL,
            on_day DATE NOT NULL,
            nothing UUID
        )",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (data, at, at_tz, on_day, nothing) VALUES
         ('{\"code\": \"ok\"}', '2026-08-04 00:46:57', '2026-08-04 00:46:57+00', '2026-08-04', NULL)",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();

    let result = driver
        .execute("SELECT id, data, at, at_tz, on_day, nothing FROM events")
        .await
        .unwrap();

    let QueryResult::Table { rows, .. } = result else {
        panic!("expected a Table result");
    };
    let row = &rows[0];
    assert_eq!(row[0], "3fa85f64-5717-4562-b3fc-2c963f66afa6");
    assert_eq!(row[1], "{\"code\": \"ok\"}");
    assert_eq!(row[2], "2026-08-04 00:46:57");
    assert!(row[3].starts_with("2026-08-04 00:46:57"), "was: {}", row[3]);
    assert_eq!(row[4], "2026-08-04");
    assert_eq!(
        row[5], "NULL",
        "an actual null must still say NULL, distinct from a decode failure"
    );
}

/// Found by running against a real server: `numeric` (the type money
/// lives in) and arrays showed `NULL` for every row, because the
/// binary-protocol decoder list never covered them. Every type must
/// show the text Postgres itself would print.
#[tokio::test]
async fn numeric_arrays_and_other_exotic_types_show_their_text_not_null() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .await
        .unwrap();

    let result = driver
        .execute(
            "SELECT 12.50::numeric(8,2) AS price, 7::numeric AS whole, \
             '{x,y}'::text[] AS tags, ARRAY[1,2,3] AS nums, \
             'happy'::mood AS feeling, '10.0.0.1'::inet AS ip, \
             '1 day 02:03:04'::interval AS span, '\\xdeadbeef'::bytea AS blob, \
             true AS yes, false AS no, NULL::numeric AS nothing, \
             int4range(1, 5) AS span_range",
        )
        .await
        .unwrap();

    let QueryResult::Table { rows, .. } = result else {
        panic!("expected a Table result");
    };
    let row = &rows[0];
    assert_eq!(row[0], "12.50");
    assert_eq!(row[1], "7");
    assert_eq!(row[2], "{x,y}");
    assert_eq!(row[3], "{1,2,3}");
    assert_eq!(row[4], "happy");
    assert_eq!(row[5], "10.0.0.1");
    assert_eq!(row[6], "1 day 02:03:04");
    assert_eq!(row[7], "\\xdeadbeef");
    assert_eq!(row[8], "true");
    assert_eq!(row[9], "false");
    assert_eq!(row[10], "NULL", "a real null still says NULL");
    assert_eq!(row[11], "[1,5)");
}

#[tokio::test]
async fn a_fresh_driver_is_not_in_a_transaction() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();

    assert!(!driver.in_transaction());
}

#[tokio::test]
async fn commit_closes_the_transaction_and_keeps_the_change() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();

    driver.execute("BEGIN").await.unwrap();
    assert!(driver.in_transaction());
    driver
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();
    driver.execute("COMMIT").await.unwrap();

    assert!(!driver.in_transaction());
    let result = driver.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        result,
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        }
    );
}

#[tokio::test]
async fn an_uncommitted_insert_is_invisible_to_another_connection() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut writer = PostgresDriver::new(&conn_string);
    writer.connect().await.unwrap();
    writer
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();
    let mut reader = PostgresDriver::new(&conn_string);
    reader.connect().await.unwrap();

    writer.execute("BEGIN").await.unwrap();
    writer
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();

    // The point of holding one pooled connection for the whole
    // transaction: a second, independent connection to the same
    // database must not see the insert until it's committed.
    let seen_before_commit = reader.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        seen_before_commit,
        QueryResult::Table {
            columns: vec![],
            rows: vec![],
            truncated: false,
        }
    );

    writer.execute("ROLLBACK").await.unwrap();
    let seen_after_rollback = reader.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        seen_after_rollback,
        QueryResult::Table {
            columns: vec![],
            rows: vec![],
            truncated: false,
        },
        "a rolled-back insert must never become visible to anyone"
    );
}

#[tokio::test]
async fn a_syntax_error_reports_the_offending_line_with_a_caret() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();

    let error = driver
        .execute("SELECT * FRO users")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("LINE 1:"), "error was: {error}");
    assert!(error.contains('^'), "error was: {error}");
}

#[tokio::test]
async fn a_constraint_violation_reports_its_detail() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    driver
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();

    let error = driver
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("DETAIL:"), "error was: {error}");
}

#[tokio::test]
async fn cancel_query_really_stops_a_running_statement() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let driver = Arc::new(driver);

    let slow = Arc::clone(&driver);
    let handle = tokio::spawn(async move { slow.execute("SELECT pg_sleep(30)").await });

    // Give the query time to actually reach the server and start
    // running before cancelling it -- cancelling before `current` is
    // even set would be a no-op, not a real test of the real path.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    driver.cancel_query().await.unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
        .await
        .expect("cancel_query should make pg_sleep(30) return in well under its own 30s, not hang")
        .unwrap();

    let error = result.unwrap_err().to_string().to_lowercase();
    assert!(
        error.contains("cancel"),
        "expected a cancel-related error from Postgres, got: {error}"
    );
}
