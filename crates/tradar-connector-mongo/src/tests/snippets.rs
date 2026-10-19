use super::*;

#[test]
fn crud_snippet_covers_all_four_ops() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("db.users.find({})".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Create, &[]),
        Some("db.users.insertOne({})".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Update, &[]),
        Some("db.users.updateOne({_id: ObjectId(\"<id>\")}, {$set: {}})".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Delete, &[]),
        Some("db.users.deleteOne({_id: ObjectId(\"<id>\")})".to_string())
    );
}

#[test]
fn crud_snippet_create_with_a_real_schema_lists_every_field_with_a_named_placeholder() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &users_with_fields(),
            tradar_core::action::CrudOp::Create,
            &[]
        ),
        Some("db.users.insertOne({\n  _id: ObjectId(\"<id>\"),\n  name: <value>\n})".to_string())
    );
}

#[test]
fn crud_snippet_create_with_an_explicit_selection_lists_only_those_fields() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &users_with_fields(),
            tradar_core::action::CrudOp::Create,
            &["name".to_string()]
        ),
        Some("db.users.insertOne({name: <value>})".to_string())
    );
}

#[test]
fn crud_snippet_create_nests_dotted_fields_into_a_real_object_instead_of_a_flat_key() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let snippet = driver
        .crud_snippet(
            &orders_with_a_nested_address(),
            tradar_core::action::CrudOp::Create,
            &[],
        )
        .unwrap();

    assert_eq!(
        snippet,
        "db.orders.insertOne({\n  _id: ObjectId(\"<id>\"),\n  address: {\n    city: <value>,\n    zip: <value>\n  }\n})"
    );
    // Never a literal key with a dot in it -- that's the bug this
    // guards against.
    assert!(!snippet.contains("\"address.city\""));
    assert!(!snippet.contains("\"address.zip\""));
}

#[test]
fn crud_snippet_create_with_only_one_nested_field_selected_stays_a_single_line() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &orders_with_a_nested_address(),
            tradar_core::action::CrudOp::Create,
            &["address.city".to_string()]
        ),
        Some("db.orders.insertOne({address: {city: <value>}})".to_string())
    );
}

#[test]
fn crud_snippet_update_and_delete_still_quote_a_dotted_field_flat() {
    // Unlike Create, Update's `$set` and Delete's filter are read by
    // Mongo as path expressions, so the flat quoted form
    // (`object_literal`, unchanged) is already correct there -- this
    // guards against `nested_insert_literal`'s fix ever leaking into
    // these two by mistake.
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &orders_with_a_nested_address(),
            tradar_core::action::CrudOp::Update,
            &["address.city".to_string()]
        ),
        Some(
            "db.orders.updateOne({_id: ObjectId(\"<id>\")}, {$set: {\"address.city\": <value>}})"
                .to_string()
        )
    );
    assert_eq!(
        driver.crud_snippet(
            &orders_with_a_nested_address(),
            tradar_core::action::CrudOp::Delete,
            &["address.city".to_string()]
        ),
        Some("db.orders.deleteOne({\"address.city\": <value>})".to_string())
    );
}

#[test]
fn crud_snippet_read_with_a_selection_projects_only_those_fields() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &users_with_fields(),
            tradar_core::action::CrudOp::Read,
            &["name".to_string()]
        ),
        Some("db.users.find({}, {name: 1})".to_string())
    );
}

#[test]
fn crud_snippet_update_defaults_to_setting_non_id_fields() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &users_with_fields(),
            tradar_core::action::CrudOp::Update,
            &[]
        ),
        Some("db.users.updateOne({_id: ObjectId(\"<id>\")}, {$set: {name: <value>}})".to_string())
    );
}

#[test]
fn crud_snippet_delete_can_filter_by_a_non_id_field() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.crud_snippet(
            &users_with_fields(),
            tradar_core::action::CrudOp::Delete,
            &["name".to_string()]
        ),
        Some("db.users.deleteOne({name: <value>})".to_string())
    );
}

#[test]
fn mongo_field_key_quotes_a_dotted_nested_path() {
    assert_eq!(mongo_field_key("address.city"), "\"address.city\"");
    assert_eq!(mongo_field_key("name"), "name");
}

#[test]
fn edit_source_accepts_a_plain_find() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.edit_source("db.users.find({})"),
        Some("users".to_string())
    );
    assert_eq!(
        driver.edit_source("db.users.find({\"status\": \"active\"})"),
        Some("users".to_string())
    );
}

#[test]
fn edit_source_refuses_aggregate_and_writes() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(driver.edit_source("db.users.aggregate([])"), None);
    assert_eq!(
        driver.edit_source("db.users.insertOne({name: \"a\"})"),
        None
    );
    assert_eq!(driver.edit_source("use somedb"), None);
    assert_eq!(driver.edit_source("show dbs"), None);
}

#[test]
fn edit_key_columns_is_always_id() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");
    assert_eq!(
        driver.edit_key_columns("users"),
        Some(vec!["_id".to_string()])
    );
}

#[test]
fn edit_sql_set_values_puts_every_field_in_one_set() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let edit = query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("ObjectId(\"507f1f77bcf86cd799439011\")"),
        change: query_driver::RowChange::SetValues(vec![
            ("name".to_string(), "Ada".to_string()),
            ("age".to_string(), "36".to_string()),
        ]),
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "db.users.updateOne({_id: ObjectId(\"507f1f77bcf86cd799439011\")}, \
             {$set: {name: \"Ada\", age: 36}})"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_set_value_filters_by_id_and_infers_the_value_s_type() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let edit = query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("ObjectId(\"507f1f77bcf86cd799439011\")"),
        change: query_driver::RowChange::SetValue {
            column: "name".to_string(),
            value: "Ada".to_string(),
        },
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "db.users.updateOne({_id: ObjectId(\"507f1f77bcf86cd799439011\")}, \
             {$set: {name: \"Ada\"}})"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_infers_numbers_bools_and_null_as_bare_literals() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");
    let edit_for = |value: &str| query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("ObjectId(\"507f1f77bcf86cd799439011\")"),
        change: query_driver::RowChange::SetValue {
            column: "age".to_string(),
            value: value.to_string(),
        },
    };

    assert!(
        driver
            .edit_sql(&edit_for("42"))
            .unwrap()
            .contains("{$set: {age: 42}}")
    );
    assert!(
        driver
            .edit_sql(&edit_for("true"))
            .unwrap()
            .contains("{$set: {age: true}}")
    );
    assert!(
        driver
            .edit_sql(&edit_for("null"))
            .unwrap()
            .contains("{$set: {age: null}}")
    );
}

#[test]
fn edit_sql_quotes_a_plain_string_id_that_is_not_an_objectid() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let edit = query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("abc123"),
        change: query_driver::RowChange::DeleteRow,
    };

    // A bare "abc123" would read as a JS identifier, not a string --
    // must come back quoted or the generated statement wouldn't parse
    // as the filter it's meant to be.
    assert_eq!(
        driver.edit_sql(&edit),
        Some("db.users.deleteOne({_id: \"abc123\"})".to_string())
    );
}

#[test]
fn edit_sql_delete_row_uses_the_key_as_the_filter() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let edit = query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("ObjectId(\"507f1f77bcf86cd799439011\")"),
        change: query_driver::RowChange::DeleteRow,
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some("db.users.deleteOne({_id: ObjectId(\"507f1f77bcf86cd799439011\")})".to_string())
    );
}

#[test]
fn edit_sql_quotes_a_dotted_nested_field_being_set() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let edit = query_driver::RowEdit {
        table: "users".to_string(),
        key: id_key("ObjectId(\"507f1f77bcf86cd799439011\")"),
        change: query_driver::RowChange::SetValue {
            column: "address.city".to_string(),
            value: "Hanoi".to_string(),
        },
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "db.users.updateOne({_id: ObjectId(\"507f1f77bcf86cd799439011\")}, \
             {$set: {\"address.city\": \"Hanoi\"}})"
                .to_string()
        )
    );
}

#[tokio::test]
async fn find_chain_validates_hint_and_explain_before_touching_the_server() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "find".to_string(),
        args: vec![],
    };
    let call = |name: &str, args: Vec<serde_json::Value>| MethodCall {
        name: name.to_string(),
        args,
    };
    // explain / count must be last.
    for early in ["explain", "count"] {
        let err = run_method(
            &collection,
            &primary,
            &[
                call(early, vec![]),
                call("limit", vec![serde_json::json!(1)]),
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("must be the last call"), "{early}: {err}");
    }
    // hint takes an index name or a key document, nothing else.
    let err = run_method(
        &collection,
        &primary,
        &[call("hint", vec![serde_json::json!(3)])],
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("hint"), "{err}");
    // Without a database there is nowhere to send explain.
    let err = run_method(&collection, &primary, &[call("explain", vec![])])
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("needs a database"), "{err}");
    // hint does not reshape documents, so a hinted find stays editable.
    assert!(
        MongoDriver::new("mongodb://x")
            .edit_source("db.users.find({}).hint({a: 1}).sort({a: 1})")
            .is_some()
    );
}

#[test]
fn edit_source_still_accepts_a_chained_find_with_sort_limit_skip() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(
        driver.edit_source(r#"db.users.find({}).sort({"name": 1}).limit(10).skip(5)"#),
        Some("users".to_string())
    );
}

#[test]
fn edit_source_refuses_a_chained_count() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    assert_eq!(driver.edit_source(r#"db.users.find({}).count()"#), None);
}
