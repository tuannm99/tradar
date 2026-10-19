use super::*;

#[test]
fn crud_snippet_delegates_to_the_shared_sql_builder() {
    let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");
    let entry = SchemaInfo::new("users");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("SELECT * FROM \"users\" LIMIT 100;".to_string())
    );
}

#[test]
fn table_ddl_and_migrations_are_not_supported_at_v1() {
    // Unlike Postgres, this driver doesn't override `table_ddl` or
    // `supports_migrations` -- both features generate dialect-specific
    // DDL/tracking SQL that hasn't been built for MySQL/MariaDB yet,
    // so they fall back to the `QueryDriver` trait's own defaults.
    let driver = MySqlDriver::new("mysql://user:pass@127.0.0.1:1/db");

    assert_eq!(
        driver.table_ddl(&query_driver::TableDesignerOp::RenameTable {
            table: "users".to_string(),
            new_name: "accounts".to_string(),
        }),
        None
    );
    assert!(!driver.supports_migrations());
}
