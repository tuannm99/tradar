use super::*;

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
