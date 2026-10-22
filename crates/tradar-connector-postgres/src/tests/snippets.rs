use super::*;

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn table_ddl_delegates_to_the_shared_ddl_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

    assert_eq!(
        driver.table_ddl(&query_driver::TableDesignerOp::RenameTable {
            table: "users".to_string(),
            new_name: "accounts".to_string(),
        }),
        Some("ALTER TABLE \"users\" RENAME TO \"accounts\"".to_string())
    );
}

#[test]
fn supports_migrations_is_true() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");

    assert!(driver.supports_migrations());
}

#[test]
fn crud_snippet_forwards_a_column_selection_to_the_shared_sql_builder() {
    let driver = PostgresDriver::new("postgres://user:pass@127.0.0.1:1/db");
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
        driver.crud_snippet(&entry, tradar_query::CrudOp::Read, &["email".to_string()]),
        Some("SELECT \"email\" FROM \"users\" LIMIT 100;".to_string())
    );
}
