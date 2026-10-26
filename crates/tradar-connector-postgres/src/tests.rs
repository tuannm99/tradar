use super::*;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

mod connect;
mod errors;
mod execute;
mod schema;
mod snippets;
mod transactions;

#[test]
fn cheatsheet_is_well_formed() {
    let problems = tradar_query::cheatsheet_problems(crate::cheatsheet::ENTRIES);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn every_cheatsheet_entry_is_one_statement() {
    let driver = PostgresDriver::new("postgres://u:p@127.0.0.1:1/db");
    for entry in crate::cheatsheet::ENTRIES {
        assert_eq!(
            driver.split_statements(entry.text).len(),
            1,
            "`{}` is not one statement",
            entry.title
        );
    }
}
