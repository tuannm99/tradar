//! The `QueryDriver` trait every query-shaped database backend implements
//! (Postgres, SQLite, Elasticsearch, Redis, MongoDB today). Code outside a
//! concrete driver module must depend only on this trait, never on a
//! specific driver crate/module directly -- that's what keeps drivers
//! isolated and pluggable.

use async_trait::async_trait;

use tradar_core::action::CrudOp;

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    /// The table's columns, for backends where that's a meaningful concept.
    /// Empty for the document/key-value drivers (MongoDB collections have
    /// no fixed schema, Redis keys have no columns at all), and for any
    /// driver that hasn't been taught to report them yet -- the sidebar
    /// treats an empty list as "nothing to expand", not as an error.
    pub columns: Vec<ColumnInfo>,
    /// The backend's own name for what kind of entry this is (Redis:
    /// `"string"`/`"hash"`/`"list"`/`"set"`/`"zset"`/... from `TYPE`).
    /// `None` for backends where every entry is the same kind (a SQL table,
    /// a Mongo collection) and there's nothing to distinguish -- the browse
    /// sidebar treats `None` as "not applicable".
    pub kind: Option<String>,
}

impl SchemaInfo {
    /// A schema entry with no column detail -- what the non-columnar
    /// drivers return.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            kind: None,
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
    /// Part of this table's primary key. What makes an edit in the results
    /// grid expressible as SQL: without a key there is no `WHERE` that
    /// names exactly the row the user is pointing at, and a statement that
    /// might hit several rows is not one to run on their behalf.
    pub primary_key: bool,
}

impl ColumnInfo {
    /// A column with no key information -- what the drivers that don't
    /// report one return.
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            primary_key: false,
        }
    }
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

/// A change to one row the user already has on screen, described in terms
/// of the grid rather than any dialect's syntax: the screen fills this in
/// from what's selected, and the driver turns it into a statement it can
/// run (`QueryDriver::edit_sql`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowEdit {
    pub table: String,
    /// Column/value pairs that identify the row -- its primary key, taken
    /// from the row as fetched.
    pub key: Vec<(String, String)>,
    pub change: RowChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChange {
    SetValue { column: String, value: String },
    DeleteRow,
}

/// Turns a `RowEdit` into standard SQL, shared by the SQL connectors for
/// the same reason as `SQL_KEYWORDS`: both need the same answer and neither
/// may depend on the other.
///
/// Values are always written as string literals. Postgres and SQLite both
/// coerce `'1'` to a number where a number is wanted, so one shape covers
/// every column type without this code having to know any of them -- the
/// single exception is the literal `NULL`, which is written unquoted
/// because that is exactly how the grid renders a null.
pub fn build_sql_edit(edit: &RowEdit) -> String {
    let table = quote_identifier(&edit.table);
    let where_clause = edit
        .key
        .iter()
        .map(|(column, value)| {
            if value == "NULL" {
                // `= NULL` is never true; a key column can't be null
                // anyway, so this only shows up if the grid is lying about
                // which columns are the key.
                format!("{} IS NULL", quote_identifier(column))
            } else {
                format!("{} = {}", quote_identifier(column), sql_literal(value))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    match &edit.change {
        RowChange::SetValue { column, value } => format!(
            "UPDATE {table} SET {} = {} WHERE {where_clause}",
            quote_identifier(column),
            sql_literal(value)
        ),
        RowChange::DeleteRow => format!("DELETE FROM {table} WHERE {where_clause}"),
    }
}

/// A skeleton Create/Read/Update/Delete statement for `entry`, shared by
/// the SQL connectors -- see `Component::crud_snippet`. Placeholders are
/// `<column_name>` rather than a guessed literal: there's no type mapping
/// reliable enough across every backend's own type names to invent a
/// plausible value, and "fill in this blank" reads better than a fake
/// value the user has to notice is fake and replace anyway.
pub fn build_crud_snippet(entry: &SchemaInfo, op: CrudOp) -> String {
    let table = quote_identifier(&entry.name);
    let columns: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();
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
        CrudOp::Read => format!("SELECT * FROM {table} LIMIT 100;"),
        CrudOp::Create => {
            if columns.is_empty() {
                format!("INSERT INTO {table} (<column>) VALUES (<value>);")
            } else {
                let names = bulleted(
                    &columns
                        .iter()
                        .map(|c| quote_identifier(c))
                        .collect::<Vec<_>>(),
                );
                let placeholders =
                    bulleted(&columns.iter().map(|c| format!("<{c}>")).collect::<Vec<_>>());
                format!("INSERT INTO {table} (\n{names}\n) VALUES (\n{placeholders}\n);")
            }
        }
        CrudOp::Update => {
            let settable: Vec<&str> = columns
                .iter()
                .copied()
                .filter(|c| !primary_key.contains(c))
                .collect();
            let settable = if settable.is_empty() {
                columns.clone()
            } else {
                settable
            };
            let set_clause = if settable.is_empty() {
                "  <column> = <value>".to_string()
            } else {
                bulleted(
                    &settable
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
        CrudOp::Delete => format!("DELETE FROM {table} WHERE {};", where_clause(&primary_key)),
    }
}

/// Double-quotes an identifier, one dotted part at a time so a
/// schema-qualified `public.users` stays two identifiers rather than
/// becoming one strangely named table.
fn quote_identifier(name: &str) -> String {
    name.split('.')
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

fn sql_literal(value: &str) -> String {
    if value == "NULL" {
        return "NULL".to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
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

/// Words that start a clause after the `FROM` list, so they can't be an
/// alias for the table that precedes them.
fn is_clause_keyword(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "where" | "order" | "limit" | "offset" | "fetch" | "for"
    )
}

fn is_identifier(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
        && !word.chars().next().is_some_and(|c| c.is_ascii_digit())
}

struct SqlToken {
    word: String,
    /// Parenthesis nesting, so a keyword inside a function call or a
    /// subquery isn't mistaken for a top-level clause.
    depth: usize,
}

/// Splits SQL into words and punctuation, tracking parenthesis depth and
/// skipping over string and comment contents. The same lexer-not-parser
/// tradeoff as `split_sql_statements`: enough structure to answer one
/// narrow question, no grammar to keep up to date.
fn sql_tokens(sql: &str) -> Vec<SqlToken> {
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

    /// This driver's statement for a row edit made in the results grid.
    /// `None` -- the default -- leaves the grid read-only, which is right
    /// for a backend whose results aren't rows of a table anyone can
    /// address (Redis replies, an Elasticsearch response body). Only the
    /// driver knows its own syntax, so nothing outside one writes SQL; the
    /// SQL connectors all delegate to `build_sql_edit`.
    fn edit_sql(&self, _edit: &RowEdit) -> Option<String> {
        None
    }

    /// The table a result came from, for `edit_sql` to aim at -- `None`
    /// when this driver can't tell, which keeps the grid read-only for that
    /// result. SQL connectors delegate to `single_table_source`.
    fn edit_source(&self, _query: &str) -> Option<String> {
        None
    }

    /// A cheap round trip that only proves the connection still answers --
    /// run periodically in the background (see `QueryEngine`) so a dropped
    /// connection shows up in the UI before the user runs into it via a
    /// failed query. `Ok(())` by default: a driver that doesn't override
    /// this is assumed alive, same as every driver behaved before this
    /// existed. Each real connector overrides it with its own cheapest
    /// "are you there" call, since only the driver knows what that is.
    async fn ping(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether a transaction opened by a `BEGIN` (see `transaction_control`)
    /// is still open -- what `auto-commit: ON/OFF` in the UI shows, derived
    /// rather than tracked as separate mode: "off" just means "there is an
    /// open transaction right now", exactly how `psql`'s own indicator
    /// works. `false` by default; only the SQL connectors, which are the
    /// only ones `transaction_control` means anything for, override it.
    fn in_transaction(&self) -> bool {
        false
    }

    /// Fetches and shapes one `SchemaInfo` entry's full value for a browse
    /// sidebar's specialized per-type view (currently only Redis, keyed off
    /// `SchemaInfo::kind`) -- e.g. a hash's fields/values as a `Table`
    /// rather than a raw command reply. `Err` by default: only a driver
    /// with a browse UI overrides this, and the screen shows the error the
    /// same way a failed query shows one.
    async fn browse_entry(&self, _entry: &SchemaInfo) -> anyhow::Result<QueryResult> {
        anyhow::bail!("this connector has no browse view")
    }

    /// A skeleton statement for `op` against `entry`, in this driver's own
    /// query language -- see `Component::crud_snippet`. `None` by default:
    /// most drivers don't override this until they're taught to.
    fn crud_snippet(&self, _entry: &SchemaInfo, _op: CrudOp) -> Option<String> {
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

    fn users_table() -> SchemaInfo {
        SchemaInfo {
            name: "users".to_string(),
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    primary_key: true,
                },
                ColumnInfo::new("email", "TEXT"),
            ],
            kind: None,
        }
    }

    #[test]
    fn crud_snippet_read_is_a_bounded_select() {
        assert_eq!(
            build_crud_snippet(&users_table(), CrudOp::Read),
            "SELECT * FROM \"users\" LIMIT 100;"
        );
    }

    #[test]
    fn crud_snippet_create_lists_every_column_one_per_line_with_a_named_placeholder() {
        assert_eq!(
            build_crud_snippet(&users_table(), CrudOp::Create),
            "INSERT INTO \"users\" (\n  \"id\",\n  \"email\"\n) VALUES (\n  <id>,\n  <email>\n);"
        );
    }

    #[test]
    fn crud_snippet_update_sets_non_key_columns_one_per_line_and_filters_by_the_key() {
        assert_eq!(
            build_crud_snippet(&users_table(), CrudOp::Update),
            "UPDATE \"users\" SET\n  \"email\" = <email>\nWHERE \"id\" = <id>;"
        );
    }

    #[test]
    fn crud_snippet_delete_filters_by_the_primary_key() {
        assert_eq!(
            build_crud_snippet(&users_table(), CrudOp::Delete),
            "DELETE FROM \"users\" WHERE \"id\" = <id>;"
        );
    }

    #[test]
    fn crud_snippet_falls_back_gracefully_with_no_known_columns() {
        let entry = SchemaInfo::new("mystery");

        assert_eq!(
            build_crud_snippet(&entry, CrudOp::Create),
            "INSERT INTO \"mystery\" (<column>) VALUES (<value>);"
        );
        assert_eq!(
            build_crud_snippet(&entry, CrudOp::Update),
            "UPDATE \"mystery\" SET\n  <column> = <value>\nWHERE <condition>;"
        );
        assert_eq!(
            build_crud_snippet(&entry, CrudOp::Delete),
            "DELETE FROM \"mystery\" WHERE <condition>;"
        );
    }

    #[test]
    fn crud_snippet_create_and_update_wrap_one_column_per_line_so_no_line_runs_long() {
        let entry = SchemaInfo {
            name: "wide".to_string(),
            columns: (0..12)
                .map(|i| ColumnInfo::new(format!("column_number_{i}"), "TEXT"))
                .collect(),
            kind: None,
        };

        for op in [CrudOp::Create, CrudOp::Update] {
            let snippet = build_crud_snippet(&entry, op);
            for line in snippet.lines() {
                assert!(
                    line.chars().count() < 80,
                    "line too long for {op:?}: {line:?}"
                );
            }
        }
    }

    #[test]
    fn crud_snippet_update_with_no_primary_key_sets_every_column() {
        let entry = SchemaInfo {
            name: "logs".to_string(),
            columns: vec![ColumnInfo::new("message", "TEXT")],
            kind: None,
        };

        assert_eq!(
            build_crud_snippet(&entry, CrudOp::Update),
            "UPDATE \"logs\" SET\n  \"message\" = <message>\nWHERE <condition>;"
        );
    }

    #[test]
    fn transaction_control_recognizes_begin_commit_and_rollback() {
        assert_eq!(
            transaction_control("BEGIN;"),
            Some(TransactionControl::Begin)
        );
        assert_eq!(
            transaction_control("begin transaction"),
            Some(TransactionControl::Begin)
        );
        assert_eq!(
            transaction_control("START TRANSACTION"),
            Some(TransactionControl::Begin)
        );
        assert_eq!(
            transaction_control("COMMIT;"),
            Some(TransactionControl::Commit)
        );
        assert_eq!(transaction_control("end"), Some(TransactionControl::Commit));
        assert_eq!(
            transaction_control("ROLLBACK"),
            Some(TransactionControl::Rollback)
        );
    }

    #[test]
    fn transaction_control_ignores_an_ordinary_statement() {
        assert_eq!(transaction_control("SELECT 1"), None);
        assert_eq!(
            transaction_control("INSERT INTO commit_log VALUES (1)"),
            None,
            "a table named after a keyword must not trip this up"
        );
    }

    #[test]
    fn transaction_control_sees_past_a_leading_comment() {
        assert_eq!(
            transaction_control("-- start a transaction\nBEGIN;"),
            Some(TransactionControl::Begin)
        );
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

    fn key(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(c, v)| (c.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_update_names_the_column_the_table_and_the_key() {
        let sql = build_sql_edit(&RowEdit {
            table: "users".to_string(),
            key: key(&[("id", "7")]),
            change: RowChange::SetValue {
                column: "name".to_string(),
                value: "Ada".to_string(),
            },
        });

        assert_eq!(
            sql,
            "UPDATE \"users\" SET \"name\" = 'Ada' WHERE \"id\" = '7'"
        );
    }

    #[test]
    fn a_delete_uses_every_column_of_a_composite_key() {
        let sql = build_sql_edit(&RowEdit {
            table: "memberships".to_string(),
            key: key(&[("user_id", "1"), ("group_id", "2")]),
            change: RowChange::DeleteRow,
        });

        assert_eq!(
            sql,
            "DELETE FROM \"memberships\" WHERE \"user_id\" = '1' AND \"group_id\" = '2'"
        );
    }

    #[test]
    fn a_quote_in_a_value_is_escaped_rather_than_ending_the_literal() {
        let sql = build_sql_edit(&RowEdit {
            table: "users".to_string(),
            key: key(&[("id", "1")]),
            change: RowChange::SetValue {
                column: "name".to_string(),
                value: "O'Brien".to_string(),
            },
        });

        assert_eq!(
            sql, "UPDATE \"users\" SET \"name\" = 'O''Brien' WHERE \"id\" = '1'",
            "an unescaped quote here would be an injection, not a typo"
        );
    }

    #[test]
    fn a_null_is_written_unquoted_the_way_the_grid_shows_it() {
        let sql = build_sql_edit(&RowEdit {
            table: "users".to_string(),
            key: key(&[("id", "1")]),
            change: RowChange::SetValue {
                column: "nickname".to_string(),
                value: "NULL".to_string(),
            },
        });

        assert_eq!(
            sql,
            "UPDATE \"users\" SET \"nickname\" = NULL WHERE \"id\" = '1'"
        );
    }

    #[test]
    fn a_schema_qualified_table_stays_two_identifiers() {
        let sql = build_sql_edit(&RowEdit {
            table: "public.users".to_string(),
            key: key(&[("id", "1")]),
            change: RowChange::DeleteRow,
        });

        assert_eq!(sql, "DELETE FROM \"public\".\"users\" WHERE \"id\" = '1'");
    }

    #[test]
    fn a_plain_select_reports_the_table_it_reads() {
        assert_eq!(
            single_table_source("SELECT * FROM users"),
            Some("users".to_string())
        );
        assert_eq!(
            single_table_source("select id, name from public.users where id = 1 order by id"),
            Some("public.users".to_string())
        );
        assert_eq!(
            single_table_source("SELECT * FROM users u WHERE u.id = 1"),
            Some("users".to_string()),
            "an alias doesn't change which table the rows came from"
        );
        assert_eq!(
            single_table_source("-- yesterday's numbers\nSELECT * FROM users"),
            Some("users".to_string())
        );
    }

    #[test]
    fn a_result_that_is_not_one_table_has_no_editable_source() {
        for sql in [
            "SELECT * FROM users JOIN orders ON orders.user_id = users.id",
            "SELECT * FROM users, orders",
            "SELECT * FROM users UNION SELECT * FROM admins",
            "SELECT count(*) FROM users GROUP BY country",
            "SELECT DISTINCT country FROM users",
            "SELECT * FROM (SELECT * FROM users) t",
            "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders)",
            "INSERT INTO users VALUES (1)",
            "UPDATE users SET name = 'x'",
            "SELECT 1",
        ] {
            assert_eq!(
                single_table_source(sql),
                None,
                "must not be editable: {sql}"
            );
        }
    }

    #[test]
    fn a_keyword_inside_parentheses_does_not_disqualify_the_source() {
        assert_eq!(
            single_table_source("SELECT coalesce(name, 'x') FROM users WHERE id = 1"),
            Some("users".to_string())
        );
    }
}
