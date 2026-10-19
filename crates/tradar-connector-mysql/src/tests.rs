use super::*;
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[test]
fn near_token_marker_points_at_the_quoted_token_s_position() {
    let message =
        "You have an error in your SQL syntax; ... right syntax to use near 'FRO users' at line 1";
    let query = "SELECT * FRO users";

    let marker = near_token_marker(message, query).unwrap();

    assert!(marker.contains("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains('^'), "marker was: {marker}");
}

#[test]
fn near_token_marker_is_none_for_a_message_with_no_quoted_token() {
    assert_eq!(
        near_token_marker("Table 'mydb.users' doesn't exist", "SELECT * FROM users"),
        None
    );
}

#[test]
fn near_token_marker_is_none_when_the_token_is_not_actually_in_the_query() {
    assert_eq!(
        near_token_marker("... near 'XYZ' at line 1", "SELECT * FROM users"),
        None
    );
}

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn table_ddl_and_migrations_are_not_supported_at_v1() {
    // Unlike Postgres, this driver doesn't override `table_ddl` or
    // `supports_migrations` -- both features generate dialect-specific
    // DDL/tracking SQL that hasn't been built for MySQL/MariaDB yet,
    // so they fall back to the `QueryDriver` trait's own defaults.
    let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");

    assert_eq!(
        driver.table_ddl(&query_driver::TableDesignerOp::RenameTable {
            table: "users".to_string(),
            new_name: "accounts".to_string(),
        }),
        None
    );
    assert!(!driver.supports_migrations());
}

#[tokio::test]
async fn connect_fails_quickly_against_an_unreachable_host() {
    // Port 1 is reserved and never has a MySQL server listening -- no
    // Docker/testcontainers needed, connection is refused immediately at
    // the OS level. This is a regression test for the pool's default
    // 30s acquire_timeout, which made a bad connection target look
    // identical to a hung UI.
    let mut driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), driver.connect())
        .await
        .expect("connect() should fail well within 10s, not hang");

    assert!(result.is_err());
}

#[tokio::test]
async fn connect_succeeds_for_a_running_mysql() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");

    let mut driver = MySqlDriver::new(&conn_string);
    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn list_schema_returns_created_tables() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert_eq!(schema.len(), 1);
    assert_eq!(schema[0].name, "users");
    assert!(schema[0].columns[0].primary_key);
}

#[tokio::test]
async fn list_schema_reports_a_foreign_key_reference() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, \
         FOREIGN KEY (user_id) REFERENCES users (id))",
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
}

#[tokio::test]
async fn list_schema_reports_views_and_hides_system_schemas() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
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

    let schema = driver.list_schema().await.unwrap();

    let table = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(table.object_kind.as_deref(), Some("table"));
    let view = schema.iter().find(|s| s.name == "user_ids").unwrap();
    assert_eq!(view.object_kind.as_deref(), Some("view"));
    assert!(
        schema.iter().all(|s| !matches!(
            s.schema.as_deref(),
            Some("information_schema" | "mysql" | "performance_schema" | "sys")
        )),
        "system schemas must not show up in the navigator: {:?}",
        schema.iter().map(|s| &s.schema).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn execute_returns_columns_and_rows_for_a_select() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
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

/// Regression test: `stringify_column`'s old fallback (a plain `String`
/// decode for anything it didn't special-case) fails outright for
/// these types under sqlx's binary wire protocol -- and would otherwise
/// get swallowed into a plain "NULL", indistinguishable from an actual
/// null and wrong for every row that had one.
#[tokio::test]
async fn json_and_datetime_columns_decode_to_real_text_not_null() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query(
        "CREATE TABLE events (
            data JSON NOT NULL,
            at DATETIME NOT NULL,
            on_day DATE NOT NULL,
            nothing JSON
        )",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (data, at, on_day, nothing) VALUES
         ('{\"code\": \"ok\"}', '2026-08-04 00:46:57', '2026-08-04', NULL)",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();

    let result = driver
        .execute("SELECT data, at, on_day, nothing FROM events")
        .await
        .unwrap();

    let QueryResult::Table { rows, .. } = result else {
        panic!("expected a Table result");
    };
    let row = &rows[0];
    assert_eq!(row[0], "{\"code\": \"ok\"}");
    assert_eq!(row[1], "2026-08-04 00:46:57");
    assert_eq!(row[2], "2026-08-04");
    assert_eq!(
        row[3], "NULL",
        "an actual null must still say NULL, distinct from a decode failure"
    );
}

/// Found by running against a real server: `DECIMAL` (the type money
/// lives in), `ENUM`, `SET` and `BLOB` showed `NULL` for every row,
/// because the binary-protocol decoder list never covered them.
#[tokio::test]
async fn decimal_enum_set_and_blob_columns_show_their_text_not_null() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();
    sqlx::query(
        "CREATE TABLE exotic (
            price DECIMAL(8,2) NOT NULL,
            mood ENUM('sad','ok','happy') NOT NULL,
            perms SET('r','w','x') NOT NULL,
            blob_col BLOB NOT NULL,
            text_blob BLOB NOT NULL,
            flag BOOLEAN NOT NULL,
            nothing DECIMAL(8,2)
        )",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO exotic VALUES
         (12.50, 'happy', 'r,w', X'DEADBEEF', 'plain text', TRUE, NULL)",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();

    let result = driver
        .execute("SELECT price, mood, perms, blob_col, text_blob, flag, nothing FROM exotic")
        .await
        .unwrap();

    let QueryResult::Table { rows, .. } = result else {
        panic!("expected a Table result");
    };
    let row = &rows[0];
    assert_eq!(row[0], "12.50");
    assert_eq!(row[1], "happy");
    assert_eq!(row[2], "r,w");
    assert_eq!(row[3], "0xdeadbeef");
    assert_eq!(row[4], "plain text");
    assert_eq!(row[5], "true");
    assert_eq!(row[6], "NULL", "a real null still says NULL");
}

#[tokio::test]
async fn a_fresh_driver_is_not_in_a_transaction() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();

    assert!(!driver.in_transaction());
}

#[tokio::test]
async fn commit_closes_the_transaction_and_keeps_the_change() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
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
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut writer = MySqlDriver::new(&conn_string);
    writer.connect().await.unwrap();
    writer
        .execute("CREATE TABLE users (id INTEGER) ENGINE=InnoDB")
        .await
        .unwrap();
    let mut reader = MySqlDriver::new(&conn_string);
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
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();

    let error = driver
        .execute("SELECT * FRO users")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("LINE 1:"), "error was: {error}");
    assert!(error.contains('^'), "error was: {error}");
}
