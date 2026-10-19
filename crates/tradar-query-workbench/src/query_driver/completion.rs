use super::*;

/// What the cursor is positioned to complete, read from the SQL typed so
/// far -- deliberately schema-agnostic, the same split of responsibility
/// `single_table_source` uses: this file answers "what does the query
/// text say", `completion.rs` (which holds the connected schema) answers
/// "what does that resolve to".
#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// No special context -- fall back to the flat table/column/keyword
    /// list.
    None,
    /// Cursor is right after `<alias>.`, optionally with a partial column
    /// name already typed after the dot -- `table` is the table that alias
    /// refers to, resolved from this query's own FROM/JOIN clauses (not
    /// yet checked against the connected schema).
    TableColumns { table: String },
    /// Cursor is completing a table name right after JOIN -- `known_tables`
    /// are the tables already named in FROM/earlier JOINs, so a caller
    /// with FK data can rank a related table first.
    JoinTarget { known_tables: Vec<String> },
    /// Cursor is typing the method segment of a Mongo shell call --
    /// `db.orders.f` -- not yet past the opening `(` that would make it a
    /// field position (see `mongo_collection_in_scope`). The only legal
    /// completions here are method names (`find`, `aggregate`, ...),
    /// never a collection or field -- unlike every other context, which
    /// only *re-ranks* `matches`' flat list, this one narrows it to
    /// `CandidateKind::Keyword` alone.
    MongoMethod,
    /// Cursor is right after `<schema>.`, optionally with a partial table
    /// name already typed after the dot -- `schema` is read straight from
    /// the token, not yet resolved against anything (same split of
    /// responsibility `TableColumns` uses). This is what a `SchemaInfo`
    /// whose own `name` is always `<schema>.<table>` needs: Cassandra's
    /// `list_schema` names every entry `<keyspace>.<table>` (no bare-name
    /// case the way Postgres only qualifies on a cross-schema collision --
    /// see `qualify_colliding_names`), so the flat candidate list's
    /// `text.starts_with(prefix)` check never matches a bare table name
    /// typed after the keyspace, even though the keyspace itself was typed
    /// correctly. Produced only when the dotted token's alias part did
    /// *not* resolve as a `FROM`/`JOIN` alias (`TableColumns` already
    /// covers the case where it did) -- a real schema/keyspace name versus
    /// an unknown alias isn't told apart here; the caller with schema
    /// access finds out by trying to match it.
    SchemaQualifiedTable { schema: String },
}

/// A table named in a FROM/JOIN clause, and the alias it was given (if
/// any) -- `alias.column` resolves through this, `single_table_source`'s
/// own alias-skipping logic (`FROM users u`/`FROM users AS u`) reused here
/// because JOIN-aware completion needs it for every clause, not just one.
pub(crate) struct TableRef {
    pub(crate) table: String,
    pub(crate) alias: Option<String>,
}

/// Words that can't be an alias because they start the next clause of a
/// join, not name one -- `is_clause_keyword`'s SELECT-tail list plus the
/// join-specific words that show up between one table and the next.
pub(crate) fn is_alias_stopword(word: &str) -> bool {
    is_clause_keyword(word)
        || matches!(
            word.to_ascii_lowercase().as_str(),
            "join" | "inner" | "left" | "right" | "full" | "outer" | "cross" | "on" | "using"
        )
}

/// Every table named in a FROM/JOIN clause of `tokens`, in order, each with
/// whatever alias immediately follows it (if any).
pub(crate) fn from_join_table_refs(tokens: &[SqlToken]) -> Vec<TableRef> {
    let mut refs = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.depth != 0 || !matches!(token.word.to_ascii_lowercase().as_str(), "from" | "join")
        {
            continue;
        }
        let Some(table_tok) = tokens.get(i + 1) else {
            continue;
        };
        if !is_identifier(&table_tok.word) {
            continue;
        }
        let mut next = i + 2;
        if tokens
            .get(next)
            .is_some_and(|t| t.word.eq_ignore_ascii_case("as"))
        {
            next += 1;
        }
        let alias = tokens
            .get(next)
            .filter(|t| is_identifier(&t.word) && !is_alias_stopword(&t.word))
            .map(|t| t.word.clone());
        refs.push(TableRef {
            table: table_tok.word.clone(),
            alias,
        });
    }
    refs
}

/// The table `alias` refers to, in the query the tokens came from -- either
/// an explicit alias (`FROM users u` -> `u` resolves to `users`) or the
/// table's own name used bare (`FROM users` -> `users` resolves to itself).
pub(crate) fn resolve_alias(tokens: &[SqlToken], alias: &str) -> Option<String> {
    from_join_table_refs(tokens).into_iter().find_map(|r| {
        let matched = match &r.alias {
            Some(a) => a.eq_ignore_ascii_case(alias),
            None => r.table.eq_ignore_ascii_case(alias),
        };
        matched.then_some(r.table)
    })
}

/// Reads `text_before_cursor` for a completion-worthy context -- see
/// `CompletionContext`. Returns `None` for anything this hasn't been taught
/// to recognise, including every non-SQL dialect (Mongo/Redis/ES queries
/// have no FROM/JOIN to find, so this naturally falls through for them).
pub fn completion_context(text_before_cursor: &str) -> CompletionContext {
    let tokens = sql_tokens(text_before_cursor);
    let Some(last) = tokens.last() else {
        return CompletionContext::None;
    };
    if let Some(dot) = last.word.find('.') {
        // Mongo's own `db.<collection>.<method>` shape, still being typed
        // past the second dot -- checked before the SQL alias path below,
        // since `alias` there would just be the literal `"db"`, which
        // never resolves against a FROM/JOIN anyway.
        if mongo_call_collection(&last.word).is_some() {
            return CompletionContext::MongoMethod;
        }
        let alias = &last.word[..dot];
        if alias.is_empty() {
            return CompletionContext::None;
        }
        return match resolve_alias(&tokens, alias) {
            Some(table) => CompletionContext::TableColumns { table },
            // Not a `FROM`/`JOIN` alias -- try it as a schema/keyspace
            // name instead (`SchemaQualifiedTable`'s own doc comment).
            // The caller with schema access is the one that finds out
            // whether it really is one.
            None => CompletionContext::SchemaQualifiedTable {
                schema: alias.to_string(),
            },
        };
    }
    // The index of the triggering `JOIN` keyword itself -- either the last
    // token (nothing typed after it yet) or the second-to-last (a partial
    // table name is). Tables are read only from *before* that index, so
    // the in-progress JOIN's own partial text is never mistaken for a
    // table already in the query.
    let join_at = if last.word.eq_ignore_ascii_case("join") {
        Some(tokens.len() - 1)
    } else if tokens.len() >= 2 && tokens[tokens.len() - 2].word.eq_ignore_ascii_case("join") {
        Some(tokens.len() - 2)
    } else {
        None
    };
    if let Some(join_at) = join_at {
        return CompletionContext::JoinTarget {
            known_tables: from_join_table_refs(&tokens[..join_at])
                .into_iter()
                .map(|r| r.table)
                .collect(),
        };
    }
    match mongo_collection_in_scope(&tokens) {
        Some(table) => CompletionContext::TableColumns { table },
        None => CompletionContext::None,
    }
}

/// The collection named in a Mongo shell call token, e.g.
/// `db.orders.find` -> `orders` -- `None` for anything that isn't shaped
/// like `db.<collection>.<rest>` (including a bare `db.orders` with no
/// second dot, which isn't a call yet).
pub(crate) fn mongo_call_collection(word: &str) -> Option<&str> {
    let rest = word.strip_prefix("db.")?;
    let end = rest.find('.')?;
    let collection = &rest[..end];
    (!collection.is_empty()).then_some(collection)
}

/// The collection whose fields the cursor is completing inside a Mongo
/// shell call's arguments -- e.g. `db.orders.find({status: "open", na`
/// is completing a field of `orders`. Mongo has no FROM/JOIN for
/// `resolve_alias` to scan, so this looks back for the call's own
/// `db.<collection>.<method>` token instead, at a shallower paren depth
/// than the cursor -- that's what marks the cursor as inside that call's
/// `(...)` rather than, say, still typing the method name itself (which
/// is the same depth as the call token, since the `(` hasn't been typed
/// yet). `last.word` containing a `.` also rules that out on its own: a
/// field name never has a dot, only the `db.x.y` chain does.
pub(crate) fn mongo_collection_in_scope(tokens: &[SqlToken]) -> Option<String> {
    let last = tokens.last()?;
    if last.word.contains('.') {
        return None;
    }
    tokens[..tokens.len() - 1]
        .iter()
        .rev()
        .find_map(|t| {
            (t.depth < last.depth)
                .then(|| mongo_call_collection(&t.word))
                .flatten()
        })
        .map(str::to_string)
}

/// Each of `columns`' declared type from `schema`'s entry named `table`,
/// aligned by name -- what the results header's `id int8`-style annotation
/// and the cell preview panel's `(type)` show. `None` per column when
/// there's no known source table at all (a join, a write, `edit_source`
/// came back empty) or that particular one isn't in `schema` (an
/// expression, an alias, a computed column) -- shown as nothing rather
/// than guessed.
pub fn column_types(
    columns: &[String],
    schema: &[SchemaInfo],
    table: Option<&str>,
) -> Vec<Option<String>> {
    let types: std::collections::HashMap<&str, &str> = table
        .and_then(|name| schema.iter().find(|entry| entry.name == name))
        .map(|entry| {
            entry
                .columns
                .iter()
                .map(|c| (c.name.as_str(), c.type_name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    columns
        .iter()
        .map(|name| types.get(name.as_str()).map(|t| t.to_string()))
        .collect()
}

/// Words that start a clause after the `FROM` list, so they can't be an
/// alias for the table that precedes them.
pub(crate) fn is_clause_keyword(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "where" | "order" | "limit" | "offset" | "fetch" | "for"
    )
}

pub(crate) fn is_identifier(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
        && !word.chars().next().is_some_and(|c| c.is_ascii_digit())
}

pub(crate) struct SqlToken {
    pub(crate) word: String,
    /// Parenthesis nesting, so a keyword inside a function call or a
    /// subquery isn't mistaken for a top-level clause.
    pub(crate) depth: usize,
}

/// Splits SQL into words and punctuation, tracking parenthesis depth and
/// skipping over string and comment contents. The same lexer-not-parser
/// tradeoff as `split_sql_statements`: enough structure to answer one
/// narrow question, no grammar to keep up to date.
pub(crate) fn sql_tokens(sql: &str) -> Vec<SqlToken> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                let quote = c;
                let start = i;
                i += 1;
                while i < chars.len() {
                    if chars[i] == quote {
                        if chars.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
                // A double-quoted name is an identifier; a single-quoted
                // one is a value, and neither is a keyword.
                let word: String = chars[start..i.min(chars.len())].iter().collect();
                tokens.push(SqlToken { word, depth });
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            '(' => {
                depth += 1;
                tokens.push(SqlToken {
                    word: "(".to_string(),
                    depth,
                });
                i += 1;
            }
            ')' => {
                tokens.push(SqlToken {
                    word: ")".to_string(),
                    depth,
                });
                depth = depth.saturating_sub(1);
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            c if c.is_alphanumeric() || c == '_' || c == '.' || c == '$' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric()
                        || chars[i] == '_'
                        || chars[i] == '.'
                        || chars[i] == '$')
                {
                    i += 1;
                }
                tokens.push(SqlToken {
                    word: chars[start..i].iter().collect(),
                    depth,
                });
            }
            other => {
                tokens.push(SqlToken {
                    word: other.to_string(),
                    depth,
                });
                i += 1;
            }
        }
    }
    tokens
}

/// The SQL words worth completing, shared by the SQL connectors -- they
/// can't depend on each other, but both depend on this crate. Not a full
/// grammar's worth: the point is to save typing on the words you write
/// constantly, not to enumerate the standard.
pub const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "OUTER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS NULL",
    "IS NOT NULL",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "EXISTS",
    "UNION",
    "UNION ALL",
    "INTERSECT",
    "EXCEPT",
    "WITH",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "RETURNING",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "CREATE INDEX",
    "PRIMARY KEY",
    "FOREIGN KEY",
    "REFERENCES",
    "DEFAULT",
    "NULL",
    "TRUE",
    "FALSE",
    "ASC",
    "DESC",
];
