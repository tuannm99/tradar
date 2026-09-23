//! The migrations panel behind `F1` -- Flyway/Alembic-style: numbered
//! `.sql` files in a directory (`tradar_core::storage::default_migrations_dir`,
//! one per connection), tracked via a table in the target database
//! (`_tradar_migrations`) rather than a local file, so switching machines
//! or users against the same database still sees the right set as applied.
//! v1 scope (see `docs/backlog/migrations.md`): Postgres only
//! (`QueryDriver::supports_migrations`), independent of the table designer
//! -- neither feature generates input for the other. Every pending file
//! runs in order, wrapped in its own transaction; a failure stops the run
//! and rolls that one file back, leaving earlier files committed.
//!
//! This module is the driver-agnostic half: discovering/parsing files,
//! building the plain-SQL statements a run needs, and the ratatui overlay
//! (`MigrationsComponent`) that walks Listing -> Running -> Done/Blocked.
//! It never talks to a `QueryDriver` itself -- `QueryScreenComponent` (the
//! only thing holding the `QueryEngine`) drives the actual submissions and
//! feeds outcomes back in via `advance`/`fail`, the same split `row_edit`/
//! `table_designer` use.

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use tradar_core::theme::theme;
use tradar_core::ui;

/// The tracking table every run ensures exists first (`CREATE TABLE IF NOT
/// EXISTS`, so this is safe to send even when it's already there). A
/// leading underscore keeps it out of the way of whatever the migrations
/// themselves create.
pub const TRACKING_TABLE: &str = "_tradar_migrations";

/// One discovered migration file. Equal, comparable by `version` --
/// filenames are expected to zero-pad consistently (`001_`, `002_`, ...,
/// `010_`) so lexicographic order matches numeric order; a project that
/// doesn't pad runs into the exact same footgun Flyway itself has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    pub version: String,
    /// Humanized from the filename (underscores become spaces) -- what the
    /// panel shows, and what's recorded in `TRACKING_TABLE.name`.
    pub name: String,
    pub path: PathBuf,
}

/// `"003_add_email_index.sql"` -> `("003", "add email index")`. `None` for
/// anything that doesn't match `<digits>_<rest>.sql` -- a file that fails
/// to parse is silently left out of the list rather than erroring the
/// whole panel, so a stray README or editor swap file in the directory
/// doesn't block real migrations from running.
fn parse_migration_filename(filename: &str) -> Option<(String, String)> {
    let stem = filename.strip_suffix(".sql")?;
    let (version, rest) = stem.split_once('_')?;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if rest.is_empty() {
        return None;
    }
    Some((version.to_string(), rest.replace('_', " ")))
}

/// Every recognized migration file in `dir`, sorted by version. An empty
/// list for a directory that doesn't exist yet -- nothing's been set up
/// there, not an error (see `parse_migration_filename` for why an
/// unrecognized file is likewise not an error).
pub fn discover_migrations(dir: &Path) -> std::io::Result<Vec<MigrationFile>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<MigrationFile> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let filename = entry.file_name().into_string().ok()?;
            let (version, name) = parse_migration_filename(&filename)?;
            Some(MigrationFile {
                version,
                name,
                path: entry.path(),
            })
        })
        .collect();
    files.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(files)
}

/// `all`, minus whatever `applied` (versions already in `TRACKING_TABLE`)
/// already covers.
pub fn pending_of<'a>(all: &'a [MigrationFile], applied: &[String]) -> Vec<&'a MigrationFile> {
    all.iter()
        .filter(|f| !applied.iter().any(|v| v == &f.version))
        .collect()
}

/// The two statements that ensure `TRACKING_TABLE` exists and read back
/// which versions it already lists -- sent together via `submit_all` so
/// the read never fails with "relation does not exist" on a connection
/// that's never run a migration before.
pub fn check_applied_sql() -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE IF NOT EXISTS {TRACKING_TABLE} (version TEXT PRIMARY KEY, name TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())"
        ),
        format!("SELECT version FROM {TRACKING_TABLE}"),
    ]
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `statements` (`file`'s own SQL, already split by the driver) wrapped in
/// an explicit transaction plus the bookkeeping insert -- `BEGIN` first,
/// `COMMIT` last, so either everything in the file lands or (via a
/// follow-up `ROLLBACK` the host sends on failure) none of it does. A
/// wholesale `BEGIN`/`COMMIT` around an arbitrary buffer is exactly how the
/// app's own manual transaction control already works (see
/// `docs/backlog/migrations.md`), not new behavior invented for this.
pub fn wrap_migration(file: &MigrationFile, statements: &[String]) -> Vec<String> {
    let mut out = vec!["BEGIN".to_string()];
    out.extend(statements.iter().cloned());
    out.push(format!(
        "INSERT INTO {TRACKING_TABLE} (version, name) VALUES ({}, {})",
        sql_string_literal(&file.version),
        sql_string_literal(&file.name)
    ));
    out.push("COMMIT".to_string());
    out
}

/// What the panel wants the host to do, once a key resolves to something
/// actionable.
pub enum MigrationsOutcome {
    Cancelled,
    /// Run every pending file, starting with the first.
    RunAll,
}

enum Stage {
    /// `pending` empty means "up to date" -- shown the same way, just with
    /// nothing to run.
    Listing {
        pending: Vec<MigrationFile>,
        applied_count: usize,
    },
    Running {
        pending: Vec<MigrationFile>,
        index: usize,
    },
    Done {
        count: usize,
    },
    /// Nothing can run, and why -- no migrations directory, a filesystem
    /// error, a failed check-applied query, or a driver that doesn't
    /// support this at all. Dismiss only, same as `row_edit`/
    /// `table_designer`'s own `Blocked`.
    Blocked(String),
}

pub struct MigrationsComponent {
    stage: Stage,
}

impl MigrationsComponent {
    pub fn listing(pending: Vec<MigrationFile>, applied_count: usize) -> Self {
        Self {
            stage: Stage::Listing {
                pending,
                applied_count,
            },
        }
    }

    pub fn blocked(reason: String) -> Self {
        Self {
            stage: Stage::Blocked(reason),
        }
    }

    /// Moves from `Listing` to `Running` at the first pending file, and
    /// hands it back for the host to submit. `None` if there's nothing
    /// pending (or this isn't `Listing`) -- the host has nothing to do.
    pub fn start_running(&mut self) -> Option<MigrationFile> {
        let Stage::Listing { pending, .. } = &self.stage else {
            return None;
        };
        let pending = pending.clone();
        let first = pending.first()?.clone();
        self.stage = Stage::Running { pending, index: 0 };
        Some(first)
    }

    /// The file just submitted succeeded: advances to the next one, or to
    /// `Done` if that was the last. Returns the next file for the host to
    /// submit, `None` once there isn't one (whether because the run just
    /// finished or because this wasn't `Running` at all).
    pub fn advance(&mut self) -> Option<MigrationFile> {
        let Stage::Running { pending, index } = &mut self.stage else {
            return None;
        };
        *index += 1;
        match pending.get(*index) {
            Some(next) => Some(next.clone()),
            None => {
                self.stage = Stage::Done {
                    count: pending.len(),
                };
                None
            }
        }
    }

    /// The file just submitted failed: stop the run and say why. The host
    /// still has to send its own follow-up `ROLLBACK` -- this only updates
    /// what's on screen.
    pub fn fail(&mut self, reason: String) {
        self.stage = Stage::Blocked(reason);
    }

    /// While a run is in flight (`Running`), every key is ignored -- there
    /// is no safe "cancel" for a file whose submission is already in
    /// flight (see this module's doc comment on why the host, not this
    /// component, owns not routing an in-flight outcome anywhere once the
    /// panel's been dismissed). `Listing`/`Done`/`Blocked` all close on any
    /// key; `Listing` with something pending also answers `y`/`Y`.
    pub fn handle_key_event(&mut self, code: KeyCode) -> Option<MigrationsOutcome> {
        match &self.stage {
            Stage::Listing { pending, .. } => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') if !pending.is_empty() => {
                    Some(MigrationsOutcome::RunAll)
                }
                _ => Some(MigrationsOutcome::Cancelled),
            },
            Stage::Running { .. } => None,
            Stage::Done { .. } | Stage::Blocked(_) => Some(MigrationsOutcome::Cancelled),
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let block = ui::panel("Migrations", true);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = match &self.stage {
            Stage::Listing {
                pending,
                applied_count,
            } => {
                let mut lines = vec![Line::from(Span::styled(
                    format!("{applied_count} applied"),
                    Style::default().fg(theme.text_dim),
                ))];
                if pending.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Up to date -- nothing pending.",
                        Style::default().fg(theme.text),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("{} pending:", pending.len()),
                        Style::default().fg(theme.text_dim),
                    )));
                    for file in pending {
                        lines.push(Line::from(Span::styled(
                            format!("  {} {}", file.version, file.name),
                            Style::default().fg(theme.text),
                        )));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "y to run every pending file, any other key to close",
                        Style::default().fg(theme.warning),
                    )));
                }
                lines
            }
            Stage::Running { pending, index } => {
                let mut lines = vec![Line::from(Span::styled(
                    format!("Running {}/{}...", index + 1, pending.len()),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))];
                for (i, file) in pending.iter().enumerate() {
                    let marker = match i.cmp(index) {
                        std::cmp::Ordering::Less => "done",
                        std::cmp::Ordering::Equal => "running",
                        std::cmp::Ordering::Greater => "pending",
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  [{marker}] {} {}", file.version, file.name),
                        Style::default().fg(theme.text),
                    )));
                }
                lines
            }
            Stage::Done { count } => vec![
                Line::from(Span::styled(
                    format!("Applied {count} migration(s)."),
                    Style::default().fg(theme.text),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "any key to close",
                    Style::default().fg(theme.text_dim),
                )),
            ],
            Stage::Blocked(reason) => vec![
                Line::from(Span::styled(
                    reason.clone(),
                    Style::default().fg(theme.error),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "any key to close",
                    Style::default().fg(theme.text_dim),
                )),
            ],
        };

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(version: &str, name: &str) -> MigrationFile {
        MigrationFile {
            version: version.to_string(),
            name: name.to_string(),
            path: PathBuf::from(format!("{version}_{}.sql", name.replace(' ', "_"))),
        }
    }

    #[test]
    fn parses_version_and_humanizes_the_name() {
        assert_eq!(
            parse_migration_filename("003_add_email_index.sql"),
            Some(("003".to_string(), "add email index".to_string()))
        );
    }

    #[test]
    fn a_filename_with_no_leading_digits_does_not_parse() {
        assert_eq!(parse_migration_filename("readme.sql"), None);
        assert_eq!(parse_migration_filename("add_users.sql"), None);
    }

    #[test]
    fn a_non_sql_file_does_not_parse() {
        assert_eq!(parse_migration_filename("001_create_users.sql.bak"), None);
    }

    #[test]
    fn discover_migrations_on_a_missing_directory_is_an_empty_list_not_an_error() {
        let dir = std::path::Path::new("/nonexistent/tradar-migrations-test");

        assert_eq!(discover_migrations(dir).unwrap(), Vec::new());
    }

    #[test]
    fn discover_migrations_sorts_by_version_and_skips_unrecognized_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("002_add_email.sql"), "-- two").unwrap();
        std::fs::write(tmp.path().join("001_create_users.sql"), "-- one").unwrap();
        std::fs::write(tmp.path().join("README.md"), "not a migration").unwrap();

        let files = discover_migrations(tmp.path()).unwrap();

        assert_eq!(
            files.iter().map(|f| f.version.as_str()).collect::<Vec<_>>(),
            vec!["001", "002"]
        );
        assert_eq!(files[0].name, "create users");
    }

    #[test]
    fn pending_of_excludes_already_applied_versions() {
        let all = vec![file("001", "a"), file("002", "b"), file("003", "c")];
        let applied = vec!["001".to_string(), "003".to_string()];

        let pending = pending_of(&all, &applied);

        assert_eq!(
            pending
                .iter()
                .map(|f| f.version.as_str())
                .collect::<Vec<_>>(),
            vec!["002"]
        );
    }

    #[test]
    fn wrap_migration_brackets_the_file_s_statements_with_begin_commit_and_the_tracking_insert() {
        let f = file("001", "create users");

        let wrapped = wrap_migration(&f, &["CREATE TABLE users (id INT)".to_string()]);

        assert_eq!(
            wrapped,
            vec![
                "BEGIN".to_string(),
                "CREATE TABLE users (id INT)".to_string(),
                "INSERT INTO _tradar_migrations (version, name) VALUES ('001', 'create users')"
                    .to_string(),
                "COMMIT".to_string(),
            ]
        );
    }

    #[test]
    fn wrap_migration_escapes_a_single_quote_in_the_name() {
        let f = file("001", "users' table");

        let wrapped = wrap_migration(&f, &[]);

        assert!(wrapped.iter().any(|s| s.contains("'users'' table'")));
    }

    #[test]
    fn listing_with_pending_files_answers_y_to_run() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a")], 0);

        assert!(matches!(
            panel.handle_key_event(KeyCode::Char('y')),
            Some(MigrationsOutcome::RunAll)
        ));
    }

    #[test]
    fn listing_with_nothing_pending_closes_on_any_key_instead_of_running() {
        let mut panel = MigrationsComponent::listing(Vec::new(), 3);

        assert!(matches!(
            panel.handle_key_event(KeyCode::Char('y')),
            Some(MigrationsOutcome::Cancelled)
        ));
    }

    #[test]
    fn start_running_returns_the_first_pending_file_and_switches_stage() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a"), file("002", "b")], 0);

        let first = panel.start_running().unwrap();

        assert_eq!(first.version, "001");
        assert!(matches!(panel.stage, Stage::Running { index: 0, .. }));
    }

    #[test]
    fn advance_moves_to_the_next_pending_file() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a"), file("002", "b")], 0);
        panel.start_running();

        let next = panel.advance().unwrap();

        assert_eq!(next.version, "002");
    }

    #[test]
    fn advance_past_the_last_file_finishes_the_run() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a")], 0);
        panel.start_running();

        let next = panel.advance();

        assert_eq!(next, None);
        assert!(matches!(panel.stage, Stage::Done { count: 1 }));
    }

    #[test]
    fn a_running_panel_ignores_every_key() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a")], 0);
        panel.start_running();

        assert!(panel.handle_key_event(KeyCode::Esc).is_none());
        assert!(panel.handle_key_event(KeyCode::Char('y')).is_none());
    }

    #[test]
    fn fail_moves_to_blocked_with_the_reason_shown() {
        let mut panel = MigrationsComponent::listing(vec![file("001", "a")], 0);
        panel.start_running();

        panel.fail("boom".to_string());

        assert!(matches!(panel.stage, Stage::Blocked(ref r) if r == "boom"));
        assert!(matches!(
            panel.handle_key_event(KeyCode::Enter),
            Some(MigrationsOutcome::Cancelled)
        ));
    }
}
