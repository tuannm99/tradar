#[tokio::test]
async fn the_row_budget_stops_at_the_row_cap_or_the_byte_cap() {
    // Without a scope: the library cap.
    assert_eq!(max_rows(), MAX_ROWS);

    with_max_rows(3, async {
        assert_eq!(max_rows(), 3);
        let mut budget = RowBudget::default();
        for _ in 0..3 {
            assert!(!budget.is_full());
            budget.keep(&["x".to_string()]);
        }
        assert!(budget.is_full(), "three rows is the cap");
    })
    .await;

    with_max_rows(1_000, async {
        let mut budget = RowBudget::default();
        budget.keep(&["x".repeat(MAX_RESULT_BYTES)]);
        assert!(budget.is_full(), "one huge row already fills the byte cap");
    })
    .await;
}

use super::*;

/// Where `^` landed relative to the start of the query text on the
/// line above it, plus that line's own text -- what actually matters,
/// rather than the exact width of the `LINE N: ` prefix in front of
/// it.
fn caret_position(marker: &str) -> (usize, &str) {
    let mut lines = marker.lines();
    let line = lines.next().unwrap();
    let caret_line = lines.next().unwrap();
    let caret_column = caret_line.find('^').unwrap();
    // `line` is `LINE N: <text>` -- the first `": "` marks where
    // `<text>` starts.
    let text_start = line.find(": ").unwrap() + 2;
    (caret_column - text_start, &line[text_start..])
}

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

fn texts(sql: &str) -> Vec<String> {
    split_sql_statements(sql)
        .into_iter()
        .map(|s| s.text)
        .collect()
}

fn users_table() -> SchemaInfo {
    SchemaInfo {
        name: "users".to_string(),
        columns: vec![
            ColumnInfo {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
                primary_key: true,
                foreign_key: None,
                indexed: false,
            },
            ColumnInfo::new("email", "TEXT"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
}

#[test]
fn crud_snippet_read_is_a_bounded_select() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Read, &[]),
        "SELECT * FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_create_lists_every_column_one_per_line_with_a_named_placeholder() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Create, &[]),
        "INSERT INTO \"users\" (\n  \"id\",\n  \"email\"\n) VALUES (\n  <id>,\n  <email>\n);"
    );
}

#[test]
fn crud_snippet_update_sets_non_key_columns_one_per_line_and_filters_by_the_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &[]),
        "UPDATE \"users\" SET\n  \"email\" = <email>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_delete_filters_by_the_primary_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &[]),
        "DELETE FROM \"users\" WHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_falls_back_gracefully_with_no_known_columns() {
    let entry = SchemaInfo::new("mystery");

    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Create, &[]),
        "INSERT INTO \"mystery\" (<column>) VALUES (<value>);"
    );
    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Update, &[]),
        "UPDATE \"mystery\" SET\n  <column> = <value>\nWHERE <condition>;"
    );
    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Delete, &[]),
        "DELETE FROM \"mystery\" WHERE <condition>;"
    );
}

#[test]
fn crud_snippet_create_and_update_wrap_one_column_per_line_so_no_line_runs_long() {
    let entry = SchemaInfo {
        name: "wide".to_string(),
        columns: (0..12)
            .map(|i| ColumnInfo::new(format!("column_number_{i}"), "TEXT"))
            .collect(),
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    for op in [CrudOp::Create, CrudOp::Update] {
        let snippet = build_crud_snippet(&entry, op, &[]);
        for line in snippet.lines() {
            assert!(
                line.chars().count() < 80,
                "line too long for {op:?}: {line:?}"
            );
        }
    }
}

#[test]
fn crud_snippet_update_with_no_primary_key_sets_every_column() {
    let entry = SchemaInfo {
        name: "logs".to_string(),
        columns: vec![ColumnInfo::new("message", "TEXT")],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    };

    assert_eq!(
        build_crud_snippet(&entry, CrudOp::Update, &[]),
        "UPDATE \"logs\" SET\n  \"message\" = <message>\nWHERE <condition>;"
    );
}

#[test]
fn crud_snippet_read_with_an_explicit_selection_lists_only_those_columns_in_table_order() {
    assert_eq!(
        build_crud_snippet(
            &users_table(),
            CrudOp::Read,
            &["email".to_string(), "id".to_string()]
        ),
        "SELECT \"id\", \"email\" FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_read_with_a_selection_matching_no_real_column_falls_back_to_select_star() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Read, &["ghost".to_string()]),
        "SELECT * FROM \"users\" LIMIT 100;"
    );
}

#[test]
fn crud_snippet_create_with_an_explicit_selection_lists_only_those_columns() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Create, &["email".to_string()]),
        "INSERT INTO \"users\" (\n  \"email\"\n) VALUES (\n  <email>\n);"
    );
}

#[test]
fn crud_snippet_update_with_an_explicit_selection_sets_only_those_columns() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &["email".to_string()]),
        "UPDATE \"users\" SET\n  \"email\" = <email>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_update_with_the_primary_key_explicitly_selected_sets_it_too() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Update, &["id".to_string()]),
        "UPDATE \"users\" SET\n  \"id\" = <id>\nWHERE \"id\" = <id>;"
    );
}

#[test]
fn crud_snippet_delete_can_filter_by_a_non_primary_key_column() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &["email".to_string()]),
        "DELETE FROM \"users\" WHERE \"email\" = <email>;"
    );
}

#[test]
fn crud_snippet_delete_with_a_selection_matching_no_real_column_falls_back_to_the_primary_key() {
    assert_eq!(
        build_crud_snippet(&users_table(), CrudOp::Delete, &["ghost".to_string()]),
        "DELETE FROM \"users\" WHERE \"id\" = <id>;"
    );
}

#[test]
fn column_types_looks_up_each_column_by_name_in_the_source_table() {
    let schema = vec![users_table()];
    let columns = vec!["id".to_string(), "email".to_string()];

    assert_eq!(
        column_types(&columns, &schema, Some("users")),
        vec![Some("INTEGER".to_string()), Some("TEXT".to_string())]
    );
}

#[test]
fn column_types_is_none_for_a_column_the_source_table_does_not_have() {
    let schema = vec![users_table()];
    let columns = vec!["id".to_string(), "computed_alias".to_string()];

    assert_eq!(
        column_types(&columns, &schema, Some("users")),
        vec![Some("INTEGER".to_string()), None]
    );
}

#[test]
fn column_types_is_all_none_with_no_source_table() {
    let columns = vec!["id".to_string(), "email".to_string()];

    assert_eq!(
        column_types(&columns, &[users_table()], None),
        vec![None; 2]
    );
}

#[test]
fn column_types_is_all_none_when_the_table_is_not_in_schema() {
    let columns = vec!["id".to_string()];

    assert_eq!(
        column_types(&columns, &[users_table()], Some("ghost")),
        vec![None]
    );
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
fn offsets_point_at_the_statement_in_the_original_buffer() {
    let sql = "SELECT 1;\n\nSELECT 2;";

    let statements = split_sql_statements(sql);

    assert_eq!(&sql[statements[0].start..statements[0].end], "SELECT 1");
    assert_eq!(&sql[statements[1].start..statements[1].end], "SELECT 2");
}

#[test]
fn an_unterminated_statement_still_counts() {
    // The last statement usually has no trailing semicolon.
    assert_eq!(texts("SELECT 1;\nSELECT 2"), vec!["SELECT 1", "SELECT 2"]);
    assert_eq!(texts("SELECT 'unclosed"), vec!["SELECT 'unclosed"]);
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

fn key(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(c, v)| (c.to_string(), v.to_string()))
        .collect()
}

#[test]
fn an_update_names_the_column_the_table_and_the_key() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "7")]),
        change: RowChange::SetValue {
            column: "name".to_string(),
            value: "Ada".to_string(),
        },
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"name\" = 'Ada' WHERE \"id\" = '7'"
    );
}

#[test]
fn an_insert_lists_columns_and_literals_and_leaves_null_unquoted() {
    let sql = build_sql_insert(
        "public.users",
        &key(&[("name", "O'Brien"), ("nickname", "NULL")]),
    );

    assert_eq!(
        sql,
        "INSERT INTO \"public\".\"users\" (\"name\", \"nickname\") VALUES ('O''Brien', NULL)"
    );
}

#[test]
fn several_columns_of_a_row_become_one_update() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "7")]),
        change: RowChange::SetValues(key(&[("name", "O'Brien"), ("nickname", "NULL")])),
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"name\" = 'O''Brien', \"nickname\" = NULL WHERE \"id\" = '7'"
    );
}

#[test]
fn a_delete_uses_every_column_of_a_composite_key() {
    let sql = build_sql_edit(&RowEdit {
        table: "memberships".to_string(),
        key: key(&[("user_id", "1"), ("group_id", "2")]),
        change: RowChange::DeleteRow,
    });

    assert_eq!(
        sql,
        "DELETE FROM \"memberships\" WHERE \"user_id\" = '1' AND \"group_id\" = '2'"
    );
}

#[test]
fn a_quote_in_a_value_is_escaped_rather_than_ending_the_literal() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::SetValue {
            column: "name".to_string(),
            value: "O'Brien".to_string(),
        },
    });

    assert_eq!(
        sql, "UPDATE \"users\" SET \"name\" = 'O''Brien' WHERE \"id\" = '1'",
        "an unescaped quote here would be an injection, not a typo"
    );
}

#[test]
fn a_null_is_written_unquoted_the_way_the_grid_shows_it() {
    let sql = build_sql_edit(&RowEdit {
        table: "users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::SetValue {
            column: "nickname".to_string(),
            value: "NULL".to_string(),
        },
    });

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"nickname\" = NULL WHERE \"id\" = '1'"
    );
}

#[test]
fn a_schema_qualified_table_stays_two_identifiers() {
    let sql = build_sql_edit(&RowEdit {
        table: "public.users".to_string(),
        key: key(&[("id", "1")]),
        change: RowChange::DeleteRow,
    });

    assert_eq!(sql, "DELETE FROM \"public\".\"users\" WHERE \"id\" = '1'");
}

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

#[test]
fn dot_after_an_alias_resolves_to_its_table() {
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u.na"),
        CompletionContext::TableColumns {
            table: "users".to_string()
        },
        "a partial column name after the dot doesn't change which table it resolves to"
    );
}

#[test]
fn dot_after_an_as_aliased_table_resolves_to_its_table() {
    assert_eq!(
        completion_context("SELECT * FROM users AS u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn dot_on_the_bare_table_name_resolves_to_itself() {
    assert_eq!(
        completion_context("SELECT * FROM users WHERE users."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn dot_after_a_join_alias_resolves_to_the_joined_table() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN orders o ON o.user_id = u.id WHERE o."),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn dot_on_an_unknown_alias_is_tried_as_a_schema_name_instead() {
    // `x` isn't a `FROM`/`JOIN` alias in this query -- but this
    // function has no schema to check `x` against either way, so it
    // reports the weaker `SchemaQualifiedTable` context rather than
    // giving up; `matches_in_context` is the one with schema access,
    // and falls back to the flat list itself if `x` isn't a real
    // schema/keyspace name.
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE x."),
        CompletionContext::SchemaQualifiedTable {
            schema: "x".to_string()
        }
    );
}

#[test]
fn dot_after_a_schema_name_reports_it_for_schema_qualified_lookup() {
    assert_eq!(
        completion_context("SELECT * FROM demo.us"),
        CompletionContext::SchemaQualifiedTable {
            schema: "demo".to_string()
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM demo."),
        CompletionContext::SchemaQualifiedTable {
            schema: "demo".to_string()
        }
    );
}

#[test]
fn dot_after_a_resolvable_alias_still_wins_over_schema_qualified() {
    assert_eq!(
        completion_context("SELECT * FROM users u WHERE u."),
        CompletionContext::TableColumns {
            table: "users".to_string()
        }
    );
}

#[test]
fn right_after_join_reports_the_tables_already_in_the_query() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN "),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string()]
        }
    );
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN or"),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string()]
        },
        "a partial table name being typed doesn't change the context"
    );
}

#[test]
fn known_tables_after_join_include_every_earlier_from_and_join() {
    assert_eq!(
        completion_context("SELECT * FROM users u JOIN orders o ON o.user_id = u.id JOIN "),
        CompletionContext::JoinTarget {
            known_tables: vec!["users".to_string(), "orders".to_string()]
        }
    );
}

#[test]
fn plain_text_with_no_dot_or_join_has_no_special_context() {
    assert_eq!(
        completion_context("SELECT * FROM users WHERE na"),
        CompletionContext::None
    );
    assert_eq!(completion_context(""), CompletionContext::None);
}

#[test]
fn a_field_typed_inside_a_mongo_call_resolves_to_its_collection() {
    assert_eq!(
        completion_context("db.orders.find({status: \"open\", na"),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn an_empty_field_position_right_after_the_opening_brace_also_resolves() {
    assert_eq!(
        completion_context("db.orders.find({"),
        CompletionContext::TableColumns {
            table: "orders".to_string()
        }
    );
}

#[test]
fn typing_the_mongo_method_name_itself_is_not_scoped_to_a_collection() {
    // Still inside the `db.orders.` chain, not yet past an opening
    // paren -- this is completing the method name (`find`, `findOne`,
    // ...), which isn't collection-specific, so it gets its own
    // context rather than being scoped like a field.
    assert_eq!(
        completion_context("db.orders.f"),
        CompletionContext::MongoMethod
    );
}

#[test]
fn a_bare_db_dot_with_nothing_typed_yet_is_also_the_method_context() {
    assert_eq!(
        completion_context("db.orders."),
        CompletionContext::MongoMethod
    );
}

#[test]
fn typing_the_collection_name_itself_is_not_the_method_context() {
    // Only one dot so far -- `resolve_alias` finds nothing for `db`
    // (no FROM/JOIN exists for Mongo text), same as always; collection
    // names are already ranked correctly in the flat list (they're
    // `CandidateKind::Table`), so this must not narrow to methods.
    // `SchemaQualifiedTable` rather than `None` since that failed
    // alias lookup is now tried as a schema name too -- `db` almost
    // never matches a real Mongo database name, so
    // `matches_in_context` falls back to the exact same flat list
    // `None` would have given; it is the fallback inside that context,
    // not this function, telling the two apart.
    assert_eq!(
        completion_context("db.ord"),
        CompletionContext::SchemaQualifiedTable {
            schema: "db".to_string()
        }
    );
}

#[test]
fn a_second_mongo_call_after_the_first_closes_resolves_to_its_own_collection() {
    assert_eq!(
        completion_context("db.orders.find({}); db.payments.find({na"),
        CompletionContext::TableColumns {
            table: "payments".to_string()
        }
    );
}

#[test]
fn plain_sql_nested_in_parens_is_not_mistaken_for_a_mongo_call() {
    assert_eq!(
        completion_context("SELECT COUNT(na"),
        CompletionContext::None
    );
}

#[test]
fn add_column_builds_a_nullable_column_with_no_default_by_default() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
        type_name: "TEXT".to_string(),
        nullable: true,
        default: None,
    });

    assert_eq!(sql, "ALTER TABLE \"users\" ADD COLUMN \"nickname\" TEXT");
}

#[test]
fn add_column_appends_not_null_and_a_raw_default_expression() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "credits".to_string(),
        type_name: "INTEGER".to_string(),
        nullable: false,
        default: Some("0".to_string()),
    });

    assert_eq!(
        sql,
        "ALTER TABLE \"users\" ADD COLUMN \"credits\" INTEGER NOT NULL DEFAULT 0"
    );
}

#[test]
fn add_column_treats_a_blank_default_as_no_default() {
    let sql = build_table_ddl(&TableDesignerOp::AddColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
        type_name: "TEXT".to_string(),
        nullable: true,
        default: Some("   ".to_string()),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" ADD COLUMN \"nickname\" TEXT");
}

#[test]
fn drop_column_quotes_table_and_column() {
    let sql = build_table_ddl(&TableDesignerOp::DropColumn {
        table: "users".to_string(),
        column: "nickname".to_string(),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" DROP COLUMN \"nickname\"");
}

#[test]
fn rename_table_builds_rename_to() {
    let sql = build_table_ddl(&TableDesignerOp::RenameTable {
        table: "users".to_string(),
        new_name: "accounts".to_string(),
    });

    assert_eq!(sql, "ALTER TABLE \"users\" RENAME TO \"accounts\"");
}

#[test]
fn create_table_lists_one_column_per_line_with_not_null_where_asked() {
    let sql = build_table_ddl(&TableDesignerOp::CreateTable {
        table: "accounts".to_string(),
        columns: vec![
            NewColumn {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
                nullable: false,
                primary_key: true,
            },
            NewColumn {
                name: "email".to_string(),
                type_name: "TEXT".to_string(),
                nullable: false,
                primary_key: false,
            },
            NewColumn {
                name: "nickname".to_string(),
                type_name: "TEXT".to_string(),
                nullable: true,
                primary_key: false,
            },
        ],
    });

    assert_eq!(
        sql,
        "CREATE TABLE \"accounts\" (\n  \"id\" INTEGER PRIMARY KEY,\n  \"email\" TEXT NOT NULL,\n  \"nickname\" TEXT\n)"
    );
}

#[test]
fn create_table_never_states_not_null_redundantly_on_a_primary_key() {
    let sql = build_table_ddl(&TableDesignerOp::CreateTable {
        table: "t".to_string(),
        columns: vec![NewColumn {
            name: "id".to_string(),
            type_name: "INTEGER".to_string(),
            nullable: false,
            primary_key: true,
        }],
    });

    assert!(sql.contains("PRIMARY KEY"));
    assert!(
        !sql.contains("NOT NULL"),
        "PRIMARY KEY already implies it: {sql}"
    );
}
