use super::*;

#[test]
fn a_buffer_splits_on_semicolons() {
    assert_eq!(texts("SELECT 1; SELECT 2"), vec!["SELECT 1", "SELECT 2"]);
}

#[test]
fn a_statement_may_span_several_lines() {
    let sql = "SELECT id,\n       name\nFROM users\nWHERE id = 1;\n\nSELECT 2;";

    assert_eq!(
        texts(sql),
        vec![
            "SELECT id,\n       name\nFROM users\nWHERE id = 1",
            "SELECT 2"
        ],
        "line breaks are not statement boundaries"
    );
}

#[test]
fn a_semicolon_inside_a_string_is_not_a_separator() {
    assert_eq!(
        texts("SELECT ';' AS x; SELECT 2"),
        vec!["SELECT ';' AS x", "SELECT 2"]
    );
    assert_eq!(
        texts(r#"SELECT "a;b" FROM t"#),
        vec![r#"SELECT "a;b" FROM t"#]
    );
}

#[test]
fn a_doubled_quote_inside_a_string_does_not_end_it() {
    assert_eq!(
        texts("SELECT 'it''s; fine' AS x; SELECT 2"),
        vec!["SELECT 'it''s; fine' AS x", "SELECT 2"]
    );
}

#[test]
fn a_semicolon_inside_a_comment_is_not_a_separator() {
    assert_eq!(
        texts("SELECT 1 -- one; two\n; SELECT 2"),
        vec!["SELECT 1 -- one; two", "SELECT 2"]
    );
    assert_eq!(
        texts("SELECT /* a; b */ 1; SELECT 2"),
        vec!["SELECT /* a; b */ 1", "SELECT 2"]
    );
}

#[test]
fn a_dollar_quoted_body_may_contain_semicolons() {
    let sql =
        "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; SELECT 2";

    assert_eq!(
        texts(sql),
        vec![
            "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql",
            "SELECT 2"
        ]
    );
}

#[test]
fn a_tagged_dollar_quote_is_matched_by_its_tag() {
    let sql = "SELECT $tag$ a; $$ still inside $tag$; SELECT 2";

    assert_eq!(
        texts(sql),
        vec!["SELECT $tag$ a; $$ still inside $tag$", "SELECT 2"]
    );
}

#[test]
fn empty_statements_are_dropped() {
    assert_eq!(texts("SELECT 1;"), vec!["SELECT 1"]);
    assert_eq!(texts("SELECT 1;;;"), vec!["SELECT 1"]);
    assert!(texts("").is_empty());
    assert!(texts("  \n ; ; ").is_empty());
    assert!(
        texts("-- just a comment").is_empty(),
        "nothing to run in a comment-only buffer"
    );
}

#[test]
fn an_unterminated_statement_still_counts() {
    // The last statement usually has no trailing semicolon.
    assert_eq!(texts("SELECT 1;\nSELECT 2"), vec!["SELECT 1", "SELECT 2"]);
    assert_eq!(texts("SELECT 'unclosed"), vec!["SELECT 'unclosed"]);
}
