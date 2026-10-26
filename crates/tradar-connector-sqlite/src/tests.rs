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

/// An entry's text with every `${name}` filled with `x`.
fn filled(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        match rest[at..].find('}') {
            Some(end) => rest = &rest[at + end + 1..],
            None => break,
        }
        out.push('x');
    }
    out.push_str(rest);
    out
}

#[tokio::test]
async fn every_cheatsheet_entry_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cheat.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();
    driver
        .execute("CREATE TABLE x (x TEXT, id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    driver
        .execute("INSERT INTO x (x) VALUES ('2024-01-01')")
        .await
        .unwrap();

    for entry in crate::cheatsheet::ENTRIES {
        let sql = filled(entry.text);
        assert_eq!(
            driver.split_statements(&sql).len(),
            1,
            "`{}` is not one statement",
            entry.title
        );
        driver
            .execute(&sql)
            .await
            .unwrap_or_else(|e| panic!("`{}` failed: {e}\n{sql}", entry.title));
    }
}
