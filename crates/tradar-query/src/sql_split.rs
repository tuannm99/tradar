use super::*;

/// Splits a SQL buffer on `;`, ignoring separators that aren't really
/// separators: inside `'...'`/`"..."` strings, inside `--` line comments or
/// `/* */` blocks, and inside Postgres dollar-quoted bodies (`$$ ... $$`,
/// `$tag$ ... $tag$`) which routinely contain semicolons.
///
/// A lexer rather than a parser: knowing which characters are quoted is
/// enough to find statement boundaries, and it stays correct for dialects
/// this crate has never heard of. Shared here because every SQL connector
/// needs the same answer -- see `SQL_KEYWORDS`.
pub fn split_sql_statements(sql: &str) -> Vec<Statement> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    // A doubled quote is an escaped quote, not the end.
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
            }
            b'$' => match dollar_tag(sql, i) {
                Some(tag) => {
                    let body = i + tag.len();
                    i = sql[body..]
                        .find(&tag)
                        .map_or(bytes.len(), |n| body + n + tag.len());
                }
                None => i += 1,
            },
            b';' => {
                push_statement(sql, start, i, &mut statements);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    push_statement(sql, start, bytes.len(), &mut statements);
    statements
}

/// The dollar-quote tag starting at `at` (`$$` or `$tag$`), if there is
/// one. `None` for a bare `$` such as a parameter placeholder.
pub(crate) fn dollar_tag(sql: &str, at: usize) -> Option<String> {
    let rest = &sql[at + 1..];
    let end = rest.find('$')?;
    let tag = &rest[..end];
    tag.chars()
        .all(|c| c.is_alphanumeric() || c == '_')
        .then(|| format!("${tag}$"))
}

/// Records `sql[start..end]` as a statement unless it's only whitespace and
/// comments -- a trailing `;` or a commented-out block shouldn't turn into
/// an empty statement that then gets run.
pub(crate) fn push_statement(sql: &str, start: usize, end: usize, out: &mut Vec<Statement>) {
    let slice = &sql[start..end];
    if strip_leading_comments(slice).trim().is_empty() {
        return;
    }
    let leading = slice.len() - slice.trim_start().len();
    let trailing = slice.len() - slice.trim_end().len();
    out.push(Statement {
        text: slice.trim().to_string(),
        start: start + leading,
        end: end - trailing,
    });
}

/// The line of `sql` containing `position` (a 1-based **character**
/// index into the exact text that was sent), plus a `^` marker under it --
/// e.g. `LINE 2:   WHERE bad_col = 1` / `         ^`. `None` when
/// `position` doesn't land inside `sql` at all. Lives here rather than in
/// one connector, same reasoning as `returns_rows`: Postgres's own error
/// protocol reports a position this way (`format_pg_error` in
/// `tradar-connector-postgres`), and SQLite's error message conventionally
/// quotes the offending token, which `tradar-connector-sqlite` resolves to
/// a position of its own before calling this -- both want the identical
/// `LINE N: ... ^` rendering, and connector crates can't depend on each
/// other to share it.
pub fn line_and_caret(sql: &str, position: usize) -> Option<String> {
    let index = position.checked_sub(1)?;
    let chars: Vec<char> = sql.chars().collect();
    if index >= chars.len() {
        return None;
    }
    let mut line_number = 1;
    let mut line_start = 0;
    for (i, &c) in chars.iter().enumerate().take(index) {
        if c == '\n' {
            line_number += 1;
            line_start = i + 1;
        }
    }
    let line_end = chars[line_start..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |n| line_start + n);
    let line: String = chars[line_start..line_end].iter().collect();

    let prefix = format!("LINE {line_number}: ");
    let caret_offset = index - line_start;
    let caret_line = format!(
        "{}{}^",
        " ".repeat(prefix.chars().count()),
        " ".repeat(caret_offset)
    );
    Some(format!("{prefix}{line}\n{caret_line}"))
}

/// Whether `sql` is a statement that returns rows, and so should be run
/// with a fetch rather than an execute. Lives here rather than in one
/// connector because every SQL driver needs the same answer and connector
/// crates can't depend on each other.
///
/// A heuristic on the leading keyword, not a parser: that's what it takes
/// to tell `SELECT` from `INSERT` without pulling in a SQL grammar, and the
/// cost of being wrong is only which of two shapes the result is reported
/// in. `RETURNING` is checked separately because `INSERT ... RETURNING` (and
/// friends) do return rows despite the leading keyword.
pub fn returns_rows(sql: &str) -> bool {
    let normalized = strip_leading_comments(sql).to_ascii_lowercase();
    if normalized
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == "returning")
    {
        return true;
    }
    let first = normalized
        .split(|c: char| c.is_whitespace() || c == '(')
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    matches!(
        first,
        "select"
            | "with"
            | "values"
            | "show"
            | "explain"
            | "pragma"
            | "describe"
            | "desc"
            | "table"
    )
}

/// What a statement's leading keyword says about transaction control.
/// `None` for anything else, which just runs as a normal statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionControl {
    Begin,
    Commit,
    Rollback,
}

/// Whether `sql` is `BEGIN`/`START TRANSACTION`, `COMMIT`/`END`, or
/// `ROLLBACK` -- same leading-keyword heuristic as `returns_rows`, and for
/// the same reason: not a parser, just enough to tell these apart from an
/// ordinary statement.
///
/// A SQL connector's `execute` routes these to its own held transaction
/// instead of sending the literal text to the database, because the
/// pool-per-query model every SQL connector otherwise uses can't honour
/// them on its own -- a `BEGIN` run against one pooled connection says
/// nothing about which connection the next statement happens to land on.
/// `COMMIT`/`END` are the same command in Postgres; SQLite doesn't have
/// `END` but accepts it as a no-op-if-absent match here regardless, same
/// tradeoff `returns_rows` makes for keywords a given backend doesn't
/// actually have.
pub fn transaction_control(sql: &str) -> Option<TransactionControl> {
    let normalized = strip_leading_comments(sql).to_ascii_lowercase();
    // Unlike `returns_rows`'s equivalent split, `;` has to count as a
    // delimiter too -- "BEGIN;" has nothing separating the keyword from the
    // statement terminator, and `split_sql_statements` has usually already
    // trimmed the `;` off by the time a driver sees this, but a bare
    // one-statement buffer run without going through that first has not.
    let first = normalized
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    match first {
        "begin" | "start" => Some(TransactionControl::Begin),
        "commit" | "end" => Some(TransactionControl::Commit),
        "rollback" => Some(TransactionControl::Rollback),
        _ => None,
    }
}

/// Drops leading `--` line comments, `/* */` block comments and whitespace,
/// so a commented-out header doesn't hide the statement's first keyword.
pub(crate) fn strip_leading_comments(sql: &str) -> &str {
    let mut rest = sql.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after
                .find('\n')
                .map_or("", |i| &after[i + 1..])
                .trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after
                .find("*/")
                .map_or("", |i| &after[i + 2..])
                .trim_start();
        } else {
            return rest;
        }
    }
}
