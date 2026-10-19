use super::*;

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
