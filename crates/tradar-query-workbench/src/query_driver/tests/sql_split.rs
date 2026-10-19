use super::*;

#[test]
fn line_and_caret_points_at_a_one_based_position_on_a_single_line() {
    let query = "SELECT * FRO users";
    let position = query.find("FRO").unwrap() + 1; // 1-based index of 'F'

    let marker = line_and_caret(query, position).unwrap();

    let (offset, text) = caret_position(&marker);
    assert_eq!(text, query);
    assert_eq!(offset, query.find("FRO").unwrap());
}

#[test]
fn line_and_caret_finds_the_right_line_in_a_multi_line_query() {
    let query = "SELECT id\nFROM usres\nWHERE id = 1";
    // 1-based char index of the `u` in `usres` (line 2).
    let position = query.find("usres").unwrap() + 1;

    let marker = line_and_caret(query, position).unwrap();

    let (offset, text) = caret_position(&marker);
    assert_eq!(text, "FROM usres");
    assert_eq!(offset, text.find('u').unwrap());
    assert!(marker.starts_with("LINE 2:"), "marker was: {marker}");
}

#[test]
fn line_and_caret_is_none_past_the_end_of_the_query() {
    assert_eq!(line_and_caret("SELECT 1", 100), None);
}

#[test]
fn line_and_caret_is_none_for_position_zero() {
    // Postgres never actually sends 0, but the type is an unsigned
    // int -- this is what "no position" would look like if it did.
    assert_eq!(line_and_caret("SELECT 1", 0), None);
}

#[test]
fn transaction_control_recognizes_begin_commit_and_rollback() {
    assert_eq!(
        transaction_control("BEGIN;"),
        Some(TransactionControl::Begin)
    );
    assert_eq!(
        transaction_control("begin transaction"),
        Some(TransactionControl::Begin)
    );
    assert_eq!(
        transaction_control("START TRANSACTION"),
        Some(TransactionControl::Begin)
    );
    assert_eq!(
        transaction_control("COMMIT;"),
        Some(TransactionControl::Commit)
    );
    assert_eq!(transaction_control("end"), Some(TransactionControl::Commit));
    assert_eq!(
        transaction_control("ROLLBACK"),
        Some(TransactionControl::Rollback)
    );
}

#[test]
fn transaction_control_ignores_an_ordinary_statement() {
    assert_eq!(transaction_control("SELECT 1"), None);
    assert_eq!(
        transaction_control("INSERT INTO commit_log VALUES (1)"),
        None,
        "a table named after a keyword must not trip this up"
    );
}

#[test]
fn transaction_control_sees_past_a_leading_comment() {
    assert_eq!(
        transaction_control("-- start a transaction\nBEGIN;"),
        Some(TransactionControl::Begin)
    );
}

#[test]
fn offsets_point_at_the_statement_in_the_original_buffer() {
    let sql = "SELECT 1;\n\nSELECT 2;";

    let statements = split_sql_statements(sql);

    assert_eq!(&sql[statements[0].start..statements[0].end], "SELECT 1");
    assert_eq!(&sql[statements[1].start..statements[1].end], "SELECT 2");
}

#[test]
fn row_returning_statements_are_recognised() {
    for sql in [
        "SELECT 1",
        "  select * from users",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "VALUES (1)",
        "PRAGMA table_info(users)",
        "EXPLAIN SELECT 1",
        "show tables",
    ] {
        assert!(returns_rows(sql), "{sql:?} should return rows");
    }
}

#[test]
fn data_changing_statements_are_recognised() {
    for sql in [
        "INSERT INTO users VALUES (1)",
        "update users set name = 'x'",
        "DELETE FROM users",
        "CREATE TABLE users (id INT)",
        "DROP TABLE users",
        "  \n alter table users add column x int",
    ] {
        assert!(!returns_rows(sql), "{sql:?} should not return rows");
    }
}

#[test]
fn a_returning_clause_makes_a_write_return_rows() {
    assert!(returns_rows("INSERT INTO users VALUES (1) RETURNING id"));
    assert!(returns_rows("delete from users returning *"));
    assert!(
        !returns_rows("INSERT INTO returning_log VALUES (1)"),
        "a table merely named like the keyword is not a RETURNING clause"
    );
}

#[test]
fn leading_comments_do_not_hide_the_statement() {
    assert!(returns_rows("-- a note\nSELECT 1"));
    assert!(returns_rows("/* a note */ SELECT 1"));
    assert!(!returns_rows("-- a note\nINSERT INTO users VALUES (1)"));
    assert!(!returns_rows("/* one */ /* two */\nDELETE FROM users"));
}

#[test]
fn an_empty_statement_is_not_row_returning() {
    assert!(!returns_rows(""));
    assert!(!returns_rows("   \n  "));
    assert!(!returns_rows("-- only a comment"));
}
