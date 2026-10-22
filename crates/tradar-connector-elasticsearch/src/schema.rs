use tradar_query::ColumnInfo;

/// The fields of one index, read out of a `GET /_mapping` response.
pub(crate) fn index_fields(mappings: &serde_json::Value, index: &str) -> Vec<ColumnInfo> {
    let mut fields = Vec::new();
    if let Some(properties) = mappings
        .get(index)
        .and_then(|m| m.get("mappings"))
        .and_then(|m| m.get("properties"))
    {
        flatten_properties("", properties, &mut fields);
    }
    fields
}

/// Flattens a mapping's `properties` into `parent.child` paths, which is
/// how you refer to a nested field in a query anyway. An object node has
/// no `type` of its own, only more `properties`; a leaf's `type` is
/// reported as-is. Multi-fields (`fields`) are skipped: `title.keyword` is
/// an indexing detail rather than a field of the document.
pub(crate) fn flatten_properties(
    prefix: &str,
    properties: &serde_json::Value,
    out: &mut Vec<ColumnInfo>,
) {
    let Some(properties) = properties.as_object() else {
        return;
    };
    for (name, definition) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match definition.get("properties") {
            Some(nested) => flatten_properties(&path, nested, out),
            None => out.push(ColumnInfo::new(
                path,
                definition
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("object"),
            )),
        }
    }
}
