use super::*;

#[test]
fn near_token_marker_points_at_the_quoted_token_s_position() {
    let message =
        "You have an error in your SQL syntax; ... right syntax to use near 'FRO users' at line 1";
    let query = "SELECT * FRO users";

    let marker = near_token_marker(message, query).unwrap();

    assert!(marker.contains("LINE 1:"), "marker was: {marker}");
    assert!(marker.contains('^'), "marker was: {marker}");
}

#[test]
fn near_token_marker_is_none_for_a_message_with_no_quoted_token() {
    assert_eq!(
        near_token_marker("Table 'mydb.users' doesn't exist", "SELECT * FROM users"),
        None
    );
}

#[test]
fn near_token_marker_is_none_when_the_token_is_not_actually_in_the_query() {
    assert_eq!(
        near_token_marker("... near 'XYZ' at line 1", "SELECT * FROM users"),
        None
    );
}

#[tokio::test]
async fn a_syntax_error_reports_the_offending_line_with_a_caret() {
    let container = Mysql::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let conn_string = format!("mysql://root@127.0.0.1:{port}/test");
    let mut driver = MySqlDriver::new(&conn_string);
    driver.connect().await.unwrap();

    let error = driver
        .execute("SELECT * FRO users")
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("LINE 1:"), "error was: {error}");
    assert!(error.contains('^'), "error was: {error}");
}
