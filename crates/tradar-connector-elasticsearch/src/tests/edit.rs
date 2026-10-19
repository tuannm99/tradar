use super::*;

#[test]
fn crud_snippet_covers_all_four_ops() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    let entry = SchemaInfo::new("my-index");

    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Read, &[]),
        Some("GET my-index/_search\n{\n  \"query\": {\n    \"match_all\": {}\n  }\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Create, &[]),
        Some("POST my-index/_doc\n{\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Update, &[]),
        Some("POST my-index/_update/<id>\n{\n  \"doc\": {\n  }\n}".to_string())
    );
    assert_eq!(
        driver.crud_snippet(&entry, tradar_core::action::CrudOp::Delete, &[]),
        Some("DELETE my-index/_doc/<id>".to_string())
    );
}

#[test]
fn crud_snippet_create_with_a_real_mapping_lists_every_field() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Create,
            &[]
        ),
        Some("POST my-index/_doc\n{\n  \"title\": <value>,\n  \"views\": <value>\n}".to_string())
    );
}

#[test]
fn crud_snippet_create_with_an_explicit_selection_lists_only_those_fields() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Create,
            &["title".to_string()]
        ),
        Some("POST my-index/_doc\n{\n  \"title\": <value>\n}".to_string())
    );
}

#[test]
fn crud_snippet_read_with_a_selection_adds_a_source_filter() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Read,
            &["title".to_string()]
        ),
        Some(
            "GET my-index/_search\n{\n  \"_source\": [\"title\"],\n  \"query\": {\n    \"match_all\": {}\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn crud_snippet_update_with_a_real_mapping_sets_every_field_in_the_doc_body() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Update,
            &[]
        ),
        Some(
            "POST my-index/_update/<id>\n{\n  \"doc\": {\n    \"title\": <value>,\n    \"views\": <value>\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn crud_snippet_delete_ignores_columns_since_it_deletes_by_id_only() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.crud_snippet(
            &my_index_with_fields(),
            tradar_core::action::CrudOp::Delete,
            &["title".to_string()]
        ),
        Some("DELETE my-index/_doc/<id>".to_string())
    );
}

#[test]
fn edit_source_accepts_a_plain_single_index_search() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(
        driver.edit_source("GET my-index/_search\n{\"query\": {\"match_all\": {}}}"),
        Some("my-index".to_string())
    );
    assert_eq!(
        driver.edit_source("POST my-index/_search"),
        Some("my-index".to_string())
    );
}

#[test]
fn edit_source_refuses_multi_index_wildcard_and_non_search() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    assert_eq!(driver.edit_source("GET a,b/_search"), None);
    assert_eq!(driver.edit_source("GET my-*/_search"), None);
    assert_eq!(driver.edit_source("GET _all/_search"), None);
    assert_eq!(driver.edit_source("GET my-index/_count"), None);
    assert_eq!(driver.edit_source("GET my-index/_doc/1"), None);
    assert_eq!(driver.edit_source("DELETE my-index/_doc/1"), None);
}

#[test]
fn edit_sql_delete_row_targets_the_hit_s_id() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let edit = query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "abc123".to_string())],
        change: query_driver::RowChange::DeleteRow,
    };

    assert_eq!(
        driver.edit_sql(&edit),
        Some("DELETE my-index/_doc/abc123?refresh=true".to_string())
    );
}

#[test]
fn edit_sql_set_value_infers_the_value_s_json_type() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    let edit_for = |value: &str| query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "1".to_string())],
        change: query_driver::RowChange::SetValue {
            column: "views".to_string(),
            value: value.to_string(),
        },
    };

    assert_eq!(
        driver.edit_sql(&edit_for("42")),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"views\": 42\n  }\n}"
                .to_string()
        )
    );
    assert_eq!(
        driver.edit_sql(&edit_for("Ada")),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"views\": \"Ada\"\n  }\n}"
                .to_string()
        )
    );
}

#[test]
fn edit_sql_set_value_nests_a_dotted_column_into_a_real_object() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");

    let edit = query_driver::RowEdit {
        table: "my-index".to_string(),
        key: vec![("_id".to_string(), "1".to_string())],
        change: query_driver::RowChange::SetValue {
            column: "customer.name".to_string(),
            value: "Ada".to_string(),
        },
    };

    // Not a flat "customer.name" key -- a real nested object, or
    // Elasticsearch's partial-update merge would create a spurious
    // top-level field instead of reaching the real nested one.
    assert_eq!(
        driver.edit_sql(&edit),
        Some(
            "POST my-index/_update/1?refresh=true\n{\n  \"doc\": {\n    \"customer\": {\n      \"name\": \"Ada\"\n    }\n  }\n}"
                .to_string()
        )
    );
}
