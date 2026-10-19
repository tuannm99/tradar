use super::*;

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
