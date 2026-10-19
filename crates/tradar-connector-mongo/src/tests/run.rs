use super::*;

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
