//! The `QueryDriver` trait every query-shaped database backend implements
//! (Postgres, SQLite, Elasticsearch, Redis, MongoDB today). Code outside a
//! concrete driver module must depend only on this trait, never on a
//! specific driver crate/module directly -- that's what keeps drivers
//! isolated and pluggable.

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    /// The table's columns, for backends where that's a meaningful concept.
    /// Empty for the document/key-value drivers (MongoDB collections have
    /// no fixed schema, Redis keys have no columns at all), and for any
    /// driver that hasn't been taught to report them yet -- the sidebar
    /// treats an empty list as "nothing to expand", not as an error.
    pub columns: Vec<ColumnInfo>,
}

impl SchemaInfo {
    /// A schema entry with no column detail -- what the non-columnar
    /// drivers return.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    /// The backend's own type name (`INTEGER`, `character varying`, ...),
    /// passed through rather than normalized: the point is to show what the
    /// database actually says.
    pub type_name: String,
}

/// How many rows a driver will materialise for one query. A result set is
/// pulled into memory to be rendered, so an unbounded `SELECT *` against a
/// large table would otherwise be a straightforward way to exhaust it --
/// see "Support large result sets efficiently" in CLAUDE.md. Drivers stop
/// reading at this point and report the result as truncated.
pub const MAX_ROWS: usize = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        /// The backend had more rows than `MAX_ROWS`; what's here is a
        /// prefix. Surfaced in the UI so a truncated result can't be
        /// mistaken for the whole answer.
        truncated: bool,
    },
    Documents(Vec<serde_json::Value>),
    /// A statement that changed data instead of returning it -- `INSERT`,
    /// `UPDATE`, `DELETE`, DDL. Without this, those came back as an empty
    /// `Table` and rendered as "0 rows", indistinguishable from a `SELECT`
    /// that matched nothing: no way to tell whether the statement ran.
    Affected {
        rows: u64,
    },
}

/// One runnable statement inside a buffer that may hold several, plus
/// where it sits -- the offsets are what let "run the statement under the
/// cursor" find the right one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

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
fn dollar_tag(sql: &str, at: usize) -> Option<String> {
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
fn push_statement(sql: &str, start: usize, end: usize, out: &mut Vec<Statement>) {
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

/// Drops leading `--` line comments, `/* */` block comments and whitespace,
/// so a commented-out header doesn't hide the statement's first keyword.
fn strip_leading_comments(sql: &str) -> &str {
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

#[async_trait]
pub trait QueryDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;

    /// Splits a buffer into the statements it holds, so a file can carry
    /// several and be run one at a time. The default treats the whole
    /// buffer as a single statement, which is right for a backend with no
    /// separator of its own; each driver overrides it because every query
    /// language delimits differently (SQL on `;`, Redis per line, an
    /// Elasticsearch request as a verb line plus its JSON body).
    fn split_statements(&self, text: &str) -> Vec<Statement> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let start = text.len() - text.trim_start().len();
        vec![Statement {
            text: trimmed.to_string(),
            start,
            end: start + trimmed.len(),
        }]
    }

    /// Words this backend's query language uses, offered as completions.
    /// Empty by default: a driver that doesn't say gets schema names only,
    /// which is still useful. Each driver spells its own vocabulary -- this
    /// crate must not know that Postgres speaks SQL and Redis doesn't.
    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    /// Render `query` as a shell command that reproduces it against this
    /// driver's backend, if the driver supports export at all (currently
    /// only Elasticsearch, via `curl`). `None` means "not supported" rather
    /// than an error -- most drivers just don't implement this.
    fn export_curl(&self, _query: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<String> {
        split_sql_statements(sql)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn a_buffer_splits_on_semicolons() {
        assert_eq!(texts("SELECT 1; SELECT 2"), vec!["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn a_statement_may_span_several_lines() {
        let sql = "SELECT id,\n       name\nFROM users\nWHERE id = 1;\n\nSELECT 2;";

        assert_eq!(
            texts(sql),
            vec![
                "SELECT id,\n       name\nFROM users\nWHERE id = 1",
                "SELECT 2"
            ],
            "line breaks are not statement boundaries"
        );
    }

    #[test]
    fn a_semicolon_inside_a_string_is_not_a_separator() {
        assert_eq!(
            texts("SELECT ';' AS x; SELECT 2"),
            vec!["SELECT ';' AS x", "SELECT 2"]
        );
        assert_eq!(
            texts(r#"SELECT "a;b" FROM t"#),
            vec![r#"SELECT "a;b" FROM t"#]
        );
    }

    #[test]
    fn a_doubled_quote_inside_a_string_does_not_end_it() {
        assert_eq!(
            texts("SELECT 'it''s; fine' AS x; SELECT 2"),
            vec!["SELECT 'it''s; fine' AS x", "SELECT 2"]
        );
    }

    #[test]
    fn a_semicolon_inside_a_comment_is_not_a_separator() {
        assert_eq!(
            texts("SELECT 1 -- one; two\n; SELECT 2"),
            vec!["SELECT 1 -- one; two", "SELECT 2"]
        );
        assert_eq!(
            texts("SELECT /* a; b */ 1; SELECT 2"),
            vec!["SELECT /* a; b */ 1", "SELECT 2"]
        );
    }

    #[test]
    fn a_dollar_quoted_body_may_contain_semicolons() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; SELECT 2";

        assert_eq!(
            texts(sql),
            vec![
                "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql",
                "SELECT 2"
            ]
        );
    }

    #[test]
    fn a_tagged_dollar_quote_is_matched_by_its_tag() {
        let sql = "SELECT $tag$ a; $$ still inside $tag$; SELECT 2";

        assert_eq!(
            texts(sql),
            vec!["SELECT $tag$ a; $$ still inside $tag$", "SELECT 2"]
        );
    }

    #[test]
    fn empty_statements_are_dropped() {
        assert_eq!(texts("SELECT 1;"), vec!["SELECT 1"]);
        assert_eq!(texts("SELECT 1;;;"), vec!["SELECT 1"]);
        assert!(texts("").is_empty());
        assert!(texts("  \n ; ; ").is_empty());
        assert!(
            texts("-- just a comment").is_empty(),
            "nothing to run in a comment-only buffer"
        );
    }

    #[test]
    fn offsets_point_at_the_statement_in_the_original_buffer() {
        let sql = "SELECT 1;\n\nSELECT 2;";

        let statements = split_sql_statements(sql);

        assert_eq!(&sql[statements[0].start..statements[0].end], "SELECT 1");
        assert_eq!(&sql[statements[1].start..statements[1].end], "SELECT 2");
    }

    #[test]
    fn an_unterminated_statement_still_counts() {
        // The last statement usually has no trailing semicolon.
        assert_eq!(texts("SELECT 1;\nSELECT 2"), vec!["SELECT 1", "SELECT 2"]);
        assert_eq!(texts("SELECT 'unclosed"), vec!["SELECT 'unclosed"]);
    }

    #[test]
    fn row_returning_statements_are_recognised() {
        for sql in [
            "SELECT 1",
            "  select * from users",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "VALUES (1)",
            "PRAGMA table_info(users)",
            "EXPLAIN SELECT 1",
            "show tables",
        ] {
            assert!(returns_rows(sql), "{sql:?} should return rows");
        }
    }

    #[test]
    fn data_changing_statements_are_recognised() {
        for sql in [
            "INSERT INTO users VALUES (1)",
            "update users set name = 'x'",
            "DELETE FROM users",
            "CREATE TABLE users (id INT)",
            "DROP TABLE users",
            "  \n alter table users add column x int",
        ] {
            assert!(!returns_rows(sql), "{sql:?} should not return rows");
        }
    }

    #[test]
    fn a_returning_clause_makes_a_write_return_rows() {
        assert!(returns_rows("INSERT INTO users VALUES (1) RETURNING id"));
        assert!(returns_rows("delete from users returning *"));
        assert!(
            !returns_rows("INSERT INTO returning_log VALUES (1)"),
            "a table merely named like the keyword is not a RETURNING clause"
        );
    }

    #[test]
    fn leading_comments_do_not_hide_the_statement() {
        assert!(returns_rows("-- a note\nSELECT 1"));
        assert!(returns_rows("/* a note */ SELECT 1"));
        assert!(!returns_rows("-- a note\nINSERT INTO users VALUES (1)"));
        assert!(!returns_rows("/* one */ /* two */\nDELETE FROM users"));
    }

    #[test]
    fn an_empty_statement_is_not_row_returning() {
        assert!(!returns_rows(""));
        assert!(!returns_rows("   \n  "));
        assert!(!returns_rows("-- only a comment"));
    }
}
