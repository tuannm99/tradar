#[test]
fn show_helpers_parse_with_aliases_and_a_trailing_semicolon() {
    assert!(matches!(
        parse_shell_command("show dbs").unwrap(),
        ShellCommand::ShowDbs
    ));
    assert!(matches!(
        parse_shell_command("show databases;").unwrap(),
        ShellCommand::ShowDbs
    ));
    assert!(matches!(
        parse_shell_command("show collections").unwrap(),
        ShellCommand::ShowCollections
    ));
    assert!(matches!(
        parse_shell_command("  show tables ; ").unwrap(),
        ShellCommand::ShowCollections
    ));
    assert!(matches!(
        parse_shell_command("show users").unwrap(),
        ShellCommand::ShowUsers
    ));
    assert!(matches!(
        parse_shell_command("show roles // who").unwrap(),
        ShellCommand::ShowRoles
    ));
    assert!(matches!(
        parse_shell_command("db").unwrap(),
        ShellCommand::DbName
    ));
    assert!(matches!(
        parse_shell_command("db;").unwrap(),
        ShellCommand::DbName
    ));
    match parse_shell_command("use \"shop\";").unwrap() {
        ShellCommand::Use(name) => assert_eq!(name, "shop"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_show_is_a_clear_error_not_a_query_parse_error() {
    let err = parse_shell_command("show nothing").unwrap_err().to_string();
    assert!(err.contains("show collections"), "{err}");
    assert!(matches!(
        parse_shell_command("show profile;").unwrap(),
        ShellCommand::ShowProfile
    ));
    assert!(parse_shell_command("show").is_err());
}

#[test]
fn get_sibling_db_retargets_one_statement_at_another_database() {
    let (db, rest) = split_sibling_db("db.getSiblingDB(\"shop\").getCollectionNames();").unwrap();
    assert_eq!(db.as_deref(), Some("shop"));
    assert_eq!(rest, "db.getCollectionNames()");
    let (db, rest) = split_sibling_db("db.getSiblingDB('shop').orders.find({})").unwrap();
    assert_eq!(db.as_deref(), Some("shop"));
    assert_eq!(parse_shell_query(&rest).unwrap().collection, "orders");
    // An ordinary statement is left alone.
    let (db, rest) = split_sibling_db("db.users.find()").unwrap();
    assert!(db.is_none());
    assert_eq!(rest, "db.users.find()");
    assert!(split_sibling_db("db.getSiblingDB(\"\").stats()").is_err());
    assert!(split_sibling_db("db.getSiblingDB(\"x\")").is_err());
    assert!(
        MongoDriver::new("mongodb://x")
            .edit_source("db.getSiblingDB(\"shop\").orders.find({})")
            .is_none()
    );
}

#[test]
fn database_level_calls_are_not_mistaken_for_a_collection() {
    match parse_shell_command("db.getCollectionNames();").unwrap() {
        ShellCommand::DbCall(call) => {
            assert_eq!(call.name, "getCollectionNames");
            assert!(call.args.is_empty());
        }
        other => panic!("{other:?}"),
    }
    match parse_shell_command("db.runCommand({ping: 1})").unwrap() {
        ShellCommand::DbCall(call) => {
            assert_eq!(call.name, "runCommand");
            assert_eq!(call.args, vec![serde_json::json!({"ping": 1})]);
        }
        other => panic!("{other:?}"),
    }
    // A collection that merely starts like a db method is still a collection.
    match parse_shell_command("db.stats.find({})").unwrap() {
        ShellCommand::Query(q) => assert_eq!(q.collection, "stats"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn get_collection_addresses_a_collection_by_name() {
    match parse_shell_command("db.getCollection(\"my-coll\").find({a: 1})").unwrap() {
        ShellCommand::Query(q) => {
            assert_eq!(q.collection, "my-coll");
            assert_eq!(q.calls[0].name, "find");
            assert_eq!(q.calls[0].args, vec![serde_json::json!({"a": 1})]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn distinct_and_index_helpers_parse_as_collection_methods() {
    let q = parse_shell_query("db.users.distinct(\"country\", {\"age\": {\"$gt\": 1}});").unwrap();
    assert_eq!(q.collection, "users");
    assert_eq!(q.primary().name, "distinct");
    assert_eq!(q.primary().args.len(), 2);
    let q = parse_shell_query("db.users.getIndexes()").unwrap();
    assert_eq!(q.primary().name, "getIndexes");
    assert!(
        MongoDriver::new("mongodb://x")
            .edit_source("db.users.getIndexes()")
            .is_none()
    );
}

#[test]
fn a_sort_document_keeps_its_key_order() {
    // A compound index serves `{a: 1, b: -1}` but not `{b: -1, a: 1}`:
    // the order the user typed must reach the server.
    let q = parse_shell_query("db.t.find({}).sort({zeta: 1, alpha: -1, mid: 1})").unwrap();
    let doc = json_to_document(q.chain()[0].args[0].clone()).unwrap();
    let keys: Vec<_> = doc.keys().map(String::as_str).collect();
    assert_eq!(keys, ["zeta", "alpha", "mid"]);
}

#[test]
fn a_query_with_a_trailing_semicolon_parses() {
    match parse_shell_command("db.users.find({});").unwrap() {
        ShellCommand::Query(q) => assert_eq!(q.collection, "users"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn show_and_bare_db_lines_start_their_own_statements() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");
    let statements = driver.split_statements("show collections\ndb.users.find({})\ndb\nshow users");
    let texts: Vec<_> = statements.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        ["show collections", "db.users.find({})", "db", "show users"]
    );
}

#[test]
fn a_commented_out_line_inside_the_call_is_ignored() {
    let parsed =
        parse_shell_query("db.users.find({\n  // _id: ObjectId(\"6ad4e4cc4c3ea1739d8b502f\")\n})")
            .unwrap();

    assert_eq!(parsed.collection, "users");
    assert_eq!(parsed.calls[0].name, "find");
    assert_eq!(parsed.calls[0].args, vec![serde_json::json!({})]);
}

#[test]
fn comments_are_stripped_but_comment_markers_inside_strings_are_kept() {
    assert_eq!(
        strip_js_comments("a // gone\nb /* gone\ntoo */ c"),
        "a \nb \n  c"
    );
    let parsed =
        parse_shell_query("db.links.find({url: \"http://x/y\", /* skip */ n: 1}) // tail").unwrap();
    assert_eq!(
        parsed.calls[0].args,
        vec![serde_json::json!({"url": "http://x/y", "n": 1})]
    );
    // An apostrophe in a comment does not open a string.
    assert!(parse_shell_query("db.a.find({}) // don't").is_ok());
}

#[test]
fn relax_json_turns_shell_constructors_into_extended_json() {
    use serde_json::json;
    let parse = |text: &str| -> serde_json::Value {
        serde_json::from_str(&relax_json(text)).unwrap_or_else(|e| panic!("{text}: {e}"))
    };
    assert_eq!(
        parse(r#"{"_id": ObjectId("507f1f77bcf86cd799439011")}"#),
        json!({"_id": {"$oid": "507f1f77bcf86cd799439011"}})
    );
    assert_eq!(
        parse("{_id: ObjectId('507f1f77bcf86cd799439011'), n: NumberInt(5)}"),
        json!({"_id": {"$oid": "507f1f77bcf86cd799439011"}, "n": 5})
    );
    assert_eq!(
        parse(r#"{at: ISODate("2026-08-04T00:46:57Z"), big: NumberLong(9007199254740993)}"#),
        json!({"at": {"$date": "2026-08-04T00:46:57Z"}, "big": {"$numberLong": "9007199254740993"}})
    );
    // Something that merely looks like a constructor inside a string is data.
    assert_eq!(
        parse(r#"{"note": "ObjectId(\"x\")"}"#),
        json!({"note": "ObjectId(\"x\")"})
    );
}

#[test]
fn relax_json_accepts_what_mongosh_accepts() {
    let parse = |text: &str| -> serde_json::Value {
        serde_json::from_str(&relax_json(text)).unwrap_or_else(|e| panic!("{text}: {e}"))
    };
    use serde_json::json;
    assert_eq!(
        parse("{name: 'ann', age: 30}"),
        json!({"name": "ann", "age": 30})
    );
    assert_eq!(parse("{ $set: { age: 31 } }"), json!({"$set": {"age": 31}}));
    assert_eq!(
        parse("{age: {$gt: 18, $lt: 65}}"),
        json!({"age": {"$gt": 18, "$lt": 65}})
    );
    assert_eq!(parse("{tags: ['a', 'b',], }"), json!({"tags": ["a", "b"]}));
    assert_eq!(
        parse("{ok: true, none: null}"),
        json!({"ok": true, "none": null})
    );
    // A double quote inside a single-quoted string, and an escaped single quote.
    assert_eq!(
        parse(r#"{q: 'say "hi"', r: 'it\'s'}"#),
        json!({"q": "say \"hi\"", "r": "it's"})
    );
    // A word that merely looks like a key inside a string is left alone.
    assert_eq!(parse("{note: 'a, b: c'}"), json!({"note": "a, b: c"}));
}

#[test]
fn relax_json_leaves_strict_json_untouched() {
    for strict in [
        r#"{"name": "ann", "age": 30}"#,
        r#"{"a": [1, 2, {"b": null}], "c": "x, y: z"}"#,
        r#"{"escaped": "she said \"hi\""}"#,
        "[]",
        "42",
    ] {
        assert_eq!(relax_json(strict), strict);
    }
}

#[test]
fn single_quoted_strings_do_not_confuse_the_argument_splitter() {
    // The comma and the brace live inside single quotes: still one
    // argument each, and the closing paren is the real one.
    let text = "find({name: 'a, b}'}, {age: 1}) rest";
    let close = find_matching_close_paren(&text["find(".len()..]).unwrap();
    let args = split_top_level_args(&text["find(".len()..][..close]).unwrap();
    assert_eq!(args, vec!["{name: 'a, b}'}", " {age: 1}"]);
}

use super::*;
use crate::convert::*;
use crate::relax::*;
use crate::run::*;
use crate::shell::*;
use crate::snippets::*;
use mongodb::bson::Document;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};

#[test]
fn split_statements_treats_a_multi_line_call_as_one_statement() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let statements = driver.split_statements("db.orders.find({\n  status: \"open\"\n})");

    assert_eq!(statements.len(), 1, "statements were: {statements:?}");
    assert_eq!(
        statements[0].text,
        "db.orders.find({\n  status: \"open\"\n})"
    );
}

#[test]
fn split_statements_still_splits_two_separate_single_line_calls() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let statements = driver.split_statements("db.orders.find({})\ndb.users.find({})");

    assert_eq!(statements.len(), 2, "statements were: {statements:?}");
    assert_eq!(statements[0].text, "db.orders.find({})");
    assert_eq!(statements[1].text, "db.users.find({})");
}

#[test]
fn split_statements_keeps_a_multi_line_call_separate_from_the_next_one() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let statements =
        driver.split_statements("db.orders.find({\n  status: \"open\"\n})\ndb.users.find({})");

    assert_eq!(statements.len(), 2, "statements were: {statements:?}");
    assert_eq!(
        statements[0].text,
        "db.orders.find({\n  status: \"open\"\n})"
    );
    assert_eq!(statements[1].text, "db.users.find({})");
}

#[test]
fn split_statements_recognizes_use_and_show_dbs_as_their_own_statements() {
    let driver = MongoDriver::new("mongodb://127.0.0.1:1/db");

    let statements = driver.split_statements("use reporting\nshow dbs");

    assert_eq!(statements.len(), 2, "statements were: {statements:?}");
    assert_eq!(statements[0].text, "use reporting");
    assert_eq!(statements[1].text, "show dbs");
}

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

fn users_with_fields() -> SchemaInfo {
    let mut column = ColumnInfo::new("_id", "ObjectId");
    column.primary_key = true;
    SchemaInfo {
        name: "users".to_string(),
        columns: vec![column, ColumnInfo::new("name", "String")],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
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

fn orders_with_a_nested_address() -> SchemaInfo {
    let mut id = ColumnInfo::new("_id", "ObjectId");
    id.primary_key = true;
    SchemaInfo {
        name: "orders".to_string(),
        columns: vec![
            id,
            ColumnInfo::new("address.city", "String"),
            ColumnInfo::new("address.zip", "String"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
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

fn id_key(id: &str) -> Vec<(String, String)> {
    vec![("_id".to_string(), id.to_string())]
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

/// A `Collection` handle against an address nothing listens on --
/// `Client::with_uri_str`/`with_options` only parse the URI and set up
/// internal state, they don't connect (MongoDB drivers connect lazily,
/// on the first real operation) -- so this is safe to build without
/// Docker or network access, as long as the test never actually awaits
/// an operation on it. Every `run_method` test below relies on that:
/// each one exercises a validation error that `run_method` returns
/// *before* it ever awaits the driver call that would need a real
/// server.
async fn unreachable_collection() -> mongodb::Collection<Document> {
    let client = mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/test")
        .await
        .unwrap();
    client.database("test").collection::<Document>("users")
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

#[tokio::test]
async fn run_method_rejects_a_chain_on_a_method_that_does_not_support_one() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "insertOne".to_string(),
        args: vec![serde_json::json!({"name": "Ada"})],
    };
    let chain = vec![MethodCall {
        name: "sort".to_string(),
        args: vec![],
    }];

    let result = run_method(&collection, &primary, &chain).await;

    assert!(result.is_err(), "insertOne must refuse a chained call");
}

#[tokio::test]
async fn run_method_rejects_count_not_at_the_end_of_the_chain() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "find".to_string(),
        args: vec![],
    };
    let chain = vec![
        MethodCall {
            name: "count".to_string(),
            args: vec![],
        },
        MethodCall {
            name: "limit".to_string(),
            args: vec![serde_json::json!(1)],
        },
    ];

    let result = run_method(&collection, &primary, &chain).await;

    assert!(result.is_err(), "count() must be the chain's last call");
}

#[tokio::test]
async fn run_method_rejects_an_unrecognized_chain_call() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "find".to_string(),
        args: vec![],
    };
    let chain = vec![MethodCall {
        name: "forEach".to_string(),
        args: vec![],
    }];

    let result = run_method(&collection, &primary, &chain).await;

    assert!(result.is_err(), "forEach is not a real JS engine call");
}

#[tokio::test]
async fn run_method_rejects_a_non_numeric_limit_or_skip() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "find".to_string(),
        args: vec![],
    };

    let bad_limit = run_method(
        &collection,
        &primary,
        &[MethodCall {
            name: "limit".to_string(),
            args: vec![serde_json::json!("ten")],
        }],
    )
    .await;
    assert!(bad_limit.is_err());

    let bad_skip = run_method(
        &collection,
        &primary,
        &[MethodCall {
            name: "skip".to_string(),
            args: vec![serde_json::json!("five")],
        }],
    )
    .await;
    assert!(bad_skip.is_err());
}

#[tokio::test]
async fn run_method_rejects_sort_with_the_wrong_argument_count() {
    let collection = unreachable_collection().await;
    let primary = MethodCall {
        name: "find".to_string(),
        args: vec![],
    };
    let chain = vec![MethodCall {
        name: "sort".to_string(),
        args: vec![],
    }];

    let result = run_method(&collection, &primary, &chain).await;

    assert!(result.is_err(), "sort() with no argument must be rejected");
}

#[test]
fn simplify_extjson_formats_an_oid_as_a_mongosh_style_literal() {
    let mut value = serde_json::json!({"$oid": "507f1f77bcf86cd799439011"});

    simplify_extjson(&mut value);

    assert_eq!(
        value,
        serde_json::json!("ObjectId(\"507f1f77bcf86cd799439011\")")
    );
}

#[test]
fn simplify_extjson_formats_a_date_as_a_mongosh_style_literal() {
    let mut value = serde_json::json!({"$date": "2026-08-16T00:00:00Z"});

    simplify_extjson(&mut value);

    assert_eq!(
        value,
        serde_json::json!("ISODate(\"2026-08-16T00:00:00Z\")")
    );
}

#[test]
fn simplify_extjson_keeps_an_objectid_distinguishable_from_a_plain_string_field() {
    // The whole point: a real string field that happens to contain
    // the exact same hex text as an id must not render identically
    // to the id itself.
    let mut value = serde_json::json!({
        "_id": {"$oid": "507f1f77bcf86cd799439011"},
        "note": "507f1f77bcf86cd799439011",
    });

    simplify_extjson(&mut value);

    assert_eq!(
        value["_id"],
        serde_json::json!("ObjectId(\"507f1f77bcf86cd799439011\")")
    );
    assert_eq!(value["note"], serde_json::json!("507f1f77bcf86cd799439011"));
    assert_ne!(value["_id"], value["note"]);
}

#[test]
fn simplify_extjson_formats_nested_and_array_occurrences() {
    let mut value = serde_json::json!({
        "_id": {"$oid": "507f1f77bcf86cd799439011"},
        "name": "Ada",
        "createdBy": {"id": {"$oid": "507f191e810c19729de860ea"}},
        "history": [{"$date": "2026-08-01T00:00:00Z"}, {"$date": "2026-08-02T00:00:00Z"}],
    });

    simplify_extjson(&mut value);

    assert_eq!(
        value,
        serde_json::json!({
            "_id": "ObjectId(\"507f1f77bcf86cd799439011\")",
            "name": "Ada",
            "createdBy": {"id": "ObjectId(\"507f191e810c19729de860ea\")"},
            "history": ["ISODate(\"2026-08-01T00:00:00Z\")", "ISODate(\"2026-08-02T00:00:00Z\")"],
        })
    );
}

#[test]
fn simplify_extjson_leaves_other_wrapper_shapes_alone() {
    // `$numberLong`/`$numberDecimal`/`$binary`/... are not touched --
    // see the function's own doc comment for why only `$oid`/`$date`
    // are safe to treat as purely cosmetic.
    let mut value = serde_json::json!({"$numberLong": "9223372036854775807"});

    simplify_extjson(&mut value);

    assert_eq!(
        value,
        serde_json::json!({"$numberLong": "9223372036854775807"})
    );
}

#[test]
fn simplify_extjson_leaves_a_plain_object_with_a_dollar_key_and_other_keys_alone() {
    // A one-key check, not "has a $oid/$date key anywhere" -- a real
    // document field could coincidentally be named that.
    let mut value = serde_json::json!({"$oid": "not-actually-an-id", "other": 1});

    simplify_extjson(&mut value);

    assert_eq!(
        value,
        serde_json::json!({"$oid": "not-actually-an-id", "other": 1})
    );
}

#[test]
fn parses_a_find_with_a_filter() {
    let parsed = parse_shell_query(r#"db.users.find({"active": true})"#).unwrap();

    assert_eq!(parsed.collection, "users");
    assert_eq!(parsed.calls.len(), 1);
    assert_eq!(parsed.primary().name, "find");
    assert_eq!(
        parsed.primary().args,
        vec![serde_json::json!({"active": true})]
    );
}

#[test]
fn parses_multiple_top_level_arguments() {
    let parsed =
        parse_shell_query(r#"db.users.updateOne({"_id": 1}, {"$set": {"name": "Ada"}})"#).unwrap();

    assert_eq!(parsed.collection, "users");
    assert_eq!(parsed.primary().name, "updateOne");
    assert_eq!(
        parsed.primary().args,
        vec![
            serde_json::json!({"_id": 1}),
            serde_json::json!({"$set": {"name": "Ada"}})
        ]
    );
}

#[test]
fn parses_a_method_call_with_no_arguments() {
    let parsed = parse_shell_query("db.users.find()").unwrap();

    assert_eq!(parsed.primary().args, Vec::<serde_json::Value>::new());
}

#[test]
fn rejects_input_that_does_not_start_with_db() {
    assert!(parse_shell_query("users.find({})").is_err());
}

#[test]
fn rejects_malformed_json_arguments() {
    assert!(parse_shell_query("db.users.find({not json})").is_err());
}

#[test]
fn parses_a_chained_find_into_a_primary_call_plus_the_chain() {
    let parsed =
        parse_shell_query(r#"db.users.find({}).sort({"name": 1}).limit(10).skip(5)"#).unwrap();

    assert_eq!(parsed.collection, "users");
    assert_eq!(parsed.primary().name, "find");
    let chain: Vec<&str> = parsed.chain().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(chain, vec!["sort", "limit", "skip"]);
    assert_eq!(parsed.chain()[0].args, vec![serde_json::json!({"name": 1})]);
    assert_eq!(parsed.chain()[1].args, vec![serde_json::json!(10)]);
    assert_eq!(parsed.chain()[2].args, vec![serde_json::json!(5)]);
}

#[test]
fn parses_a_trailing_count_with_no_arguments() {
    let parsed = parse_shell_query(r#"db.users.find({}).count()"#).unwrap();

    assert_eq!(parsed.chain().len(), 1);
    assert_eq!(parsed.chain()[0].name, "count");
    assert_eq!(parsed.chain()[0].args, Vec::<serde_json::Value>::new());
}

#[test]
fn rejects_a_chain_call_missing_its_own_closing_paren() {
    assert!(parse_shell_query(r#"db.users.find({}).sort({"name": 1}"#).is_err());
}

#[test]
fn a_brace_inside_a_json_string_argument_does_not_confuse_the_call_boundary() {
    // The chain call boundary is found by `find_matching_close_paren`,
    // which has to skip over string contents -- a naive brace-depth
    // scan would miscount on a value like this.
    let parsed = parse_shell_query(r#"db.users.find({"note": "a } b"}).limit(1)"#).unwrap();

    assert_eq!(
        parsed.primary().args,
        vec![serde_json::json!({"note": "a } b"})]
    );
    assert_eq!(parsed.chain()[0].name, "limit");
}

fn doc_from_json(json: serde_json::Value) -> Document {
    json_to_document(json).unwrap()
}

#[test]
fn flatten_document_reports_a_column_per_top_level_field() {
    let doc = doc_from_json(serde_json::json!({
        "_id": 1,
        "name": "Ada",
        "active": true,
    }));
    let mut columns = Vec::new();

    flatten_document("", &doc, &mut columns);

    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["_id", "name", "active"]);
}

#[test]
fn flatten_document_marks_only_id_as_the_primary_key() {
    let doc = doc_from_json(serde_json::json!({"_id": 1, "name": "Ada"}));
    let mut columns = Vec::new();

    flatten_document("", &doc, &mut columns);

    let key: Vec<&str> = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(key, vec!["_id"]);
}

#[test]
fn flatten_document_turns_a_nested_object_into_dotted_paths() {
    let doc = doc_from_json(serde_json::json!({
        "address": {"city": "Hanoi", "zip": "100000"},
    }));
    let mut columns = Vec::new();

    flatten_document("", &doc, &mut columns);

    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["address.city", "address.zip"]);
}

#[test]
fn flatten_document_does_not_expand_array_elements() {
    let doc = doc_from_json(serde_json::json!({"tags": ["a", "b", "c"]}));
    let mut columns = Vec::new();

    flatten_document("", &doc, &mut columns);

    assert_eq!(
        columns.len(),
        1,
        "one field for the array, not one per element"
    );
    assert_eq!(columns[0].name, "tags");
    assert_eq!(columns[0].type_name, "Array");
}

use testcontainers_modules::mongo::Mongo;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

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

#[tokio::test]
async fn execute_rejects_an_unsupported_method() {
    let container = Mongo::new().start().await.unwrap();
    let port = container.get_host_port_ipv4(27017).await.unwrap();
    let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();

    let result = driver.execute("db.users.drop()").await;

    assert!(result.is_err());
}
