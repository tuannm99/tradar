/// A JS object key for `field` -- bare when it's a valid identifier, quoted
/// otherwise (a dotted path like `address.city` from a nested document, see
/// `flatten_document`). `db.x.find({...})` snippets are read by a real
/// JS-ish parser (`mongosh`), not just displayed, so an unquoted dot would
/// produce a syntax error rather than a working query with a wrong field.
pub(crate) fn mongo_field_key(field: &str) -> String {
    let is_bare = field
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && field
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if is_bare {
        field.to_string()
    } else {
        format!("\"{}\"", field.replace('"', "\\\""))
    }
}

/// One level of the tree `nested_insert_literal` groups dotted field paths
/// into -- a leaf holds the *full* original field path (what `value_for`
/// needs to pick the right placeholder), a group holds its children in
/// first-seen order.
pub(crate) enum InsertField {
    Leaf(String),
    Group(Vec<(String, InsertField)>),
}

/// Inserts `field` (one of `nested_insert_literal`'s fields, e.g.
/// `"address.city"`) into the tree being built, splitting on `.` one
/// level at a time and descending into (or creating) the group for each
/// segment but the last. `remaining` shrinks on each recursive call;
/// `original` stays the full path, for the leaf to carry at the bottom.
pub(crate) fn insert_field_path(
    roots: &mut Vec<(String, InsertField)>,
    remaining: &str,
    original: &str,
) {
    match remaining.split_once('.') {
        None => roots.push((
            remaining.to_string(),
            InsertField::Leaf(original.to_string()),
        )),
        Some((head, rest)) => {
            if let Some((_, InsertField::Group(children))) =
                roots.iter_mut().find(|(key, _)| key == head)
            {
                insert_field_path(children, rest, original);
                return;
            }
            let mut children = Vec::new();
            insert_field_path(&mut children, rest, original);
            roots.push((head.to_string(), InsertField::Group(children)));
        }
    }
}

/// A nested JS object literal for `insertOne`'s document, merging dotted
/// field paths that share a prefix (`address.city`, `address.zip`) into
/// one real nested object -- `{address: {city: <value>, zip: <value>}}`,
/// not a single flat field literally named `"address.city"`. Unlike
/// `object_literal` (used by Update/Delete, where Mongo itself reads a
/// dotted key in `$set`/a filter as a path expression, so quoting it flat
/// there is already correct), `insertOne` has no such convention: a dotted
/// key in the document it inserts is just a field name with a literal dot
/// in it, not a path -- exactly the mismatch the navigator's own dotted
/// names (from `flatten_document` sampling a real nested document) would
/// otherwise reproduce as a wrong Create snippet.
pub(crate) fn nested_insert_literal(fields: &[&str], value_for: &dyn Fn(&str) -> String) -> String {
    let mut roots: Vec<(String, InsertField)> = Vec::new();
    for &field in fields {
        insert_field_path(&mut roots, field, field);
    }
    render_insert_group(&roots, value_for, 1)
}

pub(crate) fn render_insert_node(
    node: &InsertField,
    value_for: &dyn Fn(&str) -> String,
    indent: usize,
) -> String {
    match node {
        InsertField::Leaf(field) => value_for(field),
        InsertField::Group(children) => render_insert_group(children, value_for, indent),
    }
}

/// Same one-per-line-once-there's-more-than-one shape `object_literal`
/// uses, except a multi-field group's closing `}` lands one level
/// shallower than its own body, and a nested group recurses with one more
/// level of indent -- `indent` counts 2-space units from the outermost
/// `{`.
pub(crate) fn render_insert_group(
    children: &[(String, InsertField)],
    value_for: &dyn Fn(&str) -> String,
    indent: usize,
) -> String {
    match children {
        [] => "{}".to_string(),
        [(key, node)] => format!(
            "{{{}: {}}}",
            mongo_field_key(key),
            render_insert_node(node, value_for, indent)
        ),
        children => {
            let pad = "  ".repeat(indent);
            let closing_pad = "  ".repeat(indent - 1);
            let body = children
                .iter()
                .map(|(key, node)| {
                    format!(
                        "{pad}{}: {}",
                        mongo_field_key(key),
                        render_insert_node(node, value_for, indent + 1)
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{body}\n{closing_pad}}}")
        }
    }
}

/// `{_id: ObjectId("..."), ...}` -- every `(column, displayed value)` pair
/// in a `RowEdit::key` as one mongosh filter object, values run through
/// `mongo_value_literal`. In practice this is always just `_id` (the one
/// column `list_schema` marks as a key), but built generically the same
/// shape `build_sql_edit`'s `WHERE` clause is.
pub(crate) fn mongo_filter_literal(key: &[(String, String)]) -> String {
    let body = key
        .iter()
        .map(|(column, value)| {
            format!(
                "{}: {}",
                mongo_field_key(column),
                mongo_value_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{body}}}")
}

/// Infers a mongosh literal from a value as it's displayed in the grid or
/// typed into the row-edit prompt. Unlike SQL (`build_sql_edit`'s own doc
/// comment: Postgres/SQLite coerce a quoted `'1'` to a number where a
/// number is wanted), a quoted JS string is never silently reinterpreted as
/// another type -- `{_id: "ObjectId(...)"}"` would filter for that literal
/// *string*, not call the constructor, so quoting everything as SQL does
/// would break every structured field.
///
/// `ObjectId("...")`/`ISODate("...")` (exactly the shape `simplify_extjson`
/// formats a document's own such fields as, so a key's displayed value
/// round-trips straight back into a working filter) pass through as-is;
/// anything else that parses as JSON -- a number, `true`/`false`, `null`,
/// or a typed-out array/object literal -- also passes through as-is; every
/// other text (the common case, an ordinary string field) is JSON-quoted.
/// A known, deliberate limitation of inferring type from typed text rather
/// than asking the user to pick one: a `Decimal128`/`Binary`/other BSON
/// type with no bare-text mongosh literal of its own has no way to be
/// entered here and is sent as a plain string instead.
pub(crate) fn mongo_value_literal(value: &str) -> String {
    let trimmed = value.trim();
    let is_constructor_call = |prefix: &str| {
        trimmed.starts_with(prefix) && trimmed.ends_with("\")") && trimmed.contains('"')
    };
    if is_constructor_call("ObjectId(\"") || is_constructor_call("ISODate(\"") {
        return trimmed.to_string();
    }
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// The "generate" menu for a collection: the four basic statements plus the
/// shapes people reach for first -- see `mongo_extra_snippet`.
pub(crate) fn mongo_snippet_options() -> Vec<tradar_query::SnippetOption> {
    use tradar_query::SnippetOption as O;
    vec![
        O {
            id: "read",
            label: "Find",
            hint: "find({}) -- all documents",
        },
        O {
            id: "find_latest",
            label: "Find latest",
            hint: "newest 20 by _id",
        },
        O {
            id: "find_by_id",
            label: "Find by _id",
            hint: "one document by its _id",
        },
        O {
            id: "find_one",
            label: "Find one",
            hint: "findOne({})",
        },
        O {
            id: "count",
            label: "Count",
            hint: "countDocuments({})",
        },
        O {
            id: "distinct",
            label: "Distinct",
            hint: "the distinct values of a field",
        },
        O {
            id: "aggregate",
            label: "Aggregate",
            hint: "$match / $group / $sort skeleton",
        },
        O {
            id: "create",
            label: "Insert",
            hint: "insertOne",
        },
        O {
            id: "update",
            label: "Update",
            hint: "updateOne with $set",
        },
        O {
            id: "delete",
            label: "Delete",
            hint: "deleteOne",
        },
    ]
}

/// The statements of `mongo_snippet_options` beyond the four `crud_snippet`
/// builds. A field name comes from the collection's first field after
/// `_id` when the schema knows one, else a `<field>` placeholder.
pub(crate) fn mongo_extra_snippet(entry: &tradar_query::SchemaInfo, id: &str) -> Option<String> {
    let collection = &entry.name;
    let field = entry
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .find(|name| *name != "_id")
        .unwrap_or("<field>");
    Some(match id {
        "find_latest" => format!("db.{collection}.find({{}}).sort({{_id: -1}}).limit(20)"),
        "find_by_id" => format!("db.{collection}.find({{_id: ObjectId(\"<id>\")}})"),
        "find_one" => format!("db.{collection}.findOne({{}})"),
        "count" => format!("db.{collection}.countDocuments({{}})"),
        "distinct" => format!("db.{collection}.distinct(\"{field}\")"),
        "aggregate" => format!(
            "db.{collection}.aggregate([\n  {{$match: {{}}}},\n  {{$group: {{_id: \"${field}\", n: {{$sum: 1}}}}}},\n  {{$sort: {{n: -1}}}}\n])"
        ),
        _ => return None,
    })
}
