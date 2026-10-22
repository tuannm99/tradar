use super::*;

#[test]
fn column_types_looks_up_each_column_by_name_in_the_source_table() {
    let schema = vec![users_table()];
    let columns = vec!["id".to_string(), "email".to_string()];

    assert_eq!(
        column_types(&columns, &schema, Some("users")),
        vec![Some("INTEGER".to_string()), Some("TEXT".to_string())]
    );
}

#[test]
fn column_types_is_none_for_a_column_the_source_table_does_not_have() {
    let schema = vec![users_table()];
    let columns = vec!["id".to_string(), "computed_alias".to_string()];

    assert_eq!(
        column_types(&columns, &schema, Some("users")),
        vec![Some("INTEGER".to_string()), None]
    );
}

#[test]
fn column_types_is_all_none_with_no_source_table() {
    let columns = vec!["id".to_string(), "email".to_string()];

    assert_eq!(
        column_types(&columns, &[users_table()], None),
        vec![None; 2]
    );
}

#[test]
fn column_types_is_all_none_when_the_table_is_not_in_schema() {
    let columns = vec!["id".to_string()];

    assert_eq!(
        column_types(&columns, &[users_table()], Some("ghost")),
        vec![None]
    );
}

#[test]
fn dot_after_an_alias_resolves_to_its_table() {
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u.na"),
        CompletionContext::TableColumns {
            table: "users".to_string()
        },
        "a partial column name after the dot doesn't change which table it resolves to"
    );
}

#[test]
fn dot_after_an_as_aliased_table_resolves_to_its_table() {
    assert_eq!(
        completion_context("SELECT * FROM users AS u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn dot_on_the_bare_table_name_resolves_to_itself() {
    assert_eq!(
        completion_context("SELECT * FROM users WHERE users."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn dot_after_a_join_alias_resolves_to_the_joined_table() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN orders o ON o.user_id = u.id WHERE o."),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn dot_on_an_unknown_alias_is_tried_as_a_schema_name_instead() {
    // `x` isn't a `FROM`/`JOIN` alias in this query -- but this
    // function has no schema to check `x` against either way, so it
    // reports the weaker `SchemaQualifiedTable` context rather than
    // giving up; `matches_in_context` is the one with schema access,
    // and falls back to the flat list itself if `x` isn't a real
    // schema/keyspace name.
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE x."),
        CompletionContext::SchemaQualifiedTable {
            schema: "x".to_string()
        }
    );
}

#[test]
fn dot_after_a_schema_name_reports_it_for_schema_qualified_lookup() {
    assert_eq!(
        completion_context("SELECT * FROM demo.us"),
        CompletionContext::SchemaQualifiedTable {
            schema: "demo".to_string()
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM demo."),
        CompletionContext::SchemaQualifiedTable {
            schema: "demo".to_string()
        }
    );
}

#[test]
fn dot_after_a_resolvable_alias_still_wins_over_schema_qualified() {
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn right_after_join_reports_the_tables_already_in_the_query() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN "),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string()]
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN or"),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string()]
        },
        "a partial table name being typed doesn't change the context"
    );
}

#[test]
fn known_tables_after_join_include_every_earlier_from_and_join() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN orders o ON o.user_id = u.id JOIN "),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string(), "orders".to_string()]
        }
    );
}

#[test]
fn plain_text_with_no_dot_or_join_has_no_special_context() {
    assert_eq!(
        completion_context("SELECT * FROM users WHERE na"),
        CompletionContext::None
    );
    assert_eq!(completion_context(""), CompletionContext::None);
}

#[test]
fn a_field_typed_inside_a_mongo_call_resolves_to_its_collection() {
    assert_eq!(
        completion_context("db.orders.find({status: \"open\", na"),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn an_empty_field_position_right_after_the_opening_brace_also_resolves() {
    assert_eq!(
        completion_context("db.orders.find({"),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn typing_the_mongo_method_name_itself_is_not_scoped_to_a_collection() {
    // Still inside the `db.orders.` chain, not yet past an opening
    // paren -- this is completing the method name (`find`, `findOne`,
    // ...), which isn't collection-specific, so it gets its own
    // context rather than being scoped like a field.
    assert_eq!(
        completion_context("db.orders.f"),
        CompletionContext::MongoMethod
    );
}

#[test]
fn a_bare_db_dot_with_nothing_typed_yet_is_also_the_method_context() {
    assert_eq!(
        completion_context("db.orders."),
        CompletionContext::MongoMethod
    );
}

#[test]
fn typing_the_collection_name_itself_is_not_the_method_context() {
    // Only one dot so far -- `resolve_alias` finds nothing for `db`
    // (no FROM/JOIN exists for Mongo text), same as always; collection
    // names are already ranked correctly in the flat list (they're
    // `CandidateKind::Table`), so this must not narrow to methods.
    // `SchemaQualifiedTable` rather than `None` since that failed
    // alias lookup is now tried as a schema name too -- `db` almost
    // never matches a real Mongo database name, so
    // `matches_in_context` falls back to the exact same flat list
    // `None` would have given; it is the fallback inside that context,
    // not this function, telling the two apart.
    assert_eq!(
        completion_context("db.ord"),
        CompletionContext::SchemaQualifiedTable {
            schema: "db".to_string()
        }
    );
}

#[test]
fn a_second_mongo_call_after_the_first_closes_resolves_to_its_own_collection() {
    assert_eq!(
        completion_context("db.orders.find({}); db.payments.find({na"),
        CompletionContext::TableColumns {
            table: "payments".to_string()
        }
    );
}

#[test]
fn plain_sql_nested_in_parens_is_not_mistaken_for_a_mongo_call() {
    assert_eq!(
        completion_context("SELECT COUNT(na"),
        CompletionContext::None
    );
}
