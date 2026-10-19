use super::*;

#[tokio::test]
async fn connect_succeeds_for_a_running_mongo() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));

    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
}

#[tokio::test]
async fn connect_succeeds_when_the_uri_names_no_database() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}"));

    let result = driver.connect().await;

    assert!(result.is_ok(), "connect failed: {:?}", result.err());
    // Falls back to mongosh's own default rather than staying unset.
    assert_eq!(driver.current_db_name(), "test");
}

#[tokio::test]
async fn connect_auto_selects_the_uri_s_default_database() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/shop"));

    driver.connect().await.unwrap();

    assert_eq!(driver.current_db_name(), "shop");
}

#[tokio::test]
async fn use_switches_which_database_subsequent_queries_target() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/first"));
    driver.connect().await.unwrap();

    driver.execute("use second").await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    assert_eq!(driver.current_db_name(), "second");
    let in_second = driver
        .database()
        .collection::<Document>("users")
        .find_one(Document::new())
        .await
        .unwrap();
    assert!(
        in_second.is_some(),
        "insert should have landed in \"second\""
    );
    let in_first = mongodb::Client::with_uri_str(&format!("mongodb://127.0.0.1:{port}/first"))
        .await
        .unwrap()
        .database("first")
        .collection::<Document>("users")
        .find_one(Document::new())
        .await
        .unwrap();
    assert!(
        in_first.is_none(),
        "insert should not have landed in \"first\""
    );
}

#[tokio::test]
async fn show_dbs_lists_a_database_that_has_data_in_it() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/shop"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.orders.insertOne({"item": "pen"})"#)
        .await
        .unwrap();

    let result = driver.execute("show dbs").await.unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    assert!(
        docs.iter().any(|d| d["name"] == "shop"),
        "dbs were: {docs:?}"
    );
}

#[tokio::test]
async fn list_schema_samples_every_collection_concurrently() {
    // `list_schema` fires one `find_one` per collection via
    // `join_all` rather than one after another (see its doc comment)
    // -- with several collections, this checks that concurrent
    // completion order doesn't scramble which sampled columns end up
    // matched to which collection name.
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();
    driver
        .execute(r#"db.orders.insertOne({"total": 42})"#)
        .await
        .unwrap();
    driver
        .execute(r#"db.tags.insertOne({"label": "vip"})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    fn names_of(entry: &SchemaInfo) -> Vec<&str> {
        entry.columns.iter().map(|c| c.name.as_str()).collect()
    }
    let users = schema.iter().find(|s| s.name == "users").unwrap();
    assert!(names_of(users).contains(&"name"), "{:?}", names_of(users));
    let orders = schema.iter().find(|s| s.name == "orders").unwrap();
    assert!(
        names_of(orders).contains(&"total"),
        "{:?}",
        names_of(orders)
    );
    let tags = schema.iter().find(|s| s.name == "tags").unwrap();
    assert!(names_of(tags).contains(&"label"), "{:?}", names_of(tags));
}

#[tokio::test]
async fn list_schema_returns_created_collections() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert!(
        schema.iter().any(|entry| entry.name == "users"),
        "schema was: {:?}",
        schema.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn list_schema_reports_the_fields_of_a_sampled_document() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada", "age": 36})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    let names: Vec<&str> = users.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"_id"), "columns were: {names:?}");
    assert!(names.contains(&"name"), "columns were: {names:?}");
    assert!(names.contains(&"age"), "columns were: {names:?}");
}

#[tokio::test]
async fn list_schema_marks_underscore_id_as_the_primary_key() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    let key: Vec<&str> = users
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(key, vec!["_id"]);
}

#[tokio::test]
async fn list_schema_marks_a_field_covered_by_a_real_index() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada", "age": 36})"#)
        .await
        .unwrap();
    // `execute()`'s parser doesn't understand `createIndex` (it's not
    // one of the methods `docs/architecture.md` scoped this connector
    // to) -- created directly through the driver crate instead, same
    // as any other test-only setup step outside the shell subset.
    let client = mongodb::Client::with_uri_str(format!("mongodb://127.0.0.1:{port}/test"))
        .await
        .unwrap();
    client
        .database("test")
        .collection::<Document>("users")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(mongodb::bson::doc! { "name": 1 })
                .build(),
        )
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    let indexed: Vec<&str> = users
        .columns
        .iter()
        .filter(|c| c.indexed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(indexed, vec!["name"]);
    // `_id` has the implicit default index, but it's already flagged
    // via `primary_key` -- not worth a second, redundant marker.
    assert!(
        !users
            .columns
            .iter()
            .find(|c| c.name == "_id")
            .unwrap()
            .indexed
    );
    assert!(
        !users
            .columns
            .iter()
            .find(|c| c.name == "age")
            .unwrap()
            .indexed
    );
}

#[tokio::test]
async fn an_empty_collection_reports_no_columns_rather_than_failing() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    // Creates the collection without inserting a document to sample.
    driver.database().create_collection("empty").await.unwrap();

    let schema = driver.list_schema().await.unwrap();

    let empty = schema.iter().find(|s| s.name == "empty").unwrap();
    assert!(empty.columns.is_empty());
}

#[tokio::test]
async fn list_schema_covers_every_database_grouped_by_it() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/first"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();
    driver.execute("use second").await.unwrap();
    driver
        .execute(r#"db.orders.insertOne({"total": 42})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    let users = schema.iter().find(|s| s.name == "users").unwrap();
    assert_eq!(users.schema.as_deref(), Some("first"));
    let orders = schema.iter().find(|s| s.name == "orders").unwrap();
    assert_eq!(orders.schema.as_deref(), Some("second"));
    assert!(
        schema
            .iter()
            .all(|s| !matches!(s.schema.as_deref(), Some("admin" | "local" | "config"))),
        "system databases must not show up in the navigator: {:?}",
        schema.iter().map(|s| &s.schema).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn list_schema_qualifies_only_a_collection_name_that_collides_across_databases() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/first"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();
    driver.execute("use second").await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Bob"})"#)
        .await
        .unwrap();
    driver
        .execute(r#"db.orders.insertOne({"total": 42})"#)
        .await
        .unwrap();

    let schema = driver.list_schema().await.unwrap();

    assert!(
        schema.iter().any(|s| s.name == "first.users"),
        "schema was: {:?}",
        schema.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert!(schema.iter().any(|s| s.name == "second.users"));
    assert!(
        schema.iter().any(|s| s.name == "orders"),
        "a non-colliding name must stay bare"
    );
}

#[tokio::test]
async fn ping_succeeds_against_a_reachable_mongo() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    assert!(driver.ping().await.is_ok());
}
