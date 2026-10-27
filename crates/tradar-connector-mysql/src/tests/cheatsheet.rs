//! Every cheat sheet entry, run against a real MySQL (Docker). A failing
//! entry is listed with its error; the entries that need something a stock
//! container does not have are named in `MAY_FAIL`.
use super::*;

/// Needs a feature the stock image does not enable.
const MAY_FAIL: &[&str] = &[];

/// Would end this session, or needs more than one statement of state.
fn skipped(entry: &tradar_query::CheatEntry) -> bool {
    entry.category == "Transaction"
        || matches!(
            entry.title,
            "Kill a query (keep the connection)" | "Kill a connection"
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
            "constraint" => "fk_new",
            "index_name" => "ix_new",
            "view" => "v_new",
            "database" => "test",
            "host" => "%",
            "user" if title.starts_with("CREATE") => "u_create",
            "user" => "u1",
            "password" => "secret",
            "id" => "1",
            "name" => "sp1",
            _ => return None,
        }
        .to_string(),
    )
}

const RESET_LOOSE: &[&str] = &[
    "DROP VIEW IF EXISTS v_new",
    "DROP TABLE IF EXISTS t_new, t_main, t_other",
    "DROP USER IF EXISTS 'u1'@'%'",
    "DROP USER IF EXISTS 'u_create'@'%'",
];
const RESET: &[&str] = &[
    "CREATE USER 'u1'@'%'",
    "GRANT SELECT ON test.* TO 'u1'@'%'",
    "CREATE TABLE t_other (id BIGINT PRIMARY KEY, c VARCHAR(50)) ENGINE=InnoDB",
    "CREATE TABLE t_main (id BIGINT AUTO_INCREMENT PRIMARY KEY, c VARCHAR(50), n INT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP, j JSON, o_id BIGINT, g VARCHAR(20), FOREIGN KEY (o_id) REFERENCES t_other (id)) ENGINE=InnoDB",
    "INSERT INTO t_other VALUES (1, 'one')",
    "INSERT INTO t_main (c, n, j, o_id, g) VALUES ('a', 5, '{\"a\": 1}', 1, 'g1'), ('b', 7, '{}', 1, 'g1'), ('c', 9, NULL, NULL, 'g2')",
    "CREATE INDEX t_main_n_idx ON t_main (n)",
];

#[tokio::test]
async fn every_cheatsheet_entry_runs_on_a_real_server() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let mut driver = MySqlDriver::new(&format!("mysql://root@127.0.0.1:{port}/test"));
    driver.connect().await.unwrap();
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
