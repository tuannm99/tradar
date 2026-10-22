use super::*;

#[test]
fn a_plain_select_reports_the_table_it_reads() {
    assert_eq!(
        single_table_source("SELECT * FROM users"),
        Some("users".to_string())
    );
    assert_eq!(
        single_table_source("select id, name from public.users where id = 1 order by id"),
        Some("public.users".to_string())
    );
    assert_eq!(
        single_table_source("SELECT * FROM users u WHERE u.id = 1"),
        Some("users".to_string()),
        "an alias doesn't change which table the rows came from"
    );
    assert_eq!(
        single_table_source("-- yesterday's numbers\nSELECT * FROM users"),
        Some("users".to_string())
    );
}

#[test]
fn a_result_that_is_not_one_table_has_no_editable_source() {
    for sql in [
        "SELECT * FROM users JOIN orders ON orders.user_id = users.id",
        "SELECT * FROM users, orders",
        "SELECT * FROM users UNION SELECT * FROM admins",
        "SELECT count(*) FROM users GROUP BY country",
        "SELECT DISTINCT country FROM users",
        "SELECT * FROM (SELECT * FROM users) t",
        "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders)",
        "INSERT INTO users VALUES (1)",
        "UPDATE users SET name = 'x'",
        "SELECT 1",
    ] {
        assert_eq!(
            single_table_source(sql),
            None,
            "must not be editable: {sql}"
        );
    }
}

#[test]
fn a_keyword_inside_parentheses_does_not_disqualify_the_source() {
    assert_eq!(
        single_table_source("SELECT coalesce(name, 'x') FROM users WHERE id = 1"),
        Some("users".to_string())
    );
}

#[test]
fn joined_column_sources_resolves_explicitly_qualified_columns() {
    let columns = ["id".to_string(), "total".to_string(), "name".to_string()];
    assert_eq!(
        joined_column_sources(
            "SELECT o.id, o.total, u.name FROM orders o JOIN users u ON u.id = o.user_id",
            &columns,
        ),
        vec![
            Some("orders".to_string()),
            Some("orders".to_string()),
            Some("users".to_string()),
        ]
    );
}

#[test]
fn joined_column_sources_leaves_unqualified_or_computed_columns_unresolved() {
    let columns = ["id".to_string(), "cnt".to_string()];
    assert_eq!(
        joined_column_sources(
            "SELECT id, count(*) AS cnt FROM orders o JOIN users u ON u.id = o.user_id",
            &columns,
        ),
        vec![None, None],
        "a bare name is ambiguous across joined tables without the schema, \
         and an expression has no one table to point at"
    );
}

#[test]
fn joined_column_sources_does_not_mistake_count_star_for_a_select_star() {
    let columns = ["id".to_string(), "cnt".to_string()];
    assert_eq!(
        joined_column_sources(
            "SELECT o.id, count(*) AS cnt FROM orders o JOIN users u ON u.id = o.user_id",
            &columns,
        ),
        vec![Some("orders".to_string()), None],
        "the `*` is count's argument, not a select-list star expansion"
    );
}

#[test]
fn joined_column_sources_bails_entirely_on_a_star() {
    let columns = ["id".to_string(), "name".to_string()];
    assert_eq!(
        joined_column_sources(
            "SELECT o.*, u.name FROM orders o JOIN users u ON u.id = o.user_id",
            &columns,
        ),
        vec![None, None],
        "`*` expands to a number of columns this can't predict from the query text"
    );
}

#[test]
fn joined_column_sources_is_empty_for_a_single_table_query() {
    let columns = ["id".to_string()];
    assert_eq!(
        joined_column_sources("SELECT id FROM users", &columns),
        vec![None],
        "not wrong, just not this function's job -- single_table_source answers this"
    );
}

#[test]
fn joined_column_sources_gives_up_on_a_subquery_or_union() {
    let columns = ["id".to_string()];
    for sql in [
        "SELECT o.id FROM orders o WHERE o.id IN (SELECT user_id FROM users)",
        "SELECT o.id FROM orders o UNION SELECT u.id FROM users u",
    ] {
        assert_eq!(joined_column_sources(sql, &columns), vec![None], "{sql}");
    }
}
