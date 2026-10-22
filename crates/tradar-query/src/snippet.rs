use super::*;

/// `columns` empty means "use `default`" -- every op's own natural
/// default, preserved so a caller that passes `&[]` (every caller before
/// the navigator grew a column picker) reproduces exactly the old
/// unconditional behavior. A selection that doesn't match any real column
/// (stale name, or a picker snapshot from before a schema reload) also
/// falls back to `default` rather than silently building an empty clause.
/// Shared by `build_crud_snippet` here and by the Mongo/Elasticsearch
/// connectors' own bespoke (non-SQL) `crud_snippet` bodies, so a picker
/// selection means the same thing everywhere it's honored. A free function
/// rather than a closure: a closure capturing `columns` while also being
/// called with two differently-lived slice arguments can't express "the
/// result borrows from whichever slice was picked" without a named
/// lifetime, which only a function signature (not a closure's elided one)
/// can carry.
pub fn pick_columns<'a>(
    columns: &[String],
    candidates: &[&'a str],
    default: &[&'a str],
) -> Vec<&'a str> {
    if columns.is_empty() {
        return default.to_vec();
    }
    let picked: Vec<&'a str> = candidates
        .iter()
        .copied()
        .filter(|c| columns.iter().any(|s| s.as_str() == *c))
        .collect();
    if picked.is_empty() {
        default.to_vec()
    } else {
        picked
    }
}

/// A skeleton Create/Read/Update/Delete statement for `entry`, shared by
/// the SQL connectors -- see `Component::crud_snippet`. Placeholders are
/// `<column_name>` rather than a guessed literal: there's no type mapping
/// reliable enough across every backend's own type names to invent a
/// plausible value, and "fill in this blank" reads better than a fake
/// value the user has to notice is fake and replace anyway.
///
/// `columns` is the navigator's column picker's selection -- empty means
/// "use this op's own default" (see `Component::crud_snippet`'s doc
/// comment), which is also what every call site that predates the picker
/// passes, so their output is unchanged. Read is the one op where an
/// empty *result* (not just an empty `columns` argument) is meaningful on
/// its own terms: it falls back to `SELECT *` rather than to some default
/// column list, since "no columns picked" and "show me the whole row" are
/// the same request for a read.
pub fn build_crud_snippet(entry: &SchemaInfo, op: CrudOp, columns: &[String]) -> String {
    let table = quote_identifier(&entry.name);
    let all_columns: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();
    let primary_key: Vec<&str> = entry
        .columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect();

    let where_clause = |keys: &[&str]| -> String {
        if keys.is_empty() {
            "<condition>".to_string()
        } else {
            keys.iter()
                .map(|c| format!("{} = <{c}>", quote_identifier(c)))
                .collect::<Vec<_>>()
                .join(" AND ")
        }
    };

    // One column per line rather than a single long comma-joined line --
    // a wide table's INSERT/UPDATE would otherwise run well past the
    // editor's width (it doesn't soft-wrap, same as vim).
    let bulleted = |items: &[String]| -> String {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let comma = if i + 1 < items.len() { "," } else { "" };
                format!("  {item}{comma}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    match op {
        CrudOp::Read => {
            let picked: Vec<&str> = all_columns
                .iter()
                .copied()
                .filter(|c| columns.iter().any(|s| s.as_str() == *c))
                .collect();
            if picked.is_empty() {
                format!("SELECT * FROM {table} LIMIT 100;")
            } else {
                let list = picked
                    .iter()
                    .map(|c| quote_identifier(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("SELECT {list} FROM {table} LIMIT 100;")
            }
        }
        CrudOp::Create => {
            if all_columns.is_empty() {
                format!("INSERT INTO {table} (<column>) VALUES (<value>);")
            } else {
                let insert_columns = pick_columns(columns, &all_columns, &all_columns);
                let names = bulleted(
                    &insert_columns
                        .iter()
                        .map(|c| quote_identifier(c))
                        .collect::<Vec<_>>(),
                );
                let placeholders = bulleted(
                    &insert_columns
                        .iter()
                        .map(|c| format!("<{c}>"))
                        .collect::<Vec<_>>(),
                );
                format!("INSERT INTO {table} (\n{names}\n) VALUES (\n{placeholders}\n);")
            }
        }
        CrudOp::Update => {
            let settable_default: Vec<&str> = all_columns
                .iter()
                .copied()
                .filter(|c| !primary_key.contains(c))
                .collect();
            let settable_default = if settable_default.is_empty() {
                all_columns.clone()
            } else {
                settable_default
            };
            let set_columns = pick_columns(columns, &all_columns, &settable_default);
            let set_clause = if set_columns.is_empty() {
                "  <column> = <value>".to_string()
            } else {
                bulleted(
                    &set_columns
                        .iter()
                        .map(|c| format!("{} = <{c}>", quote_identifier(c)))
                        .collect::<Vec<_>>(),
                )
            };
            format!(
                "UPDATE {table} SET\n{set_clause}\nWHERE {};",
                where_clause(&primary_key)
            )
        }
        CrudOp::Delete => {
            let where_columns = pick_columns(columns, &all_columns, &primary_key);
            format!(
                "DELETE FROM {table} WHERE {};",
                where_clause(&where_columns)
            )
        }
    }
}
