/// Infers a JSON value for a value as it's displayed in the grid or typed
/// into the row-edit prompt: text that parses as JSON (a number,
/// `true`/`false`, `null`, or a typed-out array/object literal) is used as
/// that value, anything else -- the common case, an ordinary string field
/// -- becomes a JSON string. Unlike Mongo's `mongo_value_literal`, there's
/// no constructor-call syntax (`ObjectId(...)`) to special-case first: ES
/// has no such wrapper convention, ids/dates are already plain strings.
pub(crate) fn es_infer_value(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

/// Wraps `value` in a nested JSON object for each `.`-separated segment of
/// `path`, innermost segment last -- `nest_dotted_path("customer.name", v)`
/// gives `{"customer": {"name": v}}`. A bare `path` with no dot gives
/// `{path: v}` unchanged. See `edit_sql`'s doc comment for why a dotted ES
/// column name has to become real nesting rather than a literal flat key.
/// Deep-merges `patch` into `target` (objects merge key by key, anything else
/// is replaced) -- how two dotted paths under one parent share its object.
pub(crate) fn merge_json(target: &mut serde_json::Value, patch: serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(into), serde_json::Value::Object(from)) => {
            for (key, value) in from {
                match into.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        into.insert(key, value);
                    }
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

pub(crate) fn nest_dotted_path(path: &str, value: serde_json::Value) -> serde_json::Value {
    path.rsplit('.').fold(value, |acc, segment| {
        let mut map = serde_json::Map::new();
        map.insert(segment.to_string(), acc);
        serde_json::Value::Object(map)
    })
}
