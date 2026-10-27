//! Every cheat sheet entry, run against a real PostgreSQL (Docker). A failing
//! entry is listed with its error; the entries that need something a stock
//! container does not have are named in `MAY_FAIL`.
use super::*;
use testcontainers_modules::testcontainers::ImageExt;

/// Needs an extension or a feature the stock image does not enable.
const MAY_FAIL: &[&str] = &["Slowest statements (needs pg_stat_statements)"];

/// Would end this session, or needs more than one statement of state.
fn skipped(entry: &tradar_query::CheatEntry) -> bool {
    entry.category == "Transaction"
        || matches!(
            entry.title,
            "Cancel a running query" | "Terminate a connection"
        )
}

fn value(name: &str, title: &str) -> Option<String> {
    Some(
        match name {
            "table" => "t_main",
            "other_table" => "t_other",
            "new_table" => "t_new",
            "column" => "c",
            "date_column" => "created_at",
            "number_column" => "n",
            "fk_column" => "o_id",
            "new_column" => "nc",
            "key_column" => "id",
            "join_column" => "id",
            "group_column" => "g",
            "value_column" => "n",
            "json_column" => "j",
            "json_key" => "a",
            "key" => "1",
            "value" => "a",
            "a" => "a",
            "b" => "b",
            "filter" => "1 = 1",
            "last_value" => "",
            "pattern" => "work",
            "schema" => "public",
            "constraint" => "fk_new",
            "index_name" => "ix_new",
            "view" => "v_new",
            "role" if title.starts_with("CREATE") => "r_create",
            "role" => "r1",
            "password" => "secret",
            "pid" => "1",
            "name" => "sp1",
            _ => return None,
        }
        .to_string(),
    )
}

/// Statements that put the fixture back as it was; the first group may fail
/// (nothing to drop yet) and is not checked.
const RESET_LOOSE: &[&str] = &[
    "DROP VIEW IF EXISTS v_new",
    "DROP TABLE IF EXISTS t_new, t_main, t_other CASCADE",
    "DROP OWNED BY r1",
    "DROP ROLE IF EXISTS r1",
    "DROP OWNED BY r_create",
    "DROP ROLE IF EXISTS r_create",
];
const RESET: &[&str] = &[
    "CREATE ROLE r1",
    "CREATE TABLE t_other (id bigint PRIMARY KEY, c text)",
    "CREATE TABLE t_main (id bigserial PRIMARY KEY, c text, n int, created_at timestamptz DEFAULT now(), j jsonb, o_id bigint REFERENCES t_other (id), g text)",
    "INSERT INTO t_other VALUES (1, 'one')",
    "INSERT INTO t_main (c, n, j, o_id, g) VALUES ('a', 5, '{\"a\": 1}', 1, 'g1'), ('b', 7, '{}', 1, 'g1'), ('c', 9, NULL, NULL, 'g2')",
    "CREATE INDEX t_main_n_idx ON t_main (n)",
];

#[tokio::test]
async fn every_cheatsheet_entry_runs_on_a_real_server() {
    // A current server: the entries are written for PostgreSQL 12+.
    let container = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let mut driver = PostgresDriver::new(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ));
    driver.connect().await.unwrap();
    // `index_name` is also an existing index for the entries that drop one.
    let mut failures = Vec::new();
    let mut ran = 0;
    for entry in crate::cheatsheet::ENTRIES {
        if skipped(entry) {
            continue;
        }
        for sql in RESET_LOOSE {
            let _ = driver.execute(sql).await;
        }
        for sql in RESET {
            driver
                .execute(sql)
                .await
                .unwrap_or_else(|e| panic!("fixture `{sql}`: {e}"));
        }
        let sql = match tradar_query::fill_placeholders(entry.text, |n| value(n, entry.title)) {
            Ok(sql) => sql,
            Err(name) => {
                failures.push(format!("`{}`: no value for `${{{name}}}`", entry.title));
                continue;
            }
        };
        // The entries that drop an index drop the one the fixture has.
        let sql = if entry.title == "DROP INDEX" {
            sql.replace("ix_new", "t_main_n_idx")
        } else {
            sql
        };
        ran += 1;
        if let Err(e) = driver.execute(&sql).await
            && !MAY_FAIL.contains(&entry.title)
        {
            failures.push(format!("`{}`: {e}\n{sql}", entry.title));
        }
    }
    assert!(ran > 60, "only {ran} entries ran");
    assert!(
        failures.is_empty(),
        "{} of {ran} cheat sheet entries failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
