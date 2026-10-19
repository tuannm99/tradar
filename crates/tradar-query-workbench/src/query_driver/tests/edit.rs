use super::*;

#[test]
fn an_update_names_the_column_the_table_and_the_key() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "7")]),
        change: RowChange::SetValue {
            column: "name".to_string(),
            value: "Ada".to_string(),
        },
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"name\" = 'Ada' WHERE \"id\" = '7'"
    );
}

#[test]
fn an_insert_lists_columns_and_literals_and_leaves_null_unquoted() {
    let sql = build_sql_insert(
        "public.users",
        &key(&[("name", "O'Brien"), ("nickname", "NULL")]),
    );

    assert_eq!(
        sql,
        "INSERT INTO \"public\".\"users\" (\"name\", \"nickname\") VALUES ('O''Brien', NULL)"
    );
}

#[test]
fn several_columns_of_a_row_become_one_update() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "7")]),
        change: RowChange::SetValues(key(&[("name", "O'Brien"), ("nickname", "NULL")])),
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"name\" = 'O''Brien', \"nickname\" = NULL WHERE \"id\" = '7'"
    );
}

#[test]
fn a_delete_uses_every_column_of_a_composite_key() {
    let sql = build_sql_edit(&RowEdit {
        table: "memberships".to_string(),
        key: key(&[("user_id", "1"), ("group_id", "2")]),
        change: RowChange::DeleteRow,
    });

    assert_eq!(
        sql,
        "DELETE FROM \"memberships\" WHERE \"user_id\" = '1' AND \"group_id\" = '2'"
    );
}

#[test]
fn a_quote_in_a_value_is_escaped_rather_than_ending_the_literal() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::SetValue {
            column: "name".to_string(),
            value: "O'Brien".to_string(),
        },
    });

    assert_eq!(
        sql, "UPDATE \"users\" SET \"name\" = 'O''Brien' WHERE \"id\" = '1'",
        "an unescaped quote here would be an injection, not a typo"
    );
}

#[test]
fn a_null_is_written_unquoted_the_way_the_grid_shows_it() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::SetValue {
            column: "nickname".to_string(),
            value: "NULL".to_string(),
        },
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"nickname\" = NULL WHERE \"id\" = '1'"
    );
}

#[test]
fn a_schema_qualified_table_stays_two_identifiers() {
    let sql = build_sql_edit(&RowEdit {
        table: "public.users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::DeleteRow,
    });

    assert_eq!(sql, "DELETE FROM \"public\".\"users\" WHERE \"id\" = '1'");
}

#[test]
fn add_column_builds_a_nullable_column_with_no_default_by_default() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
        type_name: "TEXT".to_string(),
        nullable: true,
        default: None,
    });

    assert_eq!(sql, "ALTER TABLE \"users\" ADD COLUMN \"nickname\" TEXT");
}

#[test]
fn add_column_appends_not_null_and_a_raw_default_expression() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "credits".to_string(),
        type_name: "INTEGER".to_string(),
        nullable: false,
        default: Some("0".to_string()),
    });

    assert_eq!(
        sql,
        "ALTER TABLE \"users\" ADD COLUMN \"credits\" INTEGER NOT NULL DEFAULT 0"
    );
}

#[test]
fn add_column_treats_a_blank_default_as_no_default() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
        type_name: "TEXT".to_string(),
        nullable: true,
        default: Some("   ".to_string()),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" ADD COLUMN \"nickname\" TEXT");
}

#[test]
fn drop_column_quotes_table_and_column() {
    let sql = build_table_ddl(&TableDesignerOp::DropColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" DROP COLUMN \"nickname\"");
}

#[test]
fn rename_table_builds_rename_to() {
    let sql = build_table_ddl(&TableDesignerOp::RenameTable {
        table: "users".to_string(),
        new_name: "accounts".to_string(),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" RENAME TO \"accounts\"");
}

#[test]
fn create_table_lists_one_column_per_line_with_not_null_where_asked() {
    let sql = build_table_ddl(&TableDesignerOp::CreateTable {
        table: "accounts".to_string(),
        columns: vec![
            NewColumn {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
                nullable: false,
                primary_key: true,
            },
            NewColumn {
                name: "email".to_string(),
                type_name: "TEXT".to_string(),
                nullable: false,
                primary_key: false,
            },
            NewColumn {
                name: "nickname".to_string(),
                type_name: "TEXT".to_string(),
                nullable: true,
                primary_key: false,
            },
        ],
    });

    assert_eq!(
        sql,
        "CREATE TABLE \"accounts\" (\n  \"id\" INTEGER PRIMARY KEY,\n  \"email\" TEXT NOT NULL,\n  \"nickname\" TEXT\n)"
    );
}

#[test]
fn create_table_never_states_not_null_redundantly_on_a_primary_key() {
    let sql = build_table_ddl(&TableDesignerOp::CreateTable {
        table: "t".to_string(),
        columns: vec![NewColumn {
            name: "id".to_string(),
            type_name: "INTEGER".to_string(),
            nullable: false,
            primary_key: true,
        }],
    });

    assert!(sql.contains("PRIMARY KEY"));
    assert!(
        !sql.contains("NOT NULL"),
        "PRIMARY KEY already implies it: {sql}"
    );
}
