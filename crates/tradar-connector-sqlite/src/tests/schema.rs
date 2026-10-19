use super::*;

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
