//! Turns the results grid's current `QueryResult` into CSV or JSON text for
//! `Ctrl+E` in `QueryScreenComponent` -- picked by the extension the user
//! types in the export prompt (`.csv` vs `.json`), the same idea as
//! `Ctrl+S` picking a query file's format by extension. Always exports the
//! full result, not whatever `/` currently has filtered the grid down to --
//! "export what I asked for" rather than "export what's on screen right
//! now", so a forgotten filter can't silently ship a partial file.

use crate::query_driver::QueryResult;

pub fn to_csv(result: &QueryResult) -> Result<String, String> {
    match result {
        QueryResult::Table { columns, rows, .. } => {
            let mut out = csv_line(columns.iter().map(String::as_str));
            for row in rows {
                out.push_str(&csv_line(row.iter().map(String::as_str)));
            }
            Ok(out)
        }
        QueryResult::Documents(_) => Err(
            "CSV export needs a table result -- this connector returns documents, use a .json path instead"
                .to_string(),
        ),
        QueryResult::Affected { .. } => Err("nothing to export".to_string()),
    }
}

pub fn to_json(result: &QueryResult) -> Result<String, String> {
    match result {
        QueryResult::Table { columns, rows, .. } => {
            let docs: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let fields: serde_json::Map<String, serde_json::Value> = columns
                        .iter()
                        .cloned()
                        .zip(row.iter().cloned().map(serde_json::Value::String))
                        .collect();
                    serde_json::Value::Object(fields)
                })
                .collect();
            serde_json::to_string_pretty(&docs).map_err(|e| e.to_string())
        }
        QueryResult::Documents(docs) => {
            serde_json::to_string_pretty(docs).map_err(|e| e.to_string())
        }
        QueryResult::Affected { .. } => Err("nothing to export".to_string()),
    }
}

/// One CSV row (RFC 4180-ish), newline included. A field is quoted only
/// when it needs to be -- containing a comma, quote, or newline -- with
/// embedded quotes doubled.
fn csv_line<'a>(fields: impl Iterator<Item = &'a str>) -> String {
    let mut line = fields.map(csv_field).collect::<Vec<_>>().join(",");
    line.push('\n');
    line
}

fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec!["1".to_string(), "Ada".to_string()],
                vec!["2".to_string(), "O'Brien, Sam".to_string()],
            ],
            truncated: false,
        }
    }

    #[test]
    fn csv_from_a_table_has_a_header_and_one_line_per_row() {
        let csv = to_csv(&table()).unwrap();

        assert_eq!(csv, "id,name\n1,Ada\n2,\"O'Brien, Sam\"\n");
    }

    #[test]
    fn csv_doubles_an_embedded_quote() {
        let result = QueryResult::Table {
            columns: vec!["value".to_string()],
            rows: vec![vec!["she said \"hi\"".to_string()]],
            truncated: false,
        };

        let csv = to_csv(&result).unwrap();

        assert_eq!(csv, "value\n\"she said \"\"hi\"\"\"\n");
    }

    #[test]
    fn csv_refuses_documents_with_a_clear_message() {
        let result = QueryResult::Documents(vec![serde_json::json!({"a": 1})]);

        let error = to_csv(&result).unwrap_err();

        assert!(error.contains(".json"), "error was: {error}");
    }

    #[test]
    fn csv_refuses_an_affected_result() {
        let result = QueryResult::Affected { rows: 3 };

        assert_eq!(to_csv(&result).unwrap_err(), "nothing to export");
    }

    #[test]
    fn json_from_a_table_is_an_array_of_row_objects() {
        let json = to_json(&table()).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([
                {"id": "1", "name": "Ada"},
                {"id": "2", "name": "O'Brien, Sam"}
            ])
        );
    }

    #[test]
    fn json_from_documents_passes_them_through_pretty_printed() {
        let result = QueryResult::Documents(vec![serde_json::json!({"a": 1})]);

        let json = to_json(&result).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, serde_json::json!([{"a": 1}]));
    }

    #[test]
    fn json_refuses_an_affected_result() {
        let result = QueryResult::Affected { rows: 3 };

        assert_eq!(to_json(&result).unwrap_err(), "nothing to export");
    }
}
