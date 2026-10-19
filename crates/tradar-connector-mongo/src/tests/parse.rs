use super::*;

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
