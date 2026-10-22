use tradar_query::QueryResult;

use crate::command::*;

/// The five Redis types the browse sidebar has a specialized view for --
/// see item 4 ("Redis: key browser") in `docs/backlog/mockup-ui-2026-08-15.md`. Streams and
/// other types aren't in scope yet: `parse` returns `None` for them, which
/// `browse_entry` turns into a clear "not supported" error rather than a
/// silent fallback to something misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowseKind {
    String,
    Hash,
    List,
    Set,
    Zset,
}

impl BrowseKind {
    pub(crate) fn parse(kind: &str) -> Option<Self> {
        Some(match kind {
            "string" => Self::String,
            "hash" => Self::Hash,
            "list" => Self::List,
            "set" => Self::Set,
            "zset" => Self::Zset,
            _ => return None,
        })
    }

    /// The command that fetches `key`'s full value for this type.
    pub(crate) fn command(self, key: &str) -> String {
        match self {
            Self::String => format!("GET {}", quote_arg(key)),
            Self::Hash => format!("HGETALL {}", quote_arg(key)),
            Self::List => format!("LRANGE {} 0 -1", quote_arg(key)),
            Self::Set => format!("SMEMBERS {}", quote_arg(key)),
            Self::Zset => format!("ZRANGE {} 0 -1 WITHSCORES", quote_arg(key)),
        }
    }
}

/// Converts `execute()`'s `Documents` reply for one of `BrowseKind`'s
/// commands into the `Table` shape the browse view wants (field/value
/// rows for a hash, index/value for a list, ...). Falls back to returning
/// `result` unchanged if it isn't the single-`Documents` shape `execute()`
/// always produces -- defensive, not expected to trigger.
pub(crate) fn reshape_for_browse(kind: BrowseKind, result: QueryResult) -> QueryResult {
    let QueryResult::Documents { items, .. } = &result else {
        return result;
    };
    let Some(value) = items.first() else {
        return result;
    };

    let (columns, rows): (Vec<String>, Vec<Vec<String>>) = match kind {
        BrowseKind::String => (vec!["value".to_string()], vec![vec![json_to_cell(value)]]),
        BrowseKind::Hash => (
            vec!["field".to_string(), "value".to_string()],
            value
                .as_object()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(field, v)| vec![field.clone(), json_to_cell(v)])
                        .collect()
                })
                .unwrap_or_default(),
        ),
        BrowseKind::List => (
            vec!["index".to_string(), "value".to_string()],
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, v)| vec![index.to_string(), json_to_cell(v)])
                        .collect()
                })
                .unwrap_or_default(),
        ),
        BrowseKind::Set => (
            vec!["member".to_string()],
            value
                .as_array()
                .map(|items| items.iter().map(|v| vec![json_to_cell(v)]).collect())
                .unwrap_or_default(),
        ),
        BrowseKind::Zset => (
            vec!["member".to_string(), "score".to_string()],
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|pair| {
                            let member = pair.get("member").map(json_to_cell).unwrap_or_default();
                            let score = pair.get("score").map(json_to_cell).unwrap_or_default();
                            vec![member, score]
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
    };

    QueryResult::Table {
        columns,
        rows,
        truncated: false,
    }
}

/// A JSON leaf as a plain grid cell: a string as-is, null as empty, and
/// anything else (number, bool) via its JSON text -- these only ever come
/// from `value_to_json`'s output, never nested objects/arrays.
pub(crate) fn json_to_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}
