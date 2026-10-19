use super::*;

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
