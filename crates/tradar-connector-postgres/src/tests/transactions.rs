use super::*;

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
            columns: vec!["id".to_string()],
            rows: vec![],
            truncated: false,
        }
    );

    writer.execute("ROLLBACK").await.unwrap();
    let seen_after_rollback = reader.execute("SELECT * FROM users").await.unwrap();
    assert_eq!(
        seen_after_rollback,
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
            truncated: false,
        },
        "a rolled-back insert must never become visible to anyone"
    );
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
