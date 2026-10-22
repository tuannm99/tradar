use tradar_query::{self as query_driver};

/// Whether `value`'s own text reads as a CQL identifier/UUID literal --
/// the hyphenated hex shape (`8-4-4-4-12`) CQL accepts unquoted for both
/// `uuid` and `timeuuid` columns alike (textually identical; which of the
/// two doesn't change how it's written).
pub(crate) fn looks_like_cql_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    [8, 4, 4, 4, 12]
        == groups
            .iter()
            .map(|g| g.len())
            .collect::<Vec<_>>()
            .as_slice()
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A `RowEdit` value, written the way CQL's own literal grammar needs --
/// unlike SQL, CQL does **not** coerce a quoted string to a number/uuid/
/// boolean for comparison (`query_driver::build_sql_edit`'s own doc
/// comment explains why SQL gets away with always quoting); CQL's parser
/// instead rejects the statement outright (`Invalid STRING constant ...
/// for int`). There is no schema to consult here either -- a column's
/// declared CQL type isn't available to this sync method without a round
/// trip -- so, same reason `tradar-connector-mongo`'s `mongo_value_literal`
/// and Elasticsearch's own `edit_sql` already infer from the value's own
/// text instead of a schema lookup, this does too: a bare `true`/`false`,
/// something that parses as a number, or a UUID-shaped string are written
/// unquoted; everything else -- including a collection literal
/// (`list<...>`/`set<...>`/`map<...>`), which CQL forbids inside a
/// `PRIMARY KEY` anyway, so a key column is never one -- is quoted as a
/// plain CQL string. Narrow, known miss: literally typing the text `NaN`/
/// `inf`/`infinity` into a text column's edit prompt is written unquoted
/// (`str::parse::<f64>` itself accepts those spellings) -- valid CQL for
/// `float`/`double`, a type error for `text`; same class of edge case
/// `mongo_value_literal` already has for the literal text `null`.
pub(crate) fn cql_value_literal(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
        return trimmed.to_ascii_lowercase();
    }
    if trimmed.parse::<f64>().is_ok() || looks_like_cql_uuid(trimmed) {
        return trimmed.to_string();
    }
    format!("'{}'", trimmed.replace('\'', "''"))
}

/// `query_driver::build_sql_edit`, with CQL's own literal rules
/// (`cql_value_literal`) in place of SQL's always-quote-it shortcut --
/// otherwise the identical `UPDATE ... SET ... WHERE ...`/`DELETE FROM
/// ... WHERE ...` shape, since CQL's DML syntax is close enough to SQL to
/// share it (same reason this driver already reuses
/// `SQL_KEYWORDS`/`split_sql_statements`). A `WHERE` built from equality
/// on every key column (`edit.key`, which is every `partition_key`/
/// `clustering` column together -- see `list_schema`'s own comment on
/// `ColumnInfo::primary_key`) is valid CQL for addressing exactly one
/// row; the composite-key tuple syntax (`(a, b) = (...)`) is only needed
/// for `IN`/range queries, not plain equality.
pub(crate) fn build_cql_edit(edit: &query_driver::RowEdit) -> String {
    let table = query_driver::quote_identifier(&edit.table);
    let where_clause = edit
        .key
        .iter()
        .map(|(column, value)| {
            if value == "NULL" {
                format!("{} IS NULL", query_driver::quote_identifier(column))
            } else {
                format!(
                    "{} = {}",
                    query_driver::quote_identifier(column),
                    cql_value_literal(value)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    match &edit.change {
        query_driver::RowChange::SetValue { column, value } => format!(
            "UPDATE {table} SET {} = {} WHERE {where_clause}",
            query_driver::quote_identifier(column),
            cql_value_literal(value)
        ),
        query_driver::RowChange::SetValues(pairs) => format!(
            "UPDATE {table} SET {} WHERE {where_clause}",
            pairs
                .iter()
                .map(|(column, value)| format!(
                    "{} = {}",
                    query_driver::quote_identifier(column),
                    cql_value_literal(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        query_driver::RowChange::DeleteRow => format!("DELETE FROM {table} WHERE {where_clause}"),
    }
}
