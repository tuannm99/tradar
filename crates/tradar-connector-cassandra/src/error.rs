use tradar_query_workbench::query_driver::{self as query_driver};

/// Formats a CQL error the same `LINE N: ... ^` shape
/// `tradar-connector-postgres`'s `format_pg_error` produces, when there's a
/// token in the message to point at. CQL has no equivalent of Postgres's
/// wire-protocol position, but Cassandra's own parser (ANTLR-based)
/// quotes the offending token in its message the same way MySQL/SQLite's
/// own messages do -- `"... mismatched input 'FRO' expecting K_FROM"` or
/// `"... no viable alternative at input 'FRO'"` -- recovered here via
/// `cassandra_error_marker`, the same trick
/// `tradar-connector-sqlite`/`tradar-connector-mysql` already play for
/// exactly this "no native position" situation. Anything that doesn't
/// match that shape (a missing keyspace/table, a timeout, a dropped
/// connection) falls straight through to `ExecutionError`'s own `Display`
/// unchanged -- there's no token to search for.
pub(crate) fn format_cassandra_error<E>(error: E, query: &str) -> anyhow::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    let message = error.to_string();
    let Some(marker) = cassandra_error_marker(&message, query) else {
        return error.into();
    };
    anyhow::anyhow!("{message}\n{marker}")
}

/// The `LINE N: ... ^` marker for the first single-quoted token in
/// `message`, found by that token's first occurrence in `query` --
/// approximate (the real mistake could in principle be a later occurrence
/// of the same token), but still the closest thing to a position
/// Cassandra's own message offers. The explicit `line N:col` its ANTLR
/// parser also reports is deliberately ignored in favor of recomputing a
/// position from the token itself, to stay consistent with every other
/// caller of `line_and_caret`. `None` when the message has no quoted
/// token, or the token it names doesn't actually occur in `query`
/// (nothing to point at either way).
pub(crate) fn cassandra_error_marker(message: &str, query: &str) -> Option<String> {
    let start = message.find('\'')? + 1;
    let after = &message[start..];
    let end = after.find('\'')?;
    let token = &after[..end];
    if token.is_empty() {
        return None;
    }
    let byte_index = query.find(token)?;
    // `line_and_caret` wants a 1-based *character* index, matching
    // Postgres's own convention -- counting chars up to the byte offset
    // `find` returns, so multi-byte UTF-8 ahead of the token doesn't throw
    // the caret off.
    let char_index = query[..byte_index].chars().count() + 1;
    query_driver::line_and_caret(query, char_index)
}
