use crate::protocol::{
    RpcError, order_insert_values, parse_insert_values, parse_row_edit, statement_json, str_param,
};
use serde_json::{Value, json};
use tradar_query_workbench::components::completion::CandidateKind;
use tradar_query_workbench::query_driver::completion_context;

use super::*;

impl Server {
    pub(super) fn split(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let text = str_param(params, "text")?;
        let statements = driver.split_statements(text);
        Ok(json!(
            statements.iter().map(statement_json).collect::<Vec<_>>()
        ))
    }

    pub(super) fn keywords(&self, params: &Value) -> Result<Value, RpcError> {
        Ok(json!(self.driver(params)?.keywords()))
    }

    /// `text` is everything from the start of the buffer up to the cursor
    /// (context -- aliases, `JOIN`, a Mongo `db.<coll>.find({` -- needs the
    /// earlier lines, not just the current one). The partial word being
    /// typed is derived here, with the same word characters the TUI editor
    /// uses, so both clients complete identically.
    pub(super) fn complete(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let text = str_param(params, "text")?;
        let source = self
            .state
            .lock()
            .unwrap()
            .completions
            .get(name)
            .cloned()
            .ok_or_else(|| {
                RpcError::app(format!("`{name}` is not connected -- call `connect` first"))
            })?;
        let prefix_start = text
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
            .last()
            .map_or(text.len(), |(i, _)| i);
        let prefix = &text[prefix_start..];
        let context = completion_context(text);
        let items: Vec<Value> = source
            .matches_in_context(prefix, &context)
            .into_iter()
            .map(|c| {
                let kind = match c.kind {
                    CandidateKind::Keyword => "keyword",
                    CandidateKind::Table => "table",
                    CandidateKind::Column => "column",
                };
                json!({"text": c.text, "kind": kind})
            })
            .collect();
        Ok(json!({"prefix": prefix, "items": items}))
    }

    pub(super) fn edit_source(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let query = str_param(params, "query")?;
        let source = driver.edit_source(query);
        let keys = source.as_deref().and_then(|s| driver.edit_key_columns(s));
        // Only worth asking once the query isn't already one table -- a
        // `JOIN`-only question, and the caller (the results grid) is the
        // one place that already knows the result's own column names,
        // which this doesn't otherwise have a reason to track server-side.
        let column_sources = source.is_none().then(|| {
            let columns: Vec<String> = params
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            driver.column_sources(query, &columns)
        });
        Ok(json!({
            "table": source,
            "key_columns": keys,
            "column_sources": column_sources.flatten(),
        }))
    }

    /// Only *builds* the statement; running it is the client's explicit
    /// follow-up `execute`, which keeps the TUI's "show it, run it only
    /// after a `y`" rule instead of hiding a write inside this call.
    pub(super) fn edit_sql(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        if let Some(insert) = params.get("insert") {
            let table = str_param(params, "table")?;
            let values =
                order_insert_values(parse_insert_values(insert)?, params.get("insert_columns"));
            return Ok(json!({"sql": driver.insert_sql(table, &values)}));
        }
        let edit = parse_row_edit(params)?;
        Ok(json!({"sql": driver.edit_sql(&edit)}))
    }
}
