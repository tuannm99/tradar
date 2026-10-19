use super::*;

#[test]
fn near_token_marker_points_at_the_quoted_token_s_position() {
    let query = "SELECT * FRO users";
    let message = r#"near "FRO": syntax error"#;

    let marker = near_token_marker(message, query).unwrap();

    assert!(marker.starts_with("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains(query), "marker was: {marker}");
    let caret_line = marker.lines().nth(1).unwrap();
    assert_eq!(
        caret_line.find('^').unwrap(),
        marker.lines().next().unwrap().find("FRO").unwrap(),
        "caret should land under the F of FRO, marker was: {marker}"
    );
}

#[test]
fn near_token_marker_is_none_for_a_message_with_no_quoted_token() {
    assert_eq!(
        near_token_marker(
            "UNIQUE constraint failed: users.email",
            "INSERT INTO users..."
        ),
        None
    );
}

#[test]
fn near_token_marker_is_none_when_the_token_is_not_actually_in_the_query() {
    // Defensive: shouldn't happen against a real SQLite message, but
    // the query and message come from two different places, so there's
    // nothing stopping them from disagreeing.
    assert_eq!(
        near_token_marker(r#"near "XYZ": syntax error"#, "SELECT * FROM users"),
        None
    );
}

#[tokio::test]
async fn a_real_syntax_error_gets_a_line_and_caret_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let err = driver.execute("SELECT * FRO users").await.unwrap_err();

    let text = err.to_string();
    assert!(
        text.starts_with("near \"FRO\": syntax error"),
        "was: {text}"
    );
    assert!(text.contains("LINE 1:"), "was: {text}");
    assert!(text.contains('^'), "was: {text}");
}

#[tokio::test]
async fn an_error_with_no_quoted_token_still_reports_the_driver_s_own_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let mut driver = SqliteDriver::new(path.to_str().unwrap());
    driver.connect().await.unwrap();

    let err = driver
        .execute("SELECT * FROM no_such_table")
        .await
        .unwrap_err();

    assert!(err.to_string().contains("no such table"), "was: {err}");
}
