use super::*;

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = SqliteDriver::new("test.db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
    let driver = SqliteDriver::new("test.db");
    let entry = SchemaInfo {
        name: "users".to_string(),
        columns: vec![
            ColumnInfo::new("id", "INTEGER"),
            ColumnInfo::new("email", "TEXT"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    assert_eq!(
        driver.crud_snippet(
            &entry,
            tradar_core::action::CrudOp::Read,
            &["email".to_string()]
        ),
        Some("SELECT \"email\" FROM \"users\" LIMIT 100;".to_string())
    );
}
