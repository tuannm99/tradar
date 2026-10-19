use super::*;
use tradar_query_workbench::query_engine::QueryOutcome;

#[test]
fn near_token_marker_points_at_the_quoted_token_s_position() {
    let query = "SELECT * FRO users";
    let message = r#"near "FRO": syntax error"#;

    let marker = near_token_marker(message, query).unwrap();

    assert!(marker.starts_with("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains(query), "marker was: {marker}");
    let caret_line = marker.lines().nth(1).unwrap();
    assert_eq!(
        caret_line.find('^').unwrap(),
        marker.lines().next().unwrap().find("FRO").unwrap(),
        "caret should land under the F of FRO, marker was: {marker}"
    );
}

#[test]
fn near_token_marker_is_none_for_a_message_with_no_quoted_token() {
    assert_eq!(
        near_token_marker(
            "UNIQUE constraint failed: users.email",
            "INSERT INTO users..."
        ),
        None
    );
}

#[test]
fn near_token_marker_is_none_when_the_token_is_not_actually_in_the_query() {
    // Defensive: shouldn't happen against a real SQLite message, but
    // the query and message come from two different places, so there's
    // nothing stopping them from disagreeing.
    assert_eq!(
        near_token_marker(r#"near "XYZ": syntax error"#, "SELECT * FROM users"),
        None
    );
}

#[tokio::test]
async fn a_real_syntax_error_gets_a_line_and_caret_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let err = driver.execute("SELECT * FRO users").await.unwrap_err();

    let text = err.to_string();
    assert!(
        text.starts_with("near \"FRO\": syntax error"),
        "was: {text}"
    );
    assert!(text.contains("LINE 1:"), "was: {text}");
    assert!(text.contains('^'), "was: {text}");
}

#[tokio::test]
async fn an_error_with_no_quoted_token_still_reports_the_driver_s_own_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let err = driver
        .execute("SELECT * FROM no_such_table")
        .await
        .unwrap_err();

    assert!(err.to_string().contains("no such table"), "was: {err}");
}

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = SqliteDriver::new("test.db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
    let driver = SqliteDriver::new("test.db");
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
async fn a_transaction_through_query_engine_does_not_get_stuck_pending() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();
    let driver: std::sync::Arc<dyn QueryDriver> = std::sync::Arc::new(driver);
    let mut engine = QueryEngine::new(
        driver,
        SavedConnection {
            name: "test".to_string(),
            driver: "sqlite".to_string(),
            target: path.to_str().unwrap().to_string(),
        },
        Ok(Vec::new()),
    );

    async fn settle(engine: &mut QueryEngine) {
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            engine.tick();
            if !engine.is_pending() {
                return;
            }
        }
        panic!("query never settled");
    }

    engine.submit_query("BEGIN".to_string());
    settle(&mut engine).await;
    assert!(
        !engine.is_pending(),
        "BEGIN's outcome must be drained, not leave `pending` stuck"
    );

    engine.submit_query("INSERT INTO users VALUES (1)".to_string());
    settle(&mut engine).await;
    assert!(
        !engine.is_pending(),
        "the insert inside the transaction must also settle"
    );
    match engine.take_outcome() {
        Some(QueryOutcome::Completed {
            result: QueryResult::Affected { rows },
        }) => assert_eq!(rows, 1, "the insert must report the row it actually added"),
        Some(QueryOutcome::Completed { .. }) => panic!("expected an Affected result"),
        Some(QueryOutcome::Failed { error }) => panic!("insert failed: {error}"),
        None => panic!("no outcome to take"),
    }
}

#[tokio::test]
async fn connect_succeeds_for_a_new_sqlite_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");

    let mut driver = SqliteDriver::new(path.to_str().unwrap());

    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn connect_fails_when_parent_directory_does_not_exist() {
    let mut driver = SqliteDriver::new("/no/such/directory/test.db");

    let result = driver.connect().await;

    assert!(result.is_err());
}

#[tokio::test]
async fn list_schema_returns_created_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
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
async fn list_schema_marks_the_primary_key_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let key: Vec<&str> = schema[0]
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        key,
        vec!["id"],
        "without this the results grid can't build a WHERE clause"
    );
}

#[tokio::test]
async fn list_schema_reports_a_foreign_key_reference() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(driver.pool.as_ref().unwrap())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, \
         FOREIGN KEY (user_id) REFERENCES users (id))",
    )
    .execute(driver.pool.as_ref().unwrap())
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
async fn a_composite_primary_key_reports_every_one_of_its_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    sqlx::query(
        "CREATE TABLE memberships (user_id INTEGER, group_id INTEGER, role TEXT, \
         PRIMARY KEY (user_id, group_id))",
    )
    .execute(driver.pool.as_ref().unwrap())
    .await
    .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let key: Vec<&str> = schema[0]
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(key, vec!["user_id", "group_id"]);
}

#[tokio::test]
async fn execute_reports_affected_rows_for_a_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let created = driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();
    assert_eq!(created, QueryResult::Affected { rows: 0 });

    let inserted = driver
        .execute("INSERT INTO users VALUES (1), (2), (3)")
        .await
        .unwrap();
    assert_eq!(
        inserted,
        QueryResult::Affected { rows: 3 },
        "a write must report what it changed, not an empty table"
    );

    let deleted = driver
        .execute("DELETE FROM users WHERE id > 1")
        .await
        .unwrap();
    assert_eq!(deleted, QueryResult::Affected { rows: 2 });
}

#[tokio::test]
async fn a_result_bigger_than_the_cap_is_truncated_rather_than_fully_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    // A recursive CTE is an unbounded source without needing a huge
    // table on disk -- exactly the shape that used to be able to
    // exhaust memory.
    let over_the_cap = query_driver::MAX_ROWS + 500;
    let result = driver
        .execute(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {over_the_cap}) SELECT i FROM n"
        ))
        .await
        .unwrap();

    match result {
        QueryResult::Table {
            rows, truncated, ..
        } => {
            assert_eq!(rows.len(), query_driver::MAX_ROWS);
            assert!(truncated, "the user has to be told this isn't everything");
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

#[tokio::test]
async fn a_result_inside_the_cap_is_not_marked_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let result = driver
        .execute("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10) SELECT i FROM n")
        .await
        .unwrap();

    match result {
        QueryResult::Table {
            rows, truncated, ..
        } => {
            assert_eq!(rows.len(), 10);
            assert!(!truncated);
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

#[tokio::test]
async fn a_select_that_matches_nothing_is_still_an_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();

    let result = driver.execute("SELECT * FROM users").await.unwrap();

    // The distinction this whole change exists for: no rows found is
    // not the same shape as no rows returned by a write.
    assert!(matches!(result, QueryResult::Table { .. }));
}

#[tokio::test]
async fn execute_returns_columns_and_rows_for_a_select() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
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

#[tokio::test]
async fn a_fresh_driver_is_not_in_a_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    assert!(!driver.in_transaction());
}

#[tokio::test]
async fn begin_opens_a_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    driver.execute("BEGIN").await.unwrap();

    assert!(driver.in_transaction());
}

#[tokio::test]
async fn commit_closes_the_transaction_and_keeps_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();

    driver.execute("BEGIN").await.unwrap();
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
async fn rollback_closes_the_transaction_and_discards_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();

    driver.execute("BEGIN").await.unwrap();
    driver
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();
    driver.execute("ROLLBACK").await.unwrap();

    assert!(!driver.in_transaction());
    let result = driver.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        result,
        // Column names come from the first row streamed back; a select
        // with nothing to return never sees one, same pre-existing
        // behavior `a_select_that_matches_nothing_is_still_an_empty_table`
        // already covers -- not something this change alters.
        QueryResult::Table {
            columns: vec![],
            rows: vec![],
            truncated: false,
        },
        "the insert must not have survived the rollback"
    );
}

#[tokio::test]
async fn a_second_begin_while_already_in_a_transaction_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();
    driver.execute("BEGIN").await.unwrap();
    driver
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();

    // A second `BEGIN` must not open a fresh transaction and orphan the
    // one already holding the uncommitted insert.
    driver.execute("BEGIN").await.unwrap();
    driver.execute("COMMIT").await.unwrap();

    let result = driver.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        result,
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        },
        "the insert from before the second BEGIN must still have committed"
    );
}

#[tokio::test]
async fn commit_with_nothing_open_is_a_harmless_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let result = driver.execute("COMMIT").await;

    assert!(result.is_ok());
    assert!(!driver.in_transaction());
}

#[tokio::test]
async fn rollback_with_nothing_open_is_a_harmless_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let result = driver.execute("ROLLBACK").await;

    assert!(result.is_ok());
    assert!(!driver.in_transaction());
}

#[tokio::test]
async fn an_uncommitted_insert_is_invisible_to_another_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut writer = SqliteDriver::new(path.to_str().unwrap());
    writer.connect().await.unwrap();
    writer
        .execute("CREATE TABLE users (id INTEGER)")
        .await
        .unwrap();
    let mut reader = SqliteDriver::new(path.to_str().unwrap());
    reader.connect().await.unwrap();

    writer.execute("BEGIN").await.unwrap();
    writer
        .execute("INSERT INTO users VALUES (1)")
        .await
        .unwrap();

    // The whole point of holding one connection for the transaction:
    // a second, independent connection to the same file must not see
    // the insert until it's committed.
    let seen_before_commit = reader.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        seen_before_commit,
        // No column names for a zero-row result -- see the comment in
        // `rollback_closes_the_transaction_and_discards_the_change`.
        QueryResult::Table {
            columns: vec![],
            rows: vec![],
            truncated: false,
        }
    );

    writer.execute("COMMIT").await.unwrap();
    let seen_after_commit = reader.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        seen_after_commit,
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        }
    );
}

#[tokio::test]
async fn cancel_query_really_stops_a_running_statement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    let driver = std::sync::Arc::new(driver);

    // Unbounded (no `LIMIT`), and the final `count(*)` means SQLite
    // evaluates the whole thing inside one blocking `sqlite3_step()`
    // call with no row-by-row yield in between -- the exact case
    // `RawHandle` exists for: a cancel that only worked between rows
    // would hang here, since there is only one row, ever.
    let slow = std::sync::Arc::clone(&driver);
    let handle = tokio::spawn(async move {
        slow.execute(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) \
             SELECT count(*) FROM c",
        )
        .await
    });

    // Give the query time to actually start running before cancelling
    // it -- cancelling too early would race against the worker thread
    // even starting `sqlite3_step()`.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    driver.cancel_query().await.unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
        .await
        .expect("cancel_query should stop the runaway recursive CTE well under 10s, not hang")
        .unwrap();

    assert!(
        result.is_err(),
        "an interrupted query should report an error, not a result"
    );
}
