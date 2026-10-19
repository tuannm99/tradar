use super::*;

#[tokio::test]
async fn list_schema_returns_created_tables() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
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
async fn list_schema_reports_a_foreign_key_reference() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER \
         REFERENCES users (id))",
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

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    let id = users.columns.iter().find(|c| c.name == "id").unwrap();
    assert!(
        id.foreign_key.is_none(),
        "users.id is a primary key, not itself a foreign key"
    );
}

#[tokio::test]
async fn list_schema_groups_multiple_schemas_and_hides_system_ones() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA reporting")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE reporting.summaries (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(users.schema.as_deref(), Some("public"));
    let summaries = schema.iter().find(|s| s.name == "summaries").unwrap();
    assert_eq!(summaries.schema.as_deref(), Some("reporting"));
    assert!(
        schema.iter().all(|s| !matches!(
            s.schema.as_deref(),
            Some("pg_catalog" | "information_schema")
        )),
        "system schemas must not show up in the navigator: {:?}",
        schema.iter().map(|s| &s.schema).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn list_schema_reports_views_functions_and_procedures_by_kind() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
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
    sqlx::query("CREATE FUNCTION greet() RETURNS int AS $$ SELECT 1 $$ LANGUAGE sql")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE PROCEDURE do_nothing() LANGUAGE sql AS $$ SELECT 1 $$")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let table = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(table.object_kind.as_deref(), Some("table"));
    let view = schema.iter().find(|s| s.name == "user_ids").unwrap();
    assert_eq!(view.object_kind.as_deref(), Some("view"));
    let function = schema.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(function.object_kind.as_deref(), Some("function"));
    let procedure = schema.iter().find(|s| s.name == "do_nothing").unwrap();
    assert_eq!(procedure.object_kind.as_deref(), Some("procedure"));
}

#[tokio::test]
async fn list_schema_qualifies_only_a_table_name_that_collides_across_schemas() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let conn_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut driver = PostgresDriver::new(&conn_string);
    driver.connect().await.unwrap();
    let pool = driver.pool.as_ref().unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA reporting")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE reporting.users (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert!(
        schema.iter().any(|s| s.name == "public.users"),
        "schema was: {:?}",
        schema.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert!(schema.iter().any(|s| s.name == "reporting.users"));
    assert!(
        schema.iter().any(|s| s.name == "orders"),
        "a non-colliding name must stay bare"
    );
}
