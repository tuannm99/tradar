use super::*;

#[tokio::test]
async fn a_multi_line_find_call_actually_executes_correctly() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada", "status": "open"})"#)
        .await
        .unwrap();

    let statements = driver.split_statements("db.users.find({\n  \"status\": \"open\"\n})");
    assert_eq!(statements.len(), 1);
    let result = driver.execute(&statements[0].text).await.unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => assert_eq!(docs.len(), 1),
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_insert_one_then_find_round_trips_a_document() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();
    let result = driver
        .execute(r#"db.users.find({"name": "Ada"})"#)
        .await
        .unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0]["name"], "Ada");
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[tokio::test]
async fn find_reports_the_id_as_an_objectid_literal_not_a_wrapped_oid() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    let result = driver
        .execute(r#"db.users.find({"name": "Ada"})"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    let id = docs[0]["_id"].as_str().expect("_id must be a string");
    assert!(
        id.starts_with("ObjectId(\"") && id.ends_with("\")"),
        "was: {id:?}"
    );
}

/// End-to-end proof this feature actually works against a real
/// MongoDB, not just that it builds the right-looking string: a
/// document's real, `ObjectId`-typed `_id` (as displayed by `find`)
/// round-trips straight back into a working filter with no reverse
/// parsing, and the update/delete this builds actually mutates the
/// collection.
#[tokio::test]
async fn edit_sql_round_trips_a_real_update_and_delete_against_a_found_document() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada", "age": 30})"#)
        .await
        .unwrap();

    assert_eq!(
        driver.edit_source(r#"db.users.find({})"#),
        Some("users".to_string())
    );
    let result = driver
        .execute(r#"db.users.find({"name": "Ada"})"#)
        .await
        .unwrap();
    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    let id = docs[0]["_id"].as_str().unwrap().to_string();

    let update_sql = driver
        .edit_sql(&query_driver::RowEdit {
            table: "users".to_string(),
            key: id_key(&id),
            change: query_driver::RowChange::SetValue {
                column: "age".to_string(),
                value: "31".to_string(),
            },
        })
        .expect("a plain find must be editable");
    driver.execute(&update_sql).await.unwrap();

    let after_update = driver
        .execute(&format!(r#"db.users.find({{"_id": {id}}})"#))
        .await
        .unwrap();
    let QueryResult::Documents { items: docs, .. } = after_update else {
        panic!("expected Documents");
    };
    assert_eq!(docs[0]["age"], 31, "doc after update: {:?}", docs[0]);

    let delete_sql = driver
        .edit_sql(&query_driver::RowEdit {
            table: "users".to_string(),
            key: id_key(&id),
            change: query_driver::RowChange::DeleteRow,
        })
        .unwrap();
    driver.execute(&delete_sql).await.unwrap();

    let after_delete = driver
        .execute(&format!(r#"db.users.find({{"_id": {id}}})"#))
        .await
        .unwrap();
    let QueryResult::Documents { items: docs, .. } = after_delete else {
        panic!("expected Documents");
    };
    assert!(docs.is_empty(), "doc should be deleted: {docs:?}");
}

#[tokio::test]
async fn insert_one_s_own_inserted_id_is_also_an_objectid_literal() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    let result = driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    let id = docs[0]["insertedId"]
        .as_str()
        .expect("insertedId must be a string");
    assert!(
        id.starts_with("ObjectId(\"") && id.ends_with("\")"),
        "was: {id:?}"
    );
}

#[tokio::test]
async fn execute_aggregate_accepts_the_real_shell_pipeline_array_syntax() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(
            r#"db.orders.insertMany([{"item": "pen", "qty": 2}, {"item": "pencil", "qty": 5}])"#,
        )
        .await
        .unwrap();

    let result = driver
        .execute(r#"db.orders.aggregate([{"$match": {"item": "pen"}}])"#)
        .await
        .unwrap();

    match result {
        QueryResult::Documents { items: docs, .. } => {
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0]["item"], "pen");
            assert_eq!(docs[0]["qty"], 2);
        }
        other => panic!("expected Documents, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_aggregate_caps_at_max_rows_and_reports_truncated() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    let docs: Vec<serde_json::Value> = (0..query_driver::MAX_ROWS + 1)
        .map(|i| serde_json::json!({"n": i}))
        .collect();
    driver
        .execute(&format!(
            "db.items.insertMany({})",
            serde_json::to_string(&docs).unwrap()
        ))
        .await
        .unwrap();

    let result = driver
        .execute(r#"db.items.aggregate([{"$match": {}}])"#)
        .await
        .unwrap();

    let QueryResult::Documents { items, truncated } = result else {
        panic!("expected Documents");
    };
    assert_eq!(items.len(), query_driver::MAX_ROWS);
    assert!(
        truncated,
        "more matches than query_driver::MAX_ROWS must be reported as truncated"
    );
}

#[tokio::test]
async fn execute_find_with_a_projection_returns_only_the_selected_fields() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada", "age": 30})"#)
        .await
        .unwrap();

    let result = driver
        .execute(r#"db.users.find({}, {"name": 1, "_id": 0})"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Ada");
    assert!(
        docs[0].get("age").is_none(),
        "age should have been excluded by the projection: {:?}",
        docs[0]
    );
}

#[tokio::test]
async fn execute_find_preserves_the_document_s_own_field_order() {
    // BSON documents are themselves ordered; `serde_json::Value::Object`
    // is a `BTreeMap` (alphabetical) unless `preserve_order` is on (see
    // the comment on this crate's `serde_json` dependency) -- without
    // it, `into_relaxed_extjson()` would silently resort `z`/`a`/`m`
    // into `a`/`m`/`z` before `tradar-server` ever gets a chance to
    // report field order back to a client.
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"z": 1, "a": 2, "m": 3})"#)
        .await
        .unwrap();

    let result = driver.execute(r#"db.users.find({})"#).await.unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    let keys: Vec<&str> = docs[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec!["_id", "z", "a", "m"],
        "insertion order must survive, not come back alphabetical"
    );
}

#[tokio::test]
async fn execute_find_chained_sort_limit_skip_orders_and_paginates() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    for n in [3, 1, 2] {
        driver
            .execute(&format!(r#"db.users.insertOne({{"n": {n}}})"#))
            .await
            .unwrap();
    }

    // Ascending by n, skip the smallest, limit to 1 -- should land on
    // exactly the middle value.
    let result = driver
        .execute(r#"db.users.find({}).sort({"n": 1}).skip(1).limit(1)"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    assert_eq!(docs.len(), 1, "docs were: {docs:?}");
    assert_eq!(docs[0]["n"], 2);
}

#[tokio::test]
async fn explain_hint_and_count_go_through_the_server_and_its_index() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    for n in 1..=5 {
        driver
            .execute(&format!(r#"db.users.insertOne({{"n": {n}}})"#))
            .await
            .unwrap();
    }
    driver
        .execute(
            r#"db.runCommand({createIndexes: "users", indexes: [{key: {n: 1}, name: "n_1"}]})"#,
        )
        .await
        .unwrap();

    // The plan names the index scan.
    let QueryResult::Documents { items, .. } = driver
        .execute(r#"db.users.find({n: {$gt: 1}}).sort({n: 1}).explain()"#)
        .await
        .unwrap()
    else {
        panic!("expected Documents");
    };
    assert!(items[0].to_string().contains("IXSCAN"), "{}", items[0]);

    // count honours the filter, skip and limit server-side.
    let count = |q: &'static str| {
        let driver = &driver;
        async move {
            let QueryResult::Documents { items, .. } = driver.execute(q).await.unwrap() else {
                panic!("expected Documents");
            };
            items[0]["count"].as_i64().unwrap()
        }
    };
    assert_eq!(count(r#"db.users.find({n: {$gt: 1}}).count()"#).await, 4);
    assert_eq!(
        count(r#"db.users.find({}).hint("n_1").skip(1).limit(3).count()"#).await,
        3
    );
    assert_eq!(count(r#"db.users.find({}).limit(0).count()"#).await, 5);
}

#[tokio::test]
async fn execute_find_chained_count_reports_the_matching_count_not_documents() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    for _ in 0..3 {
        driver
            .execute(r#"db.users.insertOne({"active": true})"#)
            .await
            .unwrap();
    }

    let result = driver
        .execute(r#"db.users.find({"active": true}).count()"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    assert_eq!(docs, vec![serde_json::json!({"count": 3})]);
}

#[tokio::test]
async fn execute_find_caps_at_max_rows_and_reports_truncated() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    let docs: Vec<serde_json::Value> = (0..query_driver::MAX_ROWS + 1)
        .map(|i| serde_json::json!({"n": i}))
        .collect();
    driver
        .execute(&format!(
            "db.items.insertMany({})",
            serde_json::to_string(&docs).unwrap()
        ))
        .await
        .unwrap();

    let result = driver.execute("db.items.find({})").await.unwrap();

    let QueryResult::Documents { items, truncated } = result else {
        panic!("expected Documents");
    };
    assert_eq!(items.len(), query_driver::MAX_ROWS);
    assert!(
        truncated,
        "more documents than query_driver::MAX_ROWS must be reported as truncated"
    );
}

#[tokio::test]
async fn execute_find_chained_count_is_accurate_even_past_max_rows() {
    // `.count()` must still report the real total -- it must not be
    // capped the same way the document list itself is, since the whole
    // point of `.count()` is an accurate number, not a sample.
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    let docs: Vec<serde_json::Value> = (0..query_driver::MAX_ROWS + 1)
        .map(|i| serde_json::json!({"n": i}))
        .collect();
    driver
        .execute(&format!(
            "db.items.insertMany({})",
            serde_json::to_string(&docs).unwrap()
        ))
        .await
        .unwrap();

    let result = driver.execute("db.items.find({}).count()").await.unwrap();

    let QueryResult::Documents { items, truncated } = result else {
        panic!("expected Documents");
    };
    assert_eq!(
        items,
        vec![serde_json::json!({"count": query_driver::MAX_ROWS + 1})]
    );
    assert!(
        !truncated,
        "a count report is one document, never itself truncated"
    );
}

#[tokio::test]
async fn execute_find_under_the_cap_is_not_truncated() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.items.insertOne({"n": 1})"#)
        .await
        .unwrap();

    let result = driver.execute("db.items.find({})").await.unwrap();

    let QueryResult::Documents { items, truncated } = result else {
        panic!("expected Documents");
    };
    assert_eq!(items.len(), 1);
    assert!(!truncated);
}

#[tokio::test]
async fn execute_find_one_returns_a_single_document_or_none() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"name": "Ada"})"#)
        .await
        .unwrap();

    let found = driver
        .execute(r#"db.users.findOne({"name": "Ada"})"#)
        .await
        .unwrap();
    let QueryResult::Documents { items: docs, .. } = found else {
        panic!("expected Documents");
    };
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["name"], "Ada");

    let missing = driver
        .execute(r#"db.users.findOne({"name": "nobody"})"#)
        .await
        .unwrap();
    let QueryResult::Documents { items: docs, .. } = missing else {
        panic!("expected Documents");
    };
    assert!(docs.is_empty(), "docs were: {docs:?}");
}

#[tokio::test]
async fn execute_runs_the_console_helpers_against_a_real_mongo() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    driver
        .execute(r#"db.users.insertOne({"n": 1});"#)
        .await
        .unwrap();
    driver
        .execute(r#"db.orders.insertOne({"n": 2})"#)
        .await
        .unwrap();

    let docs = |result: QueryResult| match result {
        QueryResult::Documents { items, .. } => items,
        _ => panic!("expected Documents"),
    };
    assert_eq!(
        docs(driver.execute("show collections").await.unwrap()),
        vec![
            serde_json::json!({"name": "orders"}),
            serde_json::json!({"name": "users"})
        ]
    );
    assert_eq!(
        docs(driver.execute("db.getCollectionNames();").await.unwrap()),
        vec![serde_json::json!(["orders", "users"])]
    );
    assert_eq!(
        docs(driver.execute("db").await.unwrap()),
        vec![serde_json::json!("test")]
    );
    assert_eq!(
        docs(driver.execute("db.getName()").await.unwrap()),
        vec![serde_json::json!("test")]
    );
    let ping = docs(driver.execute("db.runCommand({ping: 1})").await.unwrap());
    assert_eq!(ping[0]["ok"], serde_json::json!(1.0));
    assert!(!docs(driver.execute("show dbs;").await.unwrap()).is_empty());
    assert!(docs(driver.execute("db.version()").await.unwrap())[0].is_string());
    assert_eq!(
        docs(
            driver
                .execute(r#"db.getCollection("users").countDocuments({})"#)
                .await
                .unwrap()
        ),
        vec![serde_json::json!({"count": 1})]
    );
}

#[tokio::test]
async fn execute_count_documents_reports_the_matching_count() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
    for active in [true, true, false] {
        driver
            .execute(&format!(r#"db.users.insertOne({{"active": {active}}})"#))
            .await
            .unwrap();
    }

    let result = driver
        .execute(r#"db.users.countDocuments({"active": true})"#)
        .await
        .unwrap();

    let QueryResult::Documents { items: docs, .. } = result else {
        panic!("expected Documents");
    };
    assert_eq!(docs, vec![serde_json::json!({"count": 2})]);
}

#[tokio::test]
async fn execute_rejects_an_unsupported_method() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    let result = driver.execute("db.users.drop()").await;

    assert!(result.is_err());
}

/// Entries that need something a stock single server does not have.
const CHEAT_MAY_FAIL: &[&str] = &[
    // Not a replica set.
    "Replica set status",
    // Needs a text index on the field.
    "Full-text search (needs a text index)",
];

fn cheat_value(name: &str, title: &str) -> Option<String> {
    Some(
        match name {
            "collection" => "c_main",
            "other_collection" => "c_other",
            "new_collection" => "c_new",
            "database" => "test",
            "field" if title.starts_with("Create") => "u",
            "field" => "status",
            "other_field" if title == "Join another collection ($lookup)" => "status",
            "other_field" => "owner",
            "sub_field" => "city",
            "number_field" => "n",
            "array_field" => "tags",
            "new_field" => "renamed",
            "group" => "status",
            "value" => "open",
            "other_value" => "x",
            "a" => "open",
            "b" => "closed",
            "pattern" => "op",
            "date" => "2024-01-01",
            "id" => "507f1f77bcf86cd799439011",
            "index_name" if title.starts_with("Drop") => "ix_status",
            "index_name" => "ix_new",
            _ => return None,
        }
        .to_string(),
    )
}

/// Every cheat sheet entry, run against a real MongoDB (Docker).
#[tokio::test]
async fn every_cheatsheet_entry_runs_on_a_real_server() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in crate::cheatsheet::ENTRIES {
        for collection in ["c_main", "c_other", "c_new"] {
            let _ = driver
                .execute(&format!("db.runCommand({{drop: \"{collection}\"}})"))
                .await;
        }
        for setup in [
            r#"db.c_main.insertMany([{"status": "open", "owner": "ann", "n": 5, "u": 1, "tags": ["x", "y"], "address": {"city": "c1"}}, {"status": "closed", "owner": "bob", "n": 50, "u": 2, "tags": ["x"], "address": {"city": "c2"}}, {"status": "open", "owner": "cid", "n": 500, "u": 3, "tags": [], "address": {"city": "c1"}}])"#,
            r#"db.c_other.insertMany([{"status": "open", "label": "L"}])"#,
            r#"db.runCommand({createIndexes: "c_main", indexes: [{key: {status: 1}, name: "ix_status"}]})"#,
        ] {
            driver
                .execute(setup)
                .await
                .unwrap_or_else(|e| panic!("fixture `{setup}`: {e}"));
        }

        let text =
            match tradar_query::fill_placeholders(entry.text, |n| cheat_value(n, entry.title)) {
                Ok(text) => text,
                Err(name) => {
                    failures.push(format!("`{}`: no value for `${{{name}}}`", entry.title));
                    continue;
                }
            };
        ran += 1;
        if let Err(e) = driver.execute(&text).await
            && !CHEAT_MAY_FAIL.contains(&entry.title)
        {
            failures.push(format!("`{}`: {e}\n{text}", entry.title));
        }
    }
    assert!(ran > 60, "only {ran} entries ran");
    assert!(
        failures.is_empty(),
        "{} of {ran} cheat sheet entries failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
