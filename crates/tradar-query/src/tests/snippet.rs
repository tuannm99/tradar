use super::*;

#[test]
fn crud_snippet_read_is_a_bounded_select() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Read, &[]),
        "SELECT * FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_create_lists_every_column_one_per_line_with_a_named_placeholder() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Create, &[]),
        "INSERT INTO \"users\" (\n  \"id\",\n  \"email\"\n) VALUES (\n  <id>,\n  <email>\n);"
    );
}

#[test]
fn crud_snippet_update_sets_non_key_columns_one_per_line_and_filters_by_the_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &[]),
        "UPDATE \"users\" SET\n  \"email\" = <email>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_delete_filters_by_the_primary_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &[]),
        "DELETE FROM \"users\" WHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_falls_back_gracefully_with_no_known_columns() {
    let entry = SchemaInfo::new("mystery");

    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Create, &[]),
        "INSERT INTO \"mystery\" (<column>) VALUES (<value>);"
    );
    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Update, &[]),
        "UPDATE \"mystery\" SET\n  <column> = <value>\nWHERE <condition>;"
    );
    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Delete, &[]),
        "DELETE FROM \"mystery\" WHERE <condition>;"
    );
}

#[test]
fn crud_snippet_create_and_update_wrap_one_column_per_line_so_no_line_runs_long() {
    let entry = SchemaInfo {
        name: "wide".to_string(),
        columns: (0..12)
            .map(|i| ColumnInfo::new(format!("column_number_{i}"), "TEXT"))
            .collect(),
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    for op in [CrudOp::Create, CrudOp::Update] {
        let snippet = build_crud_snippet(&entry, op, &[]);
        for line in snippet.lines() {
            assert!(
                line.chars().count() < 80,
                "line too long for {op:?}: {line:?}"
            );
        }
    }
}

#[test]
fn crud_snippet_update_with_no_primary_key_sets_every_column() {
    let entry = SchemaInfo {
        name: "logs".to_string(),
        columns: vec![ColumnInfo::new("message", "TEXT")],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Update, &[]),
        "UPDATE \"logs\" SET\n  \"message\" = <message>\nWHERE <condition>;"
    );
}

#[test]
fn crud_snippet_read_with_an_explicit_selection_lists_only_those_columns_in_table_order() {
    assert_eq!(
        build_crud_snippet(
            &users_table(),
            CrudOp::Read,
            &["email".to_string(), "id".to_string()]
        ),
        "SELECT \"id\", \"email\" FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_read_with_a_selection_matching_no_real_column_falls_back_to_select_star() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Read, &["ghost".to_string()]),
        "SELECT * FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_create_with_an_explicit_selection_lists_only_those_columns() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Create, &["email".to_string()]),
        "INSERT INTO \"users\" (\n  \"email\"\n) VALUES (\n  <email>\n);"
    );
}

#[test]
fn crud_snippet_update_with_an_explicit_selection_sets_only_those_columns() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &["email".to_string()]),
        "UPDATE \"users\" SET\n  \"email\" = <email>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_update_with_the_primary_key_explicitly_selected_sets_it_too() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &["id".to_string()]),
        "UPDATE \"users\" SET\n  \"id\" = <id>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_delete_can_filter_by_a_non_primary_key_column() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &["email".to_string()]),
        "DELETE FROM \"users\" WHERE \"email\" = <email>;"
    );
}

#[test]
fn crud_snippet_delete_with_a_selection_matching_no_real_column_falls_back_to_the_primary_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &["ghost".to_string()]),
        "DELETE FROM \"users\" WHERE \"id\" = <id>;"
    );
}
