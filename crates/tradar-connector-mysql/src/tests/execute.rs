use super::*;

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
