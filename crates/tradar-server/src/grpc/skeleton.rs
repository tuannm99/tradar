use super::*;

/// A request body to start from: `method`'s input message as indented JSON
/// with every field present at its default (`\"\"`, `0`, `false`, the first
/// enum value, `[]`, `{}`, nested messages filled in). Written by hand
/// rather than through `serde_json::Value` so the fields stay in the
/// order the `.proto` declares them. Only the first member of a `oneof` is
/// included (the others would make the request invalid), and a message
/// that contains itself stops being expanded at the second level.
///
/// With `comments`, a `// ...` line goes above a field that has something to
/// say: its `.proto` doc comment (only known for a `.proto` loaded by hand --
/// reflection does not carry comments), the values of an enum, the other
/// members of a `oneof`. That is no longer valid JSON; the plugin drops
/// whole-line `//` comments before sending.
pub fn skeleton(
    pool: &DescriptorPool,
    service: &str,
    method: &str,
    comments: bool,
) -> anyhow::Result<String> {
    let method = get_method(pool, service, method)?;
    Ok(message_skeleton(
        &method.input(),
        &mut Vec::new(),
        0,
        comments,
    ))
}

/// The leading doc comment of `field` as `.proto` source wrote it, if the
/// file was loaded with source info.
pub(crate) fn field_doc(field: &FieldDescriptor) -> Option<String> {
    let file = field.parent_file();
    let info = file.file_descriptor_proto().source_code_info.as_ref()?;
    let location = info.location.iter().find(|l| l.path == field.path())?;
    let text = location.leading_comments.as_deref()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub(crate) fn message_skeleton(
    desc: &MessageDescriptor,
    stack: &mut Vec<String>,
    depth: usize,
    comments: bool,
) -> String {
    // Well-known types have a fixed JSON shape of their own.
    match desc.full_name() {
        "google.protobuf.Timestamp" => return "\"1970-01-01T00:00:00Z\"".into(),
        "google.protobuf.Duration" => return "\"0s\"".into(),
        "google.protobuf.FieldMask"
        | "google.protobuf.StringValue"
        | "google.protobuf.BytesValue" => {
            return "\"\"".into();
        }
        "google.protobuf.BoolValue" => return "false".into(),
        "google.protobuf.Int32Value"
        | "google.protobuf.UInt32Value"
        | "google.protobuf.Int64Value"
        | "google.protobuf.UInt64Value"
        | "google.protobuf.FloatValue"
        | "google.protobuf.DoubleValue" => return "0".into(),
        "google.protobuf.Value" => return "null".into(),
        "google.protobuf.ListValue" => return "[]".into(),
        "google.protobuf.Struct" | "google.protobuf.Any" | "google.protobuf.Empty" => {
            return "{}".into();
        }
        _ => {}
    }
    let name = desc.full_name().to_string();
    if stack.contains(&name) || depth > 4 {
        return "{}".into();
    }
    stack.push(name);
    let indent = "  ".repeat(depth + 1);
    // (comment lines, the field itself) -- commas go on the field line only.
    let mut fields: Vec<(Vec<String>, String)> = Vec::new();
    for field in desc.fields() {
        let mut notes = Vec::new();
        if let Some(oneof) = field.containing_oneof() {
            let first = oneof.fields().next().map(|f| f.number());
            if first != Some(field.number()) {
                continue;
            }
            if comments && oneof.fields().count() > 1 {
                let names: Vec<_> = oneof.fields().map(|f| f.json_name().to_string()).collect();
                notes.push(format!(
                    "oneof {}: {} (only one)",
                    oneof.name(),
                    names.join(" | ")
                ));
            }
        }
        if comments {
            if let Some(doc) = field_doc(&field) {
                notes.splice(0..0, doc.lines().map(|l| l.trim().to_string()));
            }
            if let Kind::Enum(e) = field.kind() {
                let values: Vec<_> = e.values().map(|v| v.name().to_string()).collect();
                notes.push(format!("enum {}: {}", e.name(), values.join(" | ")));
            }
        }
        fields.push((
            notes,
            format!(
                "{indent}{}: {}",
                serde_json::Value::String(field.json_name().to_string()),
                field_skeleton(&field, stack, depth, comments)
            ),
        ));
    }
    stack.pop();
    if fields.is_empty() {
        return "{}".into();
    }
    let last = fields.len() - 1;
    let mut lines = Vec::new();
    for (i, (notes, field)) in fields.into_iter().enumerate() {
        lines.extend(notes.into_iter().map(|n| format!("{indent}// {n}")));
        lines.push(if i == last {
            field
        } else {
            format!("{field},")
        });
    }
    format!("{{\n{}\n{}}}", lines.join("\n"), "  ".repeat(depth))
}

pub(crate) fn field_skeleton(
    field: &FieldDescriptor,
    stack: &mut Vec<String>,
    depth: usize,
    comments: bool,
) -> String {
    if field.is_map() {
        return "{}".into();
    }
    if field.is_list() {
        return "[]".into();
    }
    match field.kind() {
        Kind::Double
        | Kind::Float
        | Kind::Int32
        | Kind::Int64
        | Kind::Uint32
        | Kind::Uint64
        | Kind::Sint32
        | Kind::Sint64
        | Kind::Fixed32
        | Kind::Fixed64
        | Kind::Sfixed32
        | Kind::Sfixed64 => "0".into(),
        Kind::Bool => "false".into(),
        Kind::String | Kind::Bytes => "\"\"".into(),
        Kind::Enum(e) => {
            let first = e
                .values()
                .next()
                .map_or(String::new(), |v| v.name().to_string());
            serde_json::Value::String(first).to_string()
        }
        Kind::Message(m) => message_skeleton(&m, stack, depth + 1, comments),
    }
}
