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

#[async_trait]
pub trait QueryDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;

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
