use super::*;
use testcontainers_modules::elastic_search::ElasticSearch;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tradar_query_workbench::query_driver::{
    self as query_driver, ColumnInfo, QueryDriver, QueryResult, SchemaInfo,
};
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
