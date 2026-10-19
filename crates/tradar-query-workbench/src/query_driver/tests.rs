use super::*;
/// Where `^` landed relative to the start of the query text on the
/// line above it, plus that line's own text -- what actually matters,
/// rather than the exact width of the `LINE N: ` prefix in front of
/// it.
pub(crate) fn caret_position(marker: &str) -> (usize, &str) {
    let mut lines = marker.lines();
    let line = lines.next().unwrap();
    let caret_line = lines.next().unwrap();
    let caret_column = caret_line.find('^').unwrap();
    // `line` is `LINE N: <text>` -- the first `": "` marks where
    // `<text>` starts.
    let text_start = line.find(": ").unwrap() + 2;
    (caret_column - text_start, &line[text_start..])
}
pub(crate) fn texts(sql: &str) -> Vec<String> {
    split_sql_statements(sql)
        .into_iter()
        .map(|s| s.text)
        .collect()
}
pub(crate) fn users_table() -> SchemaInfo {
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
pub(crate) fn key(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(c, v)| (c.to_string(), v.to_string()))
        .collect()
}

mod completion;
mod edit;
mod fetch;
mod misc;
mod snippet;
mod source;
mod sql_split;
