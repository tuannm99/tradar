use super::*;

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
        // No rows, but the header still comes through (`describe`) -- see
        // `an_empty_select_still_reports_its_column_names`.
        QueryResult::Table {
            columns: vec!["id".to_string()],
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
        // Zero rows, but still the header -- see the comment in
        // `rollback_closes_the_transaction_and_discards_the_change`.
        QueryResult::Table {
            columns: vec!["id".to_string()],
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
