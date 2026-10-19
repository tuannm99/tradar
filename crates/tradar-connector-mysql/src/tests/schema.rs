use super::*;

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
