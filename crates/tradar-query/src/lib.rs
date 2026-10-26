//! The `QueryDriver` trait every query-shaped database backend implements
//! (Postgres, MySQL, SQLite, MongoDB, Elasticsearch, Redis, Cassandra,
//! ClickHouse), with the result/schema types, statement splitting,
//! completion and edit helpers they share. Code outside a concrete driver
//! must depend only on this crate, never on a specific driver -- that's what
//! keeps drivers isolated and pluggable.
//!
//! No UI and no other workspace crate: `tradar-server` and the (deprecated)
//! TUI both build on it, and a connector built for the server only needs
//! this crate.

use async_trait::async_trait;

mod candidates;
mod completion;
mod connect;
mod edit;
mod fetch;
mod snippet;
mod source;
mod sql_split;
mod types;

pub use candidates::*;
pub use completion::*;
pub use connect::*;
pub use edit::*;
pub use fetch::*;
pub use snippet::*;
pub use source::*;
pub use sql_split::*;
pub use types::*;

/// Which CRUD statement to generate a snippet for -- see
/// `QueryDriver::snippet`. (`tradar_core::action` re-exports it for the TUI.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrudOp {
    Create,
    Read,
    Update,
    Delete,
}

impl CrudOp {
    /// `create` / `read` / `update` / `delete` -- the ids `SnippetOption`
    /// uses for the four basic statements.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "create" => Some(Self::Create),
            "read" => Some(Self::Read),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// One entry of the "generate a statement" menu for a table / collection --
/// see `QueryDriver::snippet_options`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnippetOption {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

/// The four statements every driver with a `crud_snippet` offers.
pub const CRUD_OPTIONS: [SnippetOption; 4] = [
    SnippetOption {
        id: "create",
        label: "Create",
        hint: "insert a new row / document",
    },
    SnippetOption {
        id: "read",
        label: "Read",
        hint: "select / find",
    },
    SnippetOption {
        id: "update",
        label: "Update",
        hint: "change existing rows / documents",
    },
    SnippetOption {
        id: "delete",
        label: "Delete",
        hint: "remove rows / documents",
    },
];

/// One ready-made query of a driver's cheat sheet -- see
/// `QueryDriver::cheatsheet`. `text` is in the driver's own language and
/// may hold `${name}` placeholders (a table, a field, a pattern...) that the
/// editor asks for when the query is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheatEntry {
    pub category: &'static str,
    pub title: &'static str,
    pub text: &'static str,
}

impl CheatEntry {
    pub const fn new(category: &'static str, title: &'static str, text: &'static str) -> Self {
        Self {
            category,
            title,
            text,
        }
    }
}

/// What is wrong with a cheat sheet, one line per problem: an empty field,
/// a repeated title, a `${` without its `}`. For each driver's own test.
pub fn cheatsheet_problems(entries: &[CheatEntry]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if entries.is_empty() {
        problems.push("the cheat sheet is empty".to_string());
    }
    for entry in entries {
        let who = format!("`{}`", entry.title);
        if entry.category.trim().is_empty() || entry.title.trim().is_empty() {
            problems.push(format!("{who}: empty category or title"));
        }
        if entry.text.trim().is_empty() {
            problems.push(format!("{who}: empty text"));
        }
        if entry.text != entry.text.trim() {
            problems.push(format!("{who}: text has leading or trailing whitespace"));
        }
        if !seen.insert(entry.title) {
            problems.push(format!("{who}: title is used twice"));
        }
        let mut rest = entry.text;
        while let Some(at) = rest.find("${") {
            let after = &rest[at + 2..];
            match after.find('}') {
                Some(end) if !after[..end].contains("${") => rest = &after[end + 1..],
                _ => {
                    problems.push(format!("{who}: a `${{` has no closing `}}`"));
                    break;
                }
            }
        }
    }
    problems
}

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
    /// for a backend whose results aren't rows of anything addressable
    /// (Redis replies, a Kafka/RabbitMQ message). Only the driver knows its
    /// own syntax, so nothing outside one writes SQL; the SQL connectors
    /// all delegate to `build_sql_edit`.
    fn edit_sql(&self, _edit: &RowEdit) -> Option<String> {
        None
    }

    /// This driver's statement for inserting a new row into `table` with
    /// `values` (column, value) -- `None`, the default, means the connector
    /// cannot add rows. The SQL connectors delegate to `build_sql_insert`.
    fn insert_sql(&self, _table: &str, _values: &[(String, String)]) -> Option<String> {
        None
    }

    /// This driver's statement for a table-designer change (see
    /// `TableDesignerOp`). `None` -- the default -- means the table
    /// designer form can be filled in and confirmed, but running it reports
    /// "not supported on this connection" instead of a statement -- same
    /// shape as `edit_sql`'s "read-only" default, and for the same reason:
    /// only the driver knows whether/how its dialect can express this.
    /// Postgres is the only override so far (`docs/backlog/table-designer.md`);
    /// SQLite/Cassandra could add one later without anything here changing.
    fn table_ddl(&self, _op: &TableDesignerOp) -> Option<String> {
        None
    }

    /// Whether the migrations panel (`F1`) can run against this connection
    /// -- `false` by default, since the panel's own BEGIN/COMMIT/tracking-
    /// table SQL is plain ANSI SQL with nothing dialect-specific to build
    /// (unlike `table_ddl`), there's nothing for a driver to *implement*
    /// here; this is purely an opt-in flag. Postgres is the only override
    /// so far (`docs/backlog/migrations.md`) -- the same "which drivers are
    /// taught yet" gate `table_ddl`'s doc comment describes, just boolean
    /// rather than `Option<String>` since there's no statement to build.
    fn supports_migrations(&self) -> bool {
        false
    }

    /// The table a result came from, for `edit_sql` to aim at -- `None`
    /// when this driver can't tell, which keeps the grid read-only for that
    /// result. SQL connectors delegate to `single_table_source`.
    fn edit_source(&self, _query: &str) -> Option<String> {
        None
    }

    /// The column name(s) that identify one row of `edit_source`'s result,
    /// for `RowEdit::key`. `None` -- the default -- falls back to the
    /// connection's own schema (`ColumnInfo::primary_key`), right for the
    /// SQL connectors and Mongo, whose schema already declares this
    /// (Mongo's `list_schema` marks `_id` itself). Elasticsearch overrides
    /// this instead of declaring `_id` in its schema: `_id` is metadata
    /// every document has structurally, never a field an index's own
    /// mapping would ever include, so there'd be nothing in `list_schema`'s
    /// output to mark `primary_key` on in the first place.
    fn edit_key_columns(&self, _source: &str) -> Option<Vec<String>> {
        None
    }

    /// Parallel to a `Table` result's own `columns` -- which table each one
    /// came from, for `gd`-style foreign-key follow on a `JOIN` result,
    /// where `edit_source` (needing exactly one table) has already answered
    /// `None`. `None` -- the default -- means this driver hasn't been
    /// taught the question at all, as opposed to `Some(vec![None; ...])`
    /// (asked, nothing resolved -- an unqualified column, an expression, a
    /// `*`). SQL connectors with FK data in their own schema (Postgres,
    /// MySQL, SQLite) delegate to `query_driver::joined_column_sources`;
    /// ClickHouse/Cassandra have no FK concept to follow to, so there is
    /// nothing this would let `gd` do for them even if they answered it.
    fn column_sources(&self, _query: &str, _columns: &[String]) -> Option<Vec<Option<String>>> {
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

    /// Best-effort real cancel of whatever this driver has running right
    /// now, DB-side -- not just abandoning the caller's own wait on it,
    /// which every caller (TUI's `QueryEngine::cancel`, the server's
    /// `cancel` RPC) already does by aborting/dropping the future around
    /// `execute`. `Ok(())`, including this default no-op, never means
    /// "confirmed stopped": the statement may finish naturally before the
    /// cancel reaches it, or this driver may not have been taught a real
    /// mid-flight cancel at all -- every driver but Postgres/SQLite keeps
    /// this default, same behavior as before this existed. Scoped to
    /// "whatever is running on this driver instance right now" rather
    /// than one specific `execute` call -- correct for how every actual
    /// caller in this codebase uses it today (one `execute` in flight per
    /// connection at a time); it is not guaranteed to target the right
    /// one of several truly concurrent calls on the same connection, a
    /// case nothing here currently produces.
    async fn cancel_query(&self) -> anyhow::Result<()> {
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

    /// The full line to echo for what `browse_entry` will run against
    /// `entry` -- e.g. `127.0.0.1:6379> HGETALL user:1` -- so browsing
    /// doesn't feel like a black box next to the console mode that shows
    /// exactly what you type. Already formatted (target and command
    /// together): the caller only ever displays it verbatim, never parses
    /// it, so a driver-specific detail like Redis's `redis://` scheme
    /// never has to leak into shared code. `None` by default, same as
    /// `browse_entry` itself.
    fn browse_command(&self, _entry: &SchemaInfo) -> Option<String> {
        None
    }

    /// A skeleton statement for `op` against `entry`, in this driver's own
    /// query language, restricted to `columns` -- see
    /// `Component::crud_snippet`. `None` by default: most drivers don't
    /// override this until they're taught to.
    fn crud_snippet(
        &self,
        _entry: &SchemaInfo,
        _op: CrudOp,
        _columns: &[String],
    ) -> Option<String> {
        None
    }

    /// Ready-made queries for this driver -- the "cheat sheet" a user would
    /// otherwise search for: what is in the database, what is running, how
    /// big things are, the usual shapes of a query. Static text in the
    /// driver's own language (see `CheatEntry`); empty by default.
    fn cheatsheet(&self) -> &'static [CheatEntry] {
        &[]
    }

    /// The statements the "generate" menu offers for a table / collection.
    /// Create / Read / Update / Delete by default; a driver with more
    /// basic shapes worth a shortcut (Mongo: count, find by `_id`,
    /// aggregate...) lists them here and builds them in `option_snippet`.
    fn snippet_options(&self) -> Vec<SnippetOption> {
        CRUD_OPTIONS.to_vec()
    }

    /// The statement for one of `snippet_options`' ids. By default the four
    /// basic ids go to `crud_snippet`.
    fn option_snippet(&self, entry: &SchemaInfo, id: &str, columns: &[String]) -> Option<String> {
        self.crud_snippet(entry, CrudOp::from_id(id)?, columns)
    }
}

#[cfg(test)]
mod tests;
