use super::*;

/// Qualifies `entry.name` with its `entry.schema` prefix (`"schema.table"`)
/// only for entries whose bare name collides with another entry from a
/// *different* schema/database in the same list -- every entry not
/// involved in a collision keeps its plain name unchanged. Called by a
/// driver that groups `list_schema` by schema/database (Postgres, MongoDB)
/// after building the full list, once, across every kind combined.
///
/// This matters because `SchemaInfo.name` doubles as the identifier
/// `build_crud_snippet`/`column_types` match a query's own `FROM` clause
/// against: a bare `FROM users` resolves through the connection's own
/// default schema search, so keeping the common case's name bare (a
/// single schema, or a name that happens to be unique across several) is
/// what keeps that matching working exactly as it did before a
/// schema/database level existed at all. Qualifying only the rare
/// colliding case is what keeps a lookup from silently picking the wrong
/// one of two same-named tables in different schemas.
pub fn qualify_colliding_names(entries: &mut [SchemaInfo]) {
    // Owned keys, not borrowed from `entries` -- a borrow would still be
    // held by the time the loop below needs `entries` mutably.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for entry in entries.iter() {
        *counts.entry(entry.name.clone()).or_insert(0) += 1;
    }
    for entry in entries.iter_mut() {
        let colliding = counts.get(entry.name.as_str()).copied().unwrap_or(0) > 1;
        if colliding && let Some(schema) = &entry.schema {
            entry.name = format!("{schema}.{}", entry.name);
        }
    }
}

/// Whether `query_name` (as written in query text, or in FK data) and
/// `schema_name` (a connected `SchemaInfo::name`) name the same table.
/// Compares case-insensitively and, since either side may or may not carry
/// a schema qualifier (`public.users` vs. bare `users` -- see
/// `qualify_colliding_names`), falls back to comparing each side's last
/// dotted segment before giving up. Shared by completion (`alias.` and
/// JOIN-target ranking) and the ERD view (walking FK references), both of
/// which match text that may or may not be schema-qualified against
/// `SchemaInfo` entries that may or may not be either.
pub fn same_table(query_name: &str, schema_name: &str) -> bool {
    if query_name.eq_ignore_ascii_case(schema_name) {
        return true;
    }
    let query_last = query_name.rsplit('.').next().unwrap_or(query_name);
    let schema_last = schema_name.rsplit('.').next().unwrap_or(schema_name);
    query_last.eq_ignore_ascii_case(schema_name) || query_name.eq_ignore_ascii_case(schema_last)
}

/// The one table a `SELECT` reads from, or `None` when the answer isn't a
/// single table -- a join, a union, a subquery, a grouped or distinct
/// result, or anything that isn't a `SELECT` at all.
///
/// Deliberately conservative: this is what decides whether a keystroke in
/// the results grid may write to the database, so every case it can't read
/// with certainty has to come back `None`. Being wrong the other way means
/// generating an `UPDATE` against the wrong table.
pub fn single_table_source(sql: &str) -> Option<String> {
    let tokens = sql_tokens(strip_leading_comments(sql));
    let first = tokens.first()?;
    if !first.word.eq_ignore_ascii_case("select") {
        return None;
    }
    // A second `select` means a subquery or a set operation: either way the
    // rows on screen no longer map to one table's rows.
    if tokens
        .iter()
        .skip(1)
        .any(|t| t.word.eq_ignore_ascii_case("select"))
    {
        return None;
    }
    for token in &tokens {
        if token.depth == 0
            && matches!(
                token.word.to_ascii_lowercase().as_str(),
                "join" | "union" | "intersect" | "except" | "group" | "having" | "distinct"
            )
        {
            return None;
        }
    }

    let from = tokens
        .iter()
        .position(|t| t.depth == 0 && t.word.eq_ignore_ascii_case("from"))?;
    let table = tokens.get(from + 1)?;
    if !is_identifier(&table.word) {
        return None;
    }

    // Skip an alias (`FROM users u`, `FROM users AS u`) -- it changes
    // nothing about which table the rows came from.
    let mut next = from + 2;
    if tokens
        .get(next)
        .is_some_and(|t| t.word.eq_ignore_ascii_case("as"))
    {
        next += 1;
    }
    if tokens
        .get(next)
        .is_some_and(|t| is_identifier(&t.word) && !is_clause_keyword(&t.word))
    {
        next += 1;
    }
    // A comma here is the old-style join syntax; anything else that isn't a
    // clause this code recognises means the query does something it hasn't
    // been taught to read.
    match tokens.get(next) {
        None => Some(table.word.clone()),
        Some(t) if is_clause_keyword(&t.word) => Some(table.word.clone()),
        Some(_) => None,
    }
}

/// For a query against more than one table (a `JOIN`), which table each
/// result column came from -- `columns[i]` is that column's own name
/// (`QueryResult::Table::columns`, so this never has to touch
/// `QueryResult` itself to answer the question), `sources[i]` is `None`
/// unless the SELECT list named it explicitly as `alias.column` (or
/// `table.column`), resolved through the same FROM/JOIN alias table
/// `resolve_alias` already builds for completion.
///
/// Deliberately conservative, same reasoning as `single_table_source`: a
/// bare unqualified name is ambiguous across the joined tables without
/// the connected schema (which this function doesn't have, same as every
/// other helper in this file -- see `completion_context`'s own doc
/// comment), an expression/computed column has no one table to point at,
/// and a `*`/`alias.*` anywhere in the SELECT list makes the whole call
/// bail (`None` for every column) since it expands to a number of columns
/// this can't predict from the query text alone, which would silently
/// misalign every source after it. Call this only once
/// `single_table_source` has already returned `None` -- a query against
/// exactly one table is answered more precisely (and more cheaply) by
/// that, without needing `columns` at all.
pub fn joined_column_sources(sql: &str, columns: &[String]) -> Vec<Option<String>> {
    let none = || vec![None; columns.len()];
    let tokens = sql_tokens(strip_leading_comments(sql));
    let Some(first) = tokens.first() else {
        return none();
    };
    if !first.word.eq_ignore_ascii_case("select") {
        return none();
    }
    // A second `select` anywhere -- a subquery or a set operation -- means
    // the column list isn't as simple as it looks; same unconditional
    // check `single_table_source` uses for the same reason.
    if tokens
        .iter()
        .skip(1)
        .any(|t| t.word.eq_ignore_ascii_case("select"))
    {
        return none();
    }
    let Some(from) = tokens
        .iter()
        .position(|t| t.depth == 0 && t.word.eq_ignore_ascii_case("from"))
    else {
        return none();
    };
    let select_list = &tokens[1..from];

    let mut items: Vec<&[SqlToken]> = Vec::new();
    let mut start = 0;
    for (i, token) in select_list.iter().enumerate() {
        if token.depth == 0 && token.word == "," {
            items.push(&select_list[start..i]);
            start = i + 1;
        }
    }
    items.push(&select_list[start..]);

    let mut sources = Vec::with_capacity(items.len());
    for item in &items {
        let Some(first) = item.first() else {
            sources.push(None);
            continue;
        };
        // A bare `*` select-list item tokenizes as just "*"; `alias.*` as
        // "alias." (the tokenizer's identifier run stops at the `*`, which
        // isn't an identifier character) followed by a separate "*" token.
        // Checked only at the item's own front, not anywhere inside it, so
        // a `*` that's really a function argument (`count(*)`) isn't
        // mistaken for one -- it expands server-side to a number of
        // columns this can't know from the query text, so finding one
        // bails entirely rather than misaligning everything after it.
        let is_star = first.word == "*"
            || (first.word.ends_with('.') && item.get(1).is_some_and(|t| t.word == "*"));
        if is_star {
            return none();
        }
        sources.push(
            first
                .word
                .rsplit_once('.')
                .and_then(|(alias, _)| resolve_alias(&tokens, alias)),
        );
    }

    if sources.len() != columns.len() {
        // The SELECT list didn't split into as many items as the result
        // actually has columns (a syntax this hasn't been taught to read) --
        // safer to say nothing than to misalign every answer after it.
        return none();
    }
    sources
}
