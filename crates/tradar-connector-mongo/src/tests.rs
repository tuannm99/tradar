use super::*;
use crate::convert::*;
use crate::relax::*;
use crate::run::*;
use crate::shell::*;
use crate::snippets::*;
use mongodb::bson::Document;
use tradar_query::{self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo};
pub(crate) fn users_with_fields() -> SchemaInfo {
    let mut column = ColumnInfo::new("_id", "ObjectId");
    column.primary_key = true;
    SchemaInfo {
        name: "users".to_string(),
        columns: vec![column, ColumnInfo::new("name", "String")],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
}
pub(crate) fn orders_with_a_nested_address() -> SchemaInfo {
    let mut id = ColumnInfo::new("_id", "ObjectId");
    id.primary_key = true;
    SchemaInfo {
        name: "orders".to_string(),
        columns: vec![
            id,
            ColumnInfo::new("address.city", "String"),
            ColumnInfo::new("address.zip", "String"),
        ],
        kind: None,
        ttl: None,
        schema: None,
        object_kind: None,
    }
}
pub(crate) fn id_key(id: &str) -> Vec<(String, String)> {
    vec![("_id".to_string(), id.to_string())]
}
/// A `Collection` handle against an address nothing listens on --
/// `Client::with_uri_str`/`with_options` only parse the URI and set up
/// internal state, they don't connect (MongoDB drivers connect lazily,
/// on the first real operation) -- so this is safe to build without
/// Docker or network access, as long as the test never actually awaits
/// an operation on it. Every `run_method` test below relies on that:
/// each one exercises a validation error that `run_method` returns
/// *before* it ever awaits the driver call that would need a real
/// server.
pub(crate) async fn unreachable_collection() -> mongodb::Collection<Document> {
    let client = mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/test")
        .await
        .unwrap();
    client.database("test").collection::<Document>("users")
}
pub(crate) fn doc_from_json(json: serde_json::Value) -> Document {
    json_to_document(json).unwrap()
}
use testcontainers_modules::mongo::Mongo;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

mod docker_queries;
mod docker_session;
mod parse;
mod run;
mod snippets;

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
fn every_cheatsheet_entry_parses() {
    for entry in crate::cheatsheet::ENTRIES {
        let text = filled(entry.text);
        crate::shell::parse_shell_command(&text)
            .unwrap_or_else(|e| panic!("`{}` does not parse: {e}\n{text}", entry.title));
    }
}
