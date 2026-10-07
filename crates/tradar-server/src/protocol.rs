//! Wire shapes: how `tradar-query-workbench`'s types become JSON. Kept apart
//! from `server.rs` so the protocol's vocabulary is readable in one place and
//! so the workbench crate never needs a `serde` dependency just for this.

use serde_json::{Value, json};

use tradar_query_workbench::query_driver::{
    ColumnInfo, QueryResult, RowChange, RowEdit, SchemaInfo, Statement,
};

pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// Anything a driver or the server itself reports as a failed call -- the
/// message is the same text the TUI would have shown.
pub const APP_ERROR: i64 = -32000;

pub struct RpcError {
    pub code: i64,
    pub message: String,
    /// Structured detail a client can use without parsing `message`
    /// (currently `{line, column}` for a located SQL error).
    pub data: Option<Value>,
}

impl RpcError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    pub fn app(message: impl Into<String>) -> Self {
        Self {
            code: APP_ERROR,
            message: message.into(),
            data: None,
        }
    }

    /// A driver failure. When its text carries the `LINE N: ...` / `^`
    /// marker the drivers already render (see `line_and_caret`), the
    /// position is lifted into `data` so an editor can underline the exact
    /// spot instead of showing the marker as prose.
    pub fn from_query_error(message: String) -> Self {
        let data =
            error_position(&message).map(|(line, column)| json!({"line": line, "column": column}));
        Self {
            code: APP_ERROR,
            message,
            data,
        }
    }
}

/// `(line, column)` of the caret in a `LINE N: <text>` / `   ^` marker:
/// `line` is 1-based within the statement that was sent, `column` is a
/// 0-based **character** offset within that line. `None` when the message
/// has no such marker. Reads the rendered text rather than asking each
/// connector for a structured position -- the marker is the one shape all
/// five located drivers already agree on.
pub fn error_position(message: &str) -> Option<(usize, usize)> {
    let mut lines = message.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("LINE ") else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let Some(after) = rest[digits.len()..].strip_prefix(": ") else {
            continue;
        };
        let line_number: usize = digits.parse().ok()?;
        let _ = after;
        let prefix_len = "LINE ".len() + digits.len() + ": ".len();
        let caret_line = lines.peek()?;
        let caret_at = caret_line.chars().position(|c| c == '^')?;
        return Some((line_number, caret_at.checked_sub(prefix_len)?));
    }
    None
}

impl From<anyhow::Error> for RpcError {
    fn from(error: anyhow::Error) -> Self {
        Self::app(error.to_string())
    }
}

pub fn column_json(column: &ColumnInfo) -> Value {
    json!({
        "name": column.name,
        "type": column.type_name,
        "primary_key": column.primary_key,
        "indexed": column.indexed,
        "foreign_key": column.foreign_key.as_ref().map(|fk| json!({
            "table": fk.table,
            "column": fk.column,
        })),
    })
}

pub fn schema_json(entry: &SchemaInfo) -> Value {
    json!({
        "name": entry.name,
        "kind": entry.kind,
        "ttl": entry.ttl,
        "schema": entry.schema,
        "object_kind": entry.object_kind,
        "columns": entry.columns.iter().map(column_json).collect::<Vec<_>>(),
    })
}

pub fn statement_json(statement: &Statement) -> Value {
    json!({
        "text": statement.text,
        "start": statement.start,
        "end": statement.end,
    })
}

/// A result kept server-side so a client pulls only the window it is
/// showing -- the terminal-first "virtual scrolling" rule applied across a
/// process boundary instead of inside one component.
pub enum StoredResult {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        truncated: bool,
    },
    Documents {
        items: Vec<Value>,
        truncated: bool,
    },
}

impl StoredResult {
    /// `Affected` has no rows to page, so it never becomes a cursor; the
    /// caller answers it inline.
    pub fn from_query(result: QueryResult) -> Result<Self, u64> {
        match result {
            QueryResult::Table {
                columns,
                rows,
                truncated,
            } => Ok(Self::Table {
                columns,
                rows,
                truncated,
            }),
            QueryResult::Documents { items, truncated } => Ok(Self::Documents { items, truncated }),
            QueryResult::Affected { rows } => Err(rows),
        }
    }

    pub fn total(&self) -> usize {
        match self {
            Self::Table { rows, .. } => rows.len(),
            Self::Documents { items, .. } => items.len(),
        }
    }

    pub fn truncated(&self) -> bool {
        match self {
            Self::Table { truncated, .. } | Self::Documents { truncated, .. } => *truncated,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Table { .. } => "table",
            Self::Documents { .. } => "documents",
        }
    }

    /// `offset..offset+limit`, clamped to what exists -- asking past the end
    /// is an empty page, not an error, so a client can over-fetch while
    /// scrolling.
    pub fn page(&self, offset: usize, limit: usize) -> Value {
        match self {
            Self::Table { rows, .. } => {
                let end = offset.saturating_add(limit).min(rows.len());
                let start = offset.min(end);
                json!(&rows[start..end])
            }
            Self::Documents { items, .. } => {
                let end = offset.saturating_add(limit).min(items.len());
                let start = offset.min(end);
                json!(&items[start..end])
            }
        }
    }

    pub fn columns(&self) -> Option<&[String]> {
        match self {
            Self::Table { columns, .. } => Some(columns),
            Self::Documents { .. } => None,
        }
    }

    /// One ordered list of top-level field names per document in the page
    /// (`null` for a document that isn't a JSON object), `None` for a
    /// `Table` -- it already has `columns()`, which says the same thing
    /// once for the whole result rather than per row.
    ///
    /// Exists because the plugin's own JSON decode (`vim.json.decode`,
    /// Neovim's Lua binding) can't recover this on its own: a decoded
    /// object becomes a plain Lua table, and `pairs()` over a Lua table
    /// has no defined order (`render.flatten`'s own doc comment already
    /// notes this -- "Lua's JSON decode does not keep key order"). Sent as
    /// a sibling field alongside the page rather than changing the shape
    /// of the documents themselves, so a client that doesn't care (the
    /// JSON view, `yank`/export, every other RPC caller) reads the exact
    /// same `rows` it always has.
    ///
    /// Relies on `serde_json`'s `preserve_order` feature (see
    /// `tradar-connector-mongo/Cargo.toml`) -- without it `Value::Object`
    /// is a `BTreeMap` and this would just report alphabetical order back,
    /// no better than a client re-deriving it.
    pub fn field_order(&self, offset: usize, limit: usize) -> Option<Value> {
        let Self::Documents { items, .. } = self else {
            return None;
        };
        let end = offset.saturating_add(limit).min(items.len());
        let start = offset.min(end);
        Some(json!(
            items[start..end]
                .iter()
                .map(|item| item.as_object().map(|map| map.keys().collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        ))
    }
}

/// `{"table": "t", "key": {"id": "1"}, "change": {"set": {"column": "name", "value": "x"}}}`
/// or `"change": "delete"`. The key is an object, not pairs, because a
/// column name can only appear once in a row's key anyway.
pub fn parse_row_edit(params: &Value) -> Result<RowEdit, RpcError> {
    let table = params
        .get("table")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid("`table` must be a string"))?
        .to_string();
    let key = params
        .get("key")
        .and_then(Value::as_object)
        .ok_or_else(|| RpcError::invalid("`key` must be an object of column -> value"))?
        .iter()
        .map(|(column, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| RpcError::invalid(format!("`key.{column}` must be a string")))?;
            Ok((column.clone(), value.to_string()))
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let change = match params.get("change") {
        Some(Value::String(s)) if s == "delete" => RowChange::DeleteRow,
        Some(Value::Object(object)) => {
            let set = object
                .get("set")
                .ok_or_else(|| RpcError::invalid("`change` must be \"delete\" or {set: {...}}"))?;
            let column = set.get("column").and_then(Value::as_str);
            let value = set.get("value").and_then(Value::as_str);
            match (column, value) {
                (Some(column), Some(value)) => RowChange::SetValue {
                    column: column.to_string(),
                    value: value.to_string(),
                },
                _ => {
                    return Err(RpcError::invalid(
                        "`change.set` needs string `column` and `value`",
                    ));
                }
            }
        }
        _ => {
            return Err(RpcError::invalid(
                "`change` must be \"delete\" or {set: {column, value}}",
            ));
        }
    };
    Ok(RowEdit { table, key, change })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_caret_in_a_rendered_marker() {
        let message = "error: syntax error at or near \"FRO\"\nLINE 2:   FRO users\n          ^";
        assert_eq!(error_position(message), Some((2, 2)));
    }

    #[test]
    fn a_message_without_a_marker_has_no_position() {
        assert_eq!(error_position("connection refused"), None);
        assert_eq!(error_position("LINE x: nope\n  ^"), None);
    }

    #[test]
    fn field_order_reports_each_document_s_own_top_level_keys_in_order() {
        let stored = StoredResult::Documents {
            items: vec![
                json!({"b": 1, "a": 2, "_id": "x"}),
                json!({"name": "ann", "age": 3}),
            ],
            truncated: false,
        };

        let order = stored.field_order(0, 10).unwrap();

        assert_eq!(order, json!([["b", "a", "_id"], ["name", "age"]]));
    }

    #[test]
    fn field_order_is_none_for_a_table_result() {
        let stored = StoredResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        };

        assert_eq!(stored.field_order(0, 10), None);
    }

    #[test]
    fn field_order_is_null_for_a_document_that_is_not_an_object() {
        let stored = StoredResult::Documents {
            items: vec![json!(42)],
            truncated: false,
        };

        assert_eq!(stored.field_order(0, 10).unwrap(), json!([null]));
    }

    #[test]
    fn field_order_respects_the_same_offset_and_limit_window_as_page() {
        let stored = StoredResult::Documents {
            items: vec![json!({"a": 1}), json!({"b": 2}), json!({"c": 3})],
            truncated: false,
        };

        let order = stored.field_order(1, 1).unwrap();

        assert_eq!(order, json!([["b"]]));
    }
}
