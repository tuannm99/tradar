use super::*;
use testcontainers_modules::elastic_search::ElasticSearch;
use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};
pub(crate) fn my_index_with_fields() -> SchemaInfo {
    SchemaInfo {
        name: "my-index".to_string(),
        columns: vec![
            ColumnInfo::new("title", "text"),
            ColumnInfo::new("views", "long"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
}

mod docker;
mod edit;
mod misc;
mod request;
mod response;

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

#[test]
fn every_cheatsheet_entry_is_one_request_with_valid_json() {
    let driver = ElasticsearchDriver::new("http://127.0.0.1:1");
    for entry in crate::cheatsheet::ENTRIES {
        let text = filled(entry.text);
        assert_eq!(driver.split_statements(&text).len(), 1, "`{}`", entry.title);
        let (_, _, body) = crate::request::parse_query(&text)
            .unwrap_or_else(|| panic!("`{}` is not a request:\n{text}", entry.title));
        if let Some(body) = body {
            crate::request::validate_json_body(&body)
                .unwrap_or_else(|e| panic!("`{}` has a bad body: {e}\n{text}", entry.title));
        }
    }
}
