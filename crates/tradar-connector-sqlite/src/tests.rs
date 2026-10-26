use super::*;
use tradar_query_workbench::query_engine::QueryOutcome;

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

/// An entry's text with every `${name}` filled from the fixture below.
fn filled(text: &str) -> String {
    let value = |name: &str| match name {
        "table" => "x",
        "other_table" => "y",
        "new_table" => "nt",
        "column" => "c",
        "new_column" => "nc",
        "key_column" => "id",
        "join_column" => "id",
        "group_column" => "c",
        "value_column" => "n",
        "json_column" => "j",
        "json_key" => "a",
        "key" => "1",
        "id" => "1",
        "value" => "v",
        "name" => "x",
        "index_name" => "ix",
        "new_index" => "ix2",
        "view" => "vw",
        "new_view" => "vw2",
        "filter" => "1 = 1",
        "last_value" => "0",
        "path" => ":memory:",
        "a" => "a",
        "b" => "b",
        other => panic!("cheat sheet placeholder `{other}` has no value in this test"),
    };
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let end = rest[at..].find('}').expect("closed placeholder");
        out.push_str(value(&rest[at + 2..at + end]));
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
}

/// A fresh database with a small fixture, for one entry to run against.
async fn fixture() -> (tempfile::TempDir, SqliteDriver) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cheat.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    for sql in [
        "CREATE TABLE x (id INTEGER PRIMARY KEY, c TEXT, n INTEGER, j TEXT)",
        "CREATE TABLE y (id INTEGER PRIMARY KEY, c TEXT)",
        "INSERT INTO x (id, c, n, j) VALUES (1, '2024-01-01', 5, '{\"a\": 1}'), (2, '2024-01-02', 7, '{}'), (3, '2024-01-03', 9, NULL)",
        "INSERT INTO y (id, c) VALUES (1, 'one')",
        "CREATE INDEX ix ON x (n)",
        "CREATE VIEW vw AS SELECT * FROM x",
    ] {
        driver.execute(sql).await.unwrap();
    }
    (dir, driver)
}

#[tokio::test]
async fn every_cheatsheet_entry_runs() {
    for entry in crate::cheatsheet::ENTRIES {
        let sql = filled(entry.text);
        let (_dir, driver) = fixture().await;
        assert_eq!(
            driver.split_statements(&sql).len(),
            1,
            "`{}` is not one statement",
            entry.title
        );
        // Transaction control needs a transaction to be in -- it is parsed,
        // not run, here.
        if entry.category == "Transaction" {
            continue;
        }
        driver
            .execute(&sql)
            .await
            .unwrap_or_else(|e| panic!("`{}` failed: {e}\n{sql}", entry.title));
    }
}
