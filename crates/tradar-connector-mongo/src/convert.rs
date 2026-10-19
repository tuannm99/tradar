use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};

use tradar_query_workbench::query_driver::ColumnInfo;

/// Reformats MongoDB Extended JSON's `{"$oid": "<hex>"}` (an `ObjectId`)
/// and `{"$date": "<iso>"}` (a `DateTime`) into the same shell-literal
/// text `mongosh` itself prints them as -- `ObjectId("<hex>")` /
/// `ISODate("<iso>")` -- recursively through nested documents/arrays.
/// Deliberately *not* a bare string: an `ObjectId` and a plain string
/// field that happens to contain the same-looking hex text must stay
/// visually distinguishable (`ObjectId("507f...")` vs `"507f..."`), which
/// a bare-string unwrap -- tried first, then reverted after user feedback
/// -- would erase entirely. `into_relaxed_extjson()` -- and, identically,
/// `serde_json`'s own `Serialize` impl for `Bson` when a driver result
/// struct (`inserted_id`, ...) gets serialized -- still wraps these two
/// even in "relaxed" mode, since JSON has no native representation for
/// either. Safe to reformat here: this driver's results are read-only
/// display, never re-encoded back into BSON, and the type itself is also
/// visible separately via the navigator's schema (`Bson::element_type()`
/// in `flatten_document`). Any other extJSON wrapper (`$numberLong`,
/// `$numberDecimal`, `$binary`, ...) is left alone -- reformatting those
/// would lose a distinction (e.g. an int that overflowed `i32`) that
/// isn't purely cosmetic the way an id or a timestamp is.
pub(crate) fn simplify_extjson(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let reformatted = if map.len() == 1 {
                map.get("$oid")
                    .and_then(|v| v.as_str())
                    .map(|hex| format!("ObjectId(\"{hex}\")"))
                    .or_else(|| {
                        map.get("$date")
                            .and_then(|v| v.as_str())
                            .map(|iso| format!("ISODate(\"{iso}\")"))
                    })
            } else {
                None
            };
            match reformatted {
                Some(s) => *value = serde_json::Value::String(s),
                None => {
                    for v in map.values_mut() {
                        simplify_extjson(v);
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                simplify_extjson(v);
            }
        }
        _ => {}
    }
}

/// Every field named in any of this collection's indexes, `_id` excluded
/// (every collection has the implicit `_id_` index, which `primary_key`
/// already surfaces -- listing it again here would just be noise). Field
/// names come straight from `IndexModel::keys`, the same dotted-path form
/// `flatten_document` already produces for a nested field's own key
/// (Mongo's own index syntax, e.g. `{"address.city": 1}`), so a match
/// against `ColumnInfo::name` needs no extra parsing. Errors (no
/// permission to list indexes, a dropped collection) come back as "no
/// indexed fields known" rather than failing schema browsing over it --
/// same fallback `list_schema` already uses for a sample that fails to
/// load.
pub(crate) async fn indexed_field_names(
    collection: &mongodb::Collection<Document>,
) -> std::collections::HashSet<String> {
    let Ok(mut cursor) = collection.list_indexes().await else {
        return std::collections::HashSet::new();
    };
    let mut fields = std::collections::HashSet::new();
    while let Ok(Some(index)) = cursor.try_next().await {
        for key in index.keys.keys() {
            if key != "_id" {
                fields.insert(key.clone());
            }
        }
    }
    fields
}

pub(crate) fn json_to_document(value: serde_json::Value) -> anyhow::Result<Document> {
    let bson = Bson::try_from(value)?;
    bson.as_document()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected a JSON object"))
}

/// Flattens a sampled document into `parent.child` paths, the same way
/// `tradar-connector-elasticsearch` flattens a mapping's `properties` -- a nested
/// field is addressed that way in a query anyway. Arrays are reported as a
/// single `array` field rather than expanded per element: a doc's array
/// entries aren't guaranteed to share a shape, so there's no one type to
/// report for "the third element of every document's tags array".
pub(crate) fn flatten_document(prefix: &str, doc: &Document, out: &mut Vec<ColumnInfo>) {
    for (name, value) in doc {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            Bson::Document(nested) => flatten_document(&path, nested, out),
            other => {
                let mut column = ColumnInfo::new(path, bson_type_name(other));
                // `_id` is the closest thing a MongoDB document has to a
                // primary key -- every collection has exactly one, and it's
                // what a row edit in the results grid would key on if that
                // were ever built for Mongo.
                column.primary_key = name == "_id";
                out.push(column);
            }
        }
    }
}

/// The BSON element type's own name (`Bson`'s `Debug` derive already spells
/// it exactly this way: `Int32`, `ObjectId`, `String`, ...), passed through
/// rather than normalized -- same philosophy as `ColumnInfo::type_name`
/// elsewhere: show what the database actually says.
pub(crate) fn bson_type_name(value: &Bson) -> String {
    format!("{:?}", value.element_type())
}
