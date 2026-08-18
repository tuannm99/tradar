//! The post-connect screen: query editor + results, composed. The schema
//! tree lives in the app shell's navigator, not here -- see `outline`.
//! Implements `Component` because `RootComponent` routes keys and
//! ticks to it directly whenever it's the active screen. Owns the
//! `QueryEngine` directly (not through `dyn Session`) since this screen only
//! ever exists for a query-shaped connector's own engine.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use tradar_connector_spi::Session;
use tradar_core::action::{Action, Component, OutlineEntry};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedConnection;
use tradar_core::ui;
use tradar_core::vim_list::VimMove;

use crate::components::browse_sidebar::{BrowseClick, BrowseSidebarComponent};
use crate::components::completion::{CompletionPopup, CompletionSource};
use crate::components::file_picker::{FilePickerComponent, PickerOutcome};
use crate::components::file_prompt::{FilePromptComponent, PromptKind, PromptOutcome};
use crate::components::history_picker::{HistoryOutcome, HistoryPickerComponent};
use crate::components::query_editor::{Dialect, EditorMode, QueryEditorComponent};
use crate::components::results::ResultsComponent;
use crate::components::row_edit::{RowEditComponent, RowEditOutcome};
use crate::components::snippet_picker::{SnippetOutcome, SnippetPickerComponent};
use crate::query_driver::{RowChange, RowEdit, SchemaInfo};
use crate::query_engine::{QueryEngine, QueryOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
    /// The Redis key-browser sidebar has focus. Only reachable when
    /// `mode == ScreenMode::Browse`.
    Browse,
}

/// Whether a Redis query screen shows its key-browser sidebar or the usual
/// editor+results console. Irrelevant (stays `Console`, never toggled) for
/// every other connector -- `browse` is `None` there, and
/// `Command::ToggleBrowseMode` is a no-op with nothing to switch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Browse,
    Console,
}

pub struct QueryScreenComponent {
    pub focus: Focus,
    pub query_editor: QueryEditorComponent,
    pub results: ResultsComponent,
    engine: QueryEngine,
    /// Half-finished two-key binding (the first `g` of `gg`).
    pending: Option<KeyPress>,
    prompt: Option<FilePromptComponent>,
    /// The open-a-query overlay, when up.
    picker: Option<FilePickerComponent>,
    last_path: Option<String>,
    history_picker: Option<HistoryPickerComponent>,
    /// The "name this snippet" prompt, open after `Ctrl+K`.
    snippet_prompt: Option<ui::TextInput>,
    /// The snippet library overlay, open after `Ctrl+L`.
    snippet_picker: Option<SnippetPickerComponent>,
    /// Where the editor was last drawn, so a click there can focus it.
    editor_area: Rect,
    /// Everything completable for this connection, built once on connect.
    completions: CompletionSource,
    /// The navigator's flattened view of this connection's schema, built
    /// once on connect rather than re-derived from `engine.schema()` (a
    /// walk that clones every table/column name and type) on every single
    /// navigator keypress and redraw -- `engine.schema()` never changes
    /// after connect, so there's nothing to invalidate this on.
    outline: Vec<OutlineEntry>,
    /// The suggestion list, present only while there is something to
    /// suggest -- so "no popup" and "no matches" are one state.
    completion: Option<CompletionPopup>,
    /// The statement whose result is on screen, kept so an edit made in the
    /// grid knows which table those rows came from -- and so the grid can
    /// be re-read after the edit.
    last_query: Option<String>,
    /// The edit-a-row overlay, when up.
    row_edit: Option<RowEditComponent>,
    /// The next result is a re-read of the same query after an edit, so the
    /// cell cursor should stay where it is instead of jumping to the top.
    refreshing: bool,
    /// The results filter being typed. A one-line bar rather than a
    /// centered overlay: it filters what's behind it, so covering that up
    /// would hide the only feedback there is.
    search: Option<ui::TextInput>,
    /// The editor's own incremental search bar (`/` while `Focus::Editor`)
    /// -- a separate field from `search` above: that one live-filters the
    /// results grid, this one jumps the editor's cursor, and the two need
    /// different Enter/Esc behavior, so one field pretending to serve both
    /// would need to remember which mode it was in anyway.
    buffer_search: Option<ui::TextInput>,
    /// Where the cursor sat before `buffer_search` opened, so `Esc` can
    /// jump back to it and each keystroke's incremental preview re-searches
    /// from the same anchor rather than from wherever the previous partial
    /// match landed.
    search_origin: Option<(usize, usize)>,
    /// The last confirmed buffer search, for `n`/`N` to repeat once the bar
    /// itself has closed.
    last_search: Option<String>,
    /// `Some` only for a Redis connection -- see `docs/backlog/mockup-ui-2026-08-15.md`'s "Redis:
    /// key browser". `None` for every other driver, which never shows a
    /// sidebar or leaves `ScreenMode::Console`.
    browse: Option<BrowseSidebarComponent>,
    mode: ScreenMode,
    /// The driver-formatted echo line (`QueryDriver::browse_command`) for
    /// the most recent browse-sidebar `Enter`, shown verbatim under the
    /// results grid -- see `open_selected_key`. `None` before anything's
    /// been browsed yet, or for every connector but Redis.
    last_browse_command: Option<String>,
    /// The editor/results split -- stacked or side by side, and how much
    /// space each pane gets. Only used in `ScreenMode::Console`; Browse
    /// mode has its own fixed sidebar+value layout (a different shape
    /// entirely, not an editor/results pair).
    split: ui::SplitPane,
    /// Open after a right-click on a results row.
    context_menu: Option<ui::ContextMenu>,
    /// The full area this screen was last drawn into -- needed to hit-test
    /// `context_menu` clicks against the exact same bounds it was drawn
    /// with (it clamps its position to stay on screen).
    screen_area: Rect,
}

/// The folder to start browsing/prefilling from: the parent of the most
/// recently used file (`recent[0]`, persisted in `recent.toml` so this
/// survives a restart), falling back to the queries root when there's no
/// recent file yet, or when that file's folder isn't under the queries
/// root at all (it was opened via an absolute path outside it -- browsing
/// stays scoped to the queries directory, see `FilePickerComponent`'s own
/// doc comment). Takes plain data rather than `&QueryFiles` so it's
/// testable without touching the process-global singleton.
fn last_used_dir(recent: &[String], queries_dir: &std::path::Path) -> std::path::PathBuf {
    recent
        .first()
        .and_then(|path| std::path::Path::new(path).parent().map(|p| p.to_path_buf()))
        .filter(|dir| dir.starts_with(queries_dir))
        .unwrap_or_else(|| queries_dir.to_path_buf())
}

/// `schema`, flattened for the navigator: each table at depth 0 with its
/// columns at depth 1. Called once, in `QueryScreenComponent::new` -- the
/// navigator decides what to show; this only says what there is, and a
/// connection's schema never changes after connect, so there's nothing to
/// recompute this for later.
fn flatten_outline(schema: &Result<Vec<SchemaInfo>, String>) -> Vec<OutlineEntry> {
    let Ok(schema) = schema else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for table in schema {
        entries.push(OutlineEntry {
            depth: 0,
            label: table.name.clone(),
            detail: String::new(),
            has_children: !table.columns.is_empty(),
        });
        for column in &table.columns {
            entries.push(OutlineEntry {
                depth: 1,
                label: column.name.clone(),
                // Which columns are the key decides whether the results
                // grid can be edited, so it's worth seeing here.
                detail: if column.primary_key {
                    format!("{} pk", column.type_name)
                } else {
                    column.type_name.clone()
                },
                has_children: false,
            });
        }
    }
    entries
}

impl QueryScreenComponent {
    /// `_action_tx` is part of `Session::build_screen`'s contract (a screen
    /// backed by a firehose-shaped connector may need to push an `Action`
    /// proactively, outside a key press) but this screen has no use for it
    /// yet -- every state change here already goes through `tick()` or a
    /// direct key-driven method call.
    pub fn new(mut engine: QueryEngine, _action_tx: UnboundedSender<Action>) -> Self {
        // `tick()` may already have a query outcome queued up (not possible
        // right after `Connector::connect`, but keeps `engine` in a
        // consistent state regardless of how it was constructed).
        engine.tick();

        let completions =
            CompletionSource::new(engine.keywords(), engine.schema().as_deref().unwrap_or(&[]));
        let outline = flatten_outline(engine.schema());

        let mut query_editor = QueryEditorComponent::new();
        // Only Postgres/SQLite speak real SQL -- Mongo/Elasticsearch/Redis
        // use their own hand-rolled query shapes with no tree-sitter
        // grammar to match, so they stay plain text.
        if matches!(engine.connection().driver.as_str(), "postgres" | "sqlite") {
            query_editor.set_dialect(Dialect::Sql);
        }

        // Only Redis has a browse UI -- see `QueryDriver::browse_entry`.
        // Same driver-id match already used above for `Dialect::Sql`: cheap
        // and doesn't need a new `Capability` for a sidebar exactly one
        // connector uses.
        let browse = (engine.connection().driver.as_str() == "redis")
            .then(|| BrowseSidebarComponent::new(engine.schema()));
        let mode = if browse.is_some() {
            ScreenMode::Browse
        } else {
            ScreenMode::Console
        };

        Self {
            focus: if browse.is_some() {
                Focus::Browse
            } else {
                Focus::Editor
            },
            query_editor,
            results: ResultsComponent::new(),
            engine,
            pending: None,
            prompt: None,
            picker: None,
            last_path: None,
            history_picker: None,
            snippet_prompt: None,
            snippet_picker: None,
            editor_area: Rect::ZERO,
            completions,
            outline,
            completion: None,
            last_query: None,
            row_edit: None,
            refreshing: false,
            search: None,
            buffer_search: None,
            search_origin: None,
            last_search: None,
            browse,
            mode,
            last_browse_command: None,
            split: ui::SplitPane::default(),
            context_menu: None,
            screen_area: Rect::ZERO,
        }
    }

    pub fn active_connection(&self) -> &SavedConnection {
        self.engine.connection()
    }

    /// Runs whatever `command` means for this screen -- shared by keyboard
    /// dispatch (`handle_key_event`) and a right-click context menu's
    /// confirmed choice (`handle_mouse_event`), so a menu item runs
    /// through the exact same code a keyboard shortcut for it would, not a
    /// second copy that can drift out of sync.
    fn dispatch_command(&mut self, command: Command) -> Option<Action> {
        match command {
            Command::Back => return Some(Action::BackToPicker),
            Command::Help => return Some(Action::ShowHelp),
            Command::RunQuery => self.run_statement_at_cursor(),
            Command::RunAll => self.run_all_statements(),
            Command::CancelQuery => {
                if self.engine.cancel() {
                    self.results.set_error("query cancelled".to_string());
                }
            }
            Command::Commit => self.submit_transaction_control("COMMIT"),
            Command::Rollback => self.submit_transaction_control("ROLLBACK"),
            Command::CycleFocus => {
                self.focus = match self.mode {
                    // No editor to land on in browse mode -- cycle between
                    // the sidebar and whatever it last fetched into results.
                    ScreenMode::Browse => match self.focus {
                        Focus::Browse => Focus::Results,
                        _ => Focus::Browse,
                    },
                    ScreenMode::Console => match self.focus {
                        Focus::Editor => Focus::Results,
                        _ => Focus::Editor,
                    },
                };
            }
            Command::SaveFile => self.open_prompt(PromptKind::Save),
            Command::OpenFile => self.open_file_picker(),
            Command::History => self.open_history(),
            Command::SaveSnippet => self.open_snippet_prompt(),
            Command::OpenSnippets => self.open_snippet_picker(),
            Command::ExportCurl => self.export_curl(),
            Command::Export => self.open_export_prompt(),
            Command::Yank => {
                if let Some(text) = self.results.selected_text() {
                    ui::yank_to_clipboard(&text);
                }
            }
            Command::PrevColumn => self.results.prev_column(),
            Command::NextColumn => self.results.next_column(),
            Command::TogglePreview => self.results.toggle_preview(),
            Command::ToggleResultView => self.results.toggle_document_view(),
            Command::EditCell => self.begin_edit_cell(),
            Command::DeleteRow => self.begin_delete_row(),
            Command::Search => {
                self.search = Some(ui::TextInput::new(self.results.filter()));
            }
            Command::RetryQuery => self.retry_failed_query(),
            Command::EditQuery => {
                if self.results.last_error.is_some() {
                    self.focus = Focus::Editor;
                }
            }
            Command::CopyError => {
                if let Some(error) = self.results.last_error.clone() {
                    ui::yank_to_clipboard(&error);
                }
            }
            Command::BrowseOpen => self.open_selected_key(),
            Command::ToggleBrowseMode => self.toggle_browse_mode(),
            Command::SearchInBuffer => self.open_buffer_search(),
            Command::SearchNext => self.repeat_buffer_search(false),
            Command::SearchPrev => self.repeat_buffer_search(true),
            Command::Undo => self.query_editor.undo(),
            Command::Redo => self.query_editor.redo(),
            // No-op in Browse mode: it has its own fixed sidebar+value
            // layout, not an editor/results pair to resize or reorient.
            Command::ToggleSplitOrientation if self.mode != ScreenMode::Browse => {
                self.split.toggle_orientation();
            }
            Command::ZoomIn if self.mode != ScreenMode::Browse => {
                self.split.zoom_in(self.focus == Focus::Editor);
            }
            Command::ZoomOut if self.mode != ScreenMode::Browse => {
                self.split.zoom_out(self.focus == Focus::Editor);
            }
            _ => {}
        }
        None
    }

    fn export_curl(&self) {
        let query = self.query_editor.text();
        let Some(curl) = self.engine.export_curl(&query) else {
            return;
        };
        let script = format!("#!/usr/bin/env bash\n{curl}\n");
        let _ = std::fs::write("./tradar-query.sh", script);
    }

    fn open_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(FilePromptComponent::new(kind, &self.prompt_prefill()));
    }

    /// `Ctrl+E`: no prefill -- unlike `Save`/`Open`, an export path has
    /// nothing to do with the query file you last worked on.
    fn open_export_prompt(&mut self) {
        self.prompt = Some(FilePromptComponent::new(PromptKind::Export, ""));
    }

    /// What the save/open prompt starts with: the file you last worked on
    /// **this session**, shown by bare name when it's in the queries
    /// directory. The full path would be technically the same target but
    /// fills the prompt with noise you then have to delete to save under
    /// another name.
    ///
    /// With nothing saved/opened yet this session (`last_path` is `None`
    /// -- the common first-`Ctrl+S`-after-launch case), falls back to the
    /// *folder* of the most recently used file across all past sessions
    /// (`recent.toml`, via `last_used_dir`) -- not the file itself, since
    /// prefilling an old filename would look like "overwrite this" rather
    /// than "save a new query here". Only the folder, with a trailing `/`,
    /// so the cursor lands ready to type a fresh name into the right
    /// place.
    fn prompt_prefill(&self) -> String {
        if let Some(last) = self.last_path.as_deref() {
            return tradar_core::storage::query_files()
                .and_then(|files| {
                    std::path::Path::new(last)
                        .strip_prefix(files.dir())
                        .ok()
                        .map(|name| name.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| last.to_string());
        }
        let Some(files) = tradar_core::storage::query_files() else {
            return String::new();
        };
        let dir = last_used_dir(&files.recent(), files.dir());
        match dir.strip_prefix(files.dir()) {
            Ok(relative) if !relative.as_os_str().is_empty() => {
                format!("{}/", relative.display())
            }
            _ => String::new(),
        }
    }

    /// Opens the file picker, or falls back to a typed path when there's
    /// no queries directory configured (tests, mainly). Starts browsing
    /// from `last_used_dir` rather than always the queries root, so
    /// `Ctrl+O` lands you back where you were working without extra
    /// navigation.
    fn open_file_picker(&mut self) {
        match tradar_core::storage::query_files() {
            Some(files) => {
                let start_dir = last_used_dir(&files.recent(), files.dir());
                self.picker = Some(FilePickerComponent::new(
                    &files.recent(),
                    files.dir(),
                    &start_dir,
                ));
            }
            None => self.open_prompt(PromptKind::Open),
        }
    }

    fn open_query_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        self.query_editor.set_text(&content);
        self.remember(path);
        Ok(())
    }

    /// Records a file as recently used, both in memory (to prefill the
    /// next save prompt) and on disk.
    fn remember(&mut self, path: &std::path::Path) {
        self.last_path = Some(path.to_string_lossy().to_string());
        if let Some(files) = tradar_core::storage::query_files() {
            files.record(path);
        }
    }

    fn open_history(&mut self) {
        if self.engine.history().is_empty() {
            return;
        }
        let entries = self.engine.history().iter().rev().cloned().collect();
        self.history_picker = Some(HistoryPickerComponent::new(entries));
    }

    /// `Ctrl+K`: prompts for a name, then saves the whole buffer into the
    /// snippet library under it -- see `handle_snippet_name_confirmed`.
    fn open_snippet_prompt(&mut self) {
        self.snippet_prompt = Some(ui::TextInput::new(""));
    }

    /// What a history picker outcome means -- shared by keyboard `Enter`
    /// and a double-click on a row (`handle_mouse_event`), so both run the
    /// exact same load-into-editor code.
    fn handle_history_outcome(&mut self, outcome: Option<HistoryOutcome>) {
        match outcome {
            Some(HistoryOutcome::Cancelled) => self.history_picker = None,
            Some(HistoryOutcome::Selected(query)) => {
                self.query_editor.set_text(&query);
                self.mode = ScreenMode::Console;
                self.focus = Focus::Editor;
                self.history_picker = None;
            }
            None => {}
        }
    }

    /// What a snippet picker outcome means -- shared by keyboard dispatch
    /// and mouse activation (double-click, or the picker's own right-click
    /// menu), same reasoning as `handle_history_outcome`.
    fn handle_snippet_picker_outcome(&mut self, outcome: Option<SnippetOutcome>) {
        match outcome {
            Some(SnippetOutcome::Cancelled) => self.snippet_picker = None,
            Some(SnippetOutcome::Insert(text)) => {
                self.query_editor.set_text(&text);
                self.mode = ScreenMode::Console;
                self.focus = Focus::Editor;
                self.snippet_picker = None;
            }
            None => {}
        }
    }

    fn handle_snippet_name_confirmed(&mut self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(store) = tradar_core::storage::snippets() {
            store.save(
                name,
                self.active_connection().driver.clone(),
                self.query_editor.text(),
            );
        }
    }

    /// `Ctrl+L`: opens the snippet library scoped to this connection's
    /// driver.
    fn open_snippet_picker(&mut self) {
        let driver = self.active_connection().driver.clone();
        let entries = tradar_core::storage::snippets()
            .map(|s| s.for_driver(&driver))
            .unwrap_or_default();
        self.snippet_picker = Some(SnippetPickerComponent::new(driver, entries));
    }

    /// Fetches the sidebar's highlighted key into the results pane -- a
    /// no-op for every connector but Redis, since `self.browse` is only
    /// ever `Some` there.
    fn open_selected_key(&mut self) {
        let Some(entry) = self.browse.as_ref().and_then(|b| b.selected_entry()) else {
            return;
        };
        self.last_browse_command = self.engine.browse_command(entry);
        self.engine.submit_browse(entry.clone());
        self.focus = Focus::Results;
    }

    /// Switches between the browse sidebar and the raw-command console --
    /// a no-op when there's no sidebar to switch to (every connector but
    /// Redis).
    fn toggle_browse_mode(&mut self) {
        if self.browse.is_none() {
            return;
        }
        self.mode = match self.mode {
            ScreenMode::Browse => ScreenMode::Console,
            ScreenMode::Console => ScreenMode::Browse,
        };
        self.focus = match self.mode {
            ScreenMode::Browse => Focus::Browse,
            ScreenMode::Console => Focus::Editor,
        };
    }

    /// `/` while the editor has focus: opens the incremental buffer-search
    /// bar. Only from the editor's own Normal mode -- typing `/` in Insert
    /// mode is already handled as a literal character before this is ever
    /// reached (see the plain-char-in-Insert passthrough), and Visual mode
    /// deliberately doesn't support search-as-a-motion (real vim does;
    /// out of scope here -- see `docs/roadmap.md`).
    fn open_buffer_search(&mut self) {
        if self.query_editor.mode != EditorMode::Normal {
            return;
        }
        self.search_origin = Some(self.query_editor.cursor());
        self.buffer_search = Some(ui::TextInput::new(""));
    }

    /// `n`/`N`: repeats `last_search`, same Normal-mode-only restriction as
    /// `open_buffer_search`. A no-op if nothing's been searched yet.
    fn repeat_buffer_search(&mut self, backwards: bool) {
        if self.query_editor.mode != EditorMode::Normal {
            return;
        }
        let Some(pattern) = self.last_search.clone() else {
            return;
        };
        self.query_editor.find(&pattern, backwards);
    }

    /// Hands a key the screen doesn't claim to the editor -- but only when
    /// the editor has focus, since a list pane swallowing an unbound key is
    /// better than it silently editing the query behind the user's back.
    fn forward_to_editor(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if self.focus == Focus::Editor {
            self.query_editor
                .forward_key(KeyEvent::new(code, modifiers));
            self.refresh_completions();
        }
        None
    }

    /// Runs just the statement the cursor is in, so a file can hold a
    /// stack of queries and you run them one at a time. Falls back to the
    /// last statement when the cursor sits past the end (on the trailing
    /// blank line, which is where it usually is after typing).
    fn run_statement_at_cursor(&mut self) {
        if self.engine.is_pending() {
            return;
        }
        let text = self.query_editor.text();
        let statements = self.engine.split_statements(&text);
        let cursor = self.query_editor.cursor_offset();
        let statement = statements
            .iter()
            .find(|s| cursor >= s.start && cursor <= s.end)
            .or_else(|| statements.last());
        if let Some(statement) = statement {
            self.last_query = Some(statement.text.clone());
            self.engine.submit_query(statement.text.clone());
        }
    }

    /// `r` on a failed result: re-runs the exact statement that just
    /// failed, rather than re-reading the cursor position the way `F5`
    /// does -- the cursor may well have moved since. A no-op with no error
    /// showing, or nothing to retry (there always is one by the time an
    /// error can be showing, but `last_query` is still an `Option` for
    /// every other caller's sake).
    fn retry_failed_query(&mut self) {
        if self.results.last_error.is_none() || self.engine.is_pending() {
            return;
        }
        let Some(query) = self.last_query.clone() else {
            return;
        };
        self.engine.submit_query(query);
    }

    /// `COMMIT`/`ROLLBACK` shortcuts (`F8`/`F9`) -- submitted the same way
    /// any other statement is, since the driver's own `execute` already
    /// knows how to route these (see `query_driver::transaction_control`).
    /// Deliberately does **not** touch `last_query`: that field names the
    /// table the results grid can still edit through, and a commit or
    /// rollback isn't a new result to edit -- overwriting it here would
    /// break in-place cell edits against whatever `SELECT` is still on
    /// screen.
    fn submit_transaction_control(&mut self, statement: &str) {
        if self.engine.is_pending() {
            return;
        }
        self.engine.submit_query(statement.to_string());
    }

    fn run_all_statements(&mut self) {
        if self.engine.is_pending() {
            return;
        }
        let text = self.query_editor.text();
        let statements: Vec<String> = self
            .engine
            .split_statements(&text)
            .into_iter()
            .map(|s| s.text)
            .collect();
        if !statements.is_empty() {
            // The result on screen is the last statement's, so that's the
            // one an edit in the grid would be editing through.
            self.last_query = statements.last().cloned();
            self.engine.submit_all(statements);
        }
    }

    /// The table the result on screen can be edited through, and the
    /// key columns identifying the selected row. `Err` carries the reason
    /// it can't be, phrased for the user -- a grid that refuses a keystroke
    /// without saying why reads as broken.
    fn editable_row(&self) -> Result<(String, Vec<(String, String)>), String> {
        let query = self
            .last_query
            .as_deref()
            .ok_or("nothing has been run yet")?;
        let table = self.engine.edit_source(query).ok_or(
            "only a plain SELECT from a single table can be edited here — \
             a join, a grouped result or a subquery has no one row to change",
        )?;

        let schema = self
            .engine
            .schema()
            .as_ref()
            .map_err(|e| format!("the schema for this connection wasn't read: {e}"))?;
        // A Postgres source is schema-qualified (`public.users`) while the
        // sidebar lists bare names, so match on the last part.
        let bare = table.rsplit('.').next().unwrap_or(&table);
        let info = schema
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(bare))
            .ok_or_else(|| format!("'{table}' isn't in this connection's schema"))?;

        let key_columns: Vec<&str> = info
            .columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.as_str())
            .collect();
        if key_columns.is_empty() {
            return Err(format!(
                "'{table}' has no primary key — there is no WHERE clause that names exactly one row"
            ));
        }

        let columns = self.results.columns();
        let row = self.results.selected_row().ok_or("no row is selected")?;
        let mut key = Vec::with_capacity(key_columns.len());
        for name in key_columns {
            let index = columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    format!("the key column '{name}' isn't in this result — select it too")
                })?;
            let value = row.get(index).cloned().unwrap_or_default();
            key.push((name.to_string(), value));
        }
        Ok((table, key))
    }

    fn begin_edit_cell(&mut self) {
        let Some((column, value)) = self
            .results
            .selected_cell()
            .map(|(column, value)| (column.to_string(), value.to_string()))
        else {
            return;
        };
        self.row_edit = Some(match self.editable_row() {
            Ok(_) => RowEditComponent::value(&column, &value),
            Err(reason) => RowEditComponent::blocked("Edit cell", reason),
        });
    }

    fn begin_delete_row(&mut self) {
        if self.results.selected_row().is_none() {
            return;
        }
        self.row_edit = Some(match self.build_edit(RowChange::DeleteRow) {
            Ok(sql) => RowEditComponent::confirm("Delete row", sql),
            Err(reason) => RowEditComponent::blocked("Delete row", reason),
        });
    }

    /// The statement for `change` against the selected row, as this
    /// driver would write it.
    fn build_edit(&self, change: RowChange) -> Result<String, String> {
        let (table, key) = self.editable_row()?;
        self.engine
            .edit_sql(&RowEdit { table, key, change })
            .ok_or_else(|| "this connection's results can't be edited in place".to_string())
    }

    fn handle_row_edit(&mut self, outcome: RowEditOutcome) {
        match outcome {
            RowEditOutcome::Cancelled => self.row_edit = None,
            RowEditOutcome::ValueEntered(value) => {
                let Some((column, _)) = self.results.selected_cell() else {
                    self.row_edit = None;
                    return;
                };
                let built = self.build_edit(RowChange::SetValue {
                    column: column.to_string(),
                    value,
                });
                if let Some(overlay) = &mut self.row_edit {
                    match built {
                        Ok(sql) => overlay.show_statement(sql),
                        Err(reason) => overlay.show_problem(reason),
                    }
                }
            }
            RowEditOutcome::Confirmed(sql) => {
                self.row_edit = None;
                self.run_edit(sql);
            }
        }
    }

    /// Runs the edit and then re-reads the query behind the grid, as one
    /// submission: otherwise the pane would replace the table with "OK — 1
    /// row affected" and you'd have to re-run the SELECT by hand to see
    /// what you just did. Running them together also means a failed edit
    /// stops before the re-read, so the error is what's left on screen.
    fn run_edit(&mut self, sql: String) {
        let Some(query) = self.last_query.clone() else {
            return;
        };
        self.refreshing = true;
        self.engine.submit_all(vec![sql, query]);
    }

    /// Each column's declared type for the result currently on screen, for
    /// `ResultsComponent` to show in its header/preview -- see
    /// `query_driver::column_types`. Only meaningful for a `Table` result
    /// whose source is a single known table (`engine.edit_source`, the
    /// same lookup row-editing already relies on); empty for anything else
    /// (a write, a join, `Documents`), which is exactly `column_types`'
    /// own "nothing known" case for `table: None`.
    fn column_types(&self) -> Vec<Option<String>> {
        let Some(crate::query_driver::QueryResult::Table { columns, .. }) =
            &self.results.last_result
        else {
            return Vec::new();
        };
        let table = self
            .last_query
            .as_deref()
            .and_then(|query| self.engine.edit_source(query));
        crate::query_driver::column_types(
            columns,
            self.engine.schema().as_deref().unwrap_or(&[]),
            table.as_deref(),
        )
    }

    /// Rebuilds the suggestion list for the word under the cursor. Only
    /// while typing: a popup in Normal mode would cover the query while
    /// you're navigating it, and there's nothing being typed to complete.
    fn refresh_completions(&mut self) {
        if self.focus != Focus::Editor || self.query_editor.mode != EditorMode::Insert {
            self.completion = None;
            return;
        }
        let matches = self
            .completions
            .matches(&self.query_editor.word_before_cursor());
        match (&mut self.completion, matches.is_empty()) {
            (_, true) => self.completion = None,
            (Some(popup), false) => popup.set_items(matches),
            (slot @ None, false) => *slot = Some(CompletionPopup::new(matches)),
        }
    }

    fn accept_completion(&mut self) {
        let Some(text) = self
            .completion
            .as_ref()
            .and_then(|popup| popup.selected_text())
            .map(str::to_string)
        else {
            return;
        };
        self.query_editor.replace_word_before_cursor(&text);
        // The word is complete now, so there is nothing left to suggest --
        // leaving the popup up would offer to complete what was just
        // accepted.
        self.completion = None;
    }

    /// Scrolls whichever pane the pointer is over, rather than whichever
    /// has focus -- that's what a wheel is expected to do.
    fn scroll_under_cursor(&mut self, event: MouseEvent, mv: VimMove) {
        if ui::contains(self.editor_area, event.column, event.row) {
            self.query_editor.scroll(mv);
        } else {
            self.results.apply_move(mv);
        }
    }

    fn handle_prompt_confirmed(&mut self, path: String) {
        let trimmed = path.trim().to_string();
        if trimmed.is_empty() {
            if let Some(prompt) = &mut self.prompt {
                prompt.error = Some("path must not be empty".to_string());
            }
            return;
        }
        let Some(kind) = self.prompt.as_ref().map(|p| p.kind) else {
            return;
        };
        // A bare name lands in the queries directory; anything path-shaped
        // is used as typed. Export is the odd one out: its output isn't a
        // query, so there's no queries-directory concept for it -- a bare
        // name is just a relative path from the cwd.
        let path = match kind {
            PromptKind::Save | PromptKind::Open => match tradar_core::storage::query_files() {
                Some(files) => tradar_core::storage::resolve_query_path(&trimmed, files.dir()),
                None => std::path::PathBuf::from(&trimmed),
            },
            PromptKind::Export => std::path::PathBuf::from(&trimmed),
        };
        let result = match kind {
            PromptKind::Save => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, self.query_editor.text()).map_err(|e| e.to_string())
            }
            PromptKind::Open => self.open_query_file(&path),
            PromptKind::Export => self.export_result(&path),
        };
        match result {
            Ok(()) => {
                // Export files aren't queries -- keep them out of the
                // Ctrl+O recent-files list and away from Ctrl+S's prefill.
                if kind != PromptKind::Export {
                    self.remember(&path);
                }
                self.prompt = None;
            }
            Err(e) => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.error = Some(e);
                }
            }
        }
    }

    /// `Ctrl+E`: writes the current result to `path` as CSV or JSON,
    /// picked by its extension -- see `crate::export`. Always the full
    /// result, not whatever `/` has the grid filtered down to right now.
    fn export_result(&mut self, path: &std::path::Path) -> Result<(), String> {
        let Some(result) = self.results.last_result.as_ref() else {
            return Err("nothing to export".to_string());
        };
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let content = match extension.as_deref() {
            Some("csv") => crate::export::to_csv(result),
            Some("json") => crate::export::to_json(result),
            _ => Err("export path must end in .csv or .json".to_string()),
        }?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, content).map_err(|e| e.to_string())
    }
}

impl Component for QueryScreenComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(menu) = self.context_menu.as_mut() {
            match menu.handle_key_event(code) {
                ui::ContextMenuOutcome::Open => {}
                ui::ContextMenuOutcome::Closed => self.context_menu = None,
                ui::ContextMenuOutcome::Confirmed(command) => {
                    self.context_menu = None;
                    return self.dispatch_command(command);
                }
            }
            return None;
        }

        if let Some(prompt) = self.prompt.as_mut() {
            match prompt.handle_key_event(code, modifiers) {
                Some(PromptOutcome::Cancelled) => self.prompt = None,
                Some(PromptOutcome::Confirmed(path)) => self.handle_prompt_confirmed(path),
                None => {}
            }
            return None;
        }

        // Typed straight into the filter, refiltering on every key: a
        // search you have to confirm before seeing anything is a search you
        // can't correct while typing.
        if let Some(search) = self.search.as_mut() {
            match code {
                KeyCode::Esc => {
                    self.search = None;
                    // Esc undoes the whole search, so the grid is back to
                    // exactly what it was before `/`.
                    self.results.set_filter("");
                }
                KeyCode::Enter => self.search = None,
                _ => {
                    search.handle_key_event(code, modifiers);
                    let text = search.text();
                    self.results.set_filter(&text);
                }
            }
            return None;
        }

        // Same incremental shape as the results filter above, but jumping
        // the editor's cursor instead of narrowing a grid: every keystroke
        // re-searches from `search_origin`, not from wherever the previous
        // partial match landed, so typing a longer prefix doesn't drift.
        if let Some(buffer_search) = self.buffer_search.as_mut() {
            match code {
                KeyCode::Esc => {
                    self.buffer_search = None;
                    if let Some((row, col)) = self.search_origin.take() {
                        self.query_editor.set_cursor(row, col);
                    }
                }
                KeyCode::Enter => {
                    let pattern = buffer_search.text();
                    self.buffer_search = None;
                    self.search_origin = None;
                    if !pattern.is_empty() {
                        self.last_search = Some(pattern);
                    }
                }
                _ => {
                    buffer_search.handle_key_event(code, modifiers);
                    let pattern = buffer_search.text();
                    if let Some((row, col)) = self.search_origin {
                        self.query_editor.set_cursor(row, col);
                    }
                    self.query_editor.find(&pattern, false);
                }
            }
            return None;
        }

        if let Some(row_edit) = self.row_edit.as_mut() {
            if let Some(outcome) = row_edit.handle_key_event(code, modifiers) {
                self.handle_row_edit(outcome);
            }
            return None;
        }

        if let Some(picker) = self.picker.as_mut() {
            match picker.handle_key_event(code, modifiers) {
                Some(PickerOutcome::Cancelled) => self.picker = None,
                Some(PickerOutcome::Chosen(path)) => {
                    match self.open_query_file(&path) {
                        Ok(()) => self.picker = None,
                        // Keep the overlay up with the failure showing --
                        // closing it would leave no sign the open failed.
                        Err(e) => self.results.set_error(format!("{}: {e}", path.display())),
                    }
                }
                None => {}
            }
            return None;
        }

        if let Some(history_picker) = self.history_picker.as_mut() {
            let outcome = history_picker.handle_key_event(code, modifiers);
            self.handle_history_outcome(outcome);
            return None;
        }

        if let Some(prompt) = self.snippet_prompt.as_mut() {
            match code {
                KeyCode::Esc => self.snippet_prompt = None,
                KeyCode::Enter => {
                    let name = prompt.text();
                    self.snippet_prompt = None;
                    self.handle_snippet_name_confirmed(name);
                }
                _ => {
                    prompt.handle_key_event(code, modifiers);
                }
            }
            return None;
        }

        if let Some(snippet_picker) = self.snippet_picker.as_mut() {
            let outcome = snippet_picker.handle_key_event(code, modifiers);
            self.handle_snippet_picker_outcome(outcome);
            return None;
        }

        // While suggestions are showing they take the keys bound to them,
        // and nothing else -- every other key falls through to normal
        // editing, which then refilters the list.
        if self.completion.is_some() {
            let key = KeyPress::new(code, modifiers);
            let mut pending = None;
            if let Resolution::Command(command) =
                keymap().resolve(Context::Completion, &mut pending, key)
            {
                match command {
                    Command::AcceptCompletion => {
                        self.accept_completion();
                        return None;
                    }
                    Command::NextCompletion => {
                        if let Some(popup) = &mut self.completion {
                            popup.next();
                        }
                        return None;
                    }
                    Command::PrevCompletion => {
                        if let Some(popup) = &mut self.completion {
                            popup.prev();
                        }
                        return None;
                    }
                    _ => {}
                }
            }
        }

        // `Esc` belongs to the editor while it's in a mode it can leave
        // (Insert), and only means "back to the picker" from Normal mode --
        // or, with vim mode off, always: there's no Normal mode there to
        // drop into first, so every `Esc` goes straight through to the
        // keymap's `back` binding below. Checked before the keymap so a
        // user rebinding `back` can't accidentally trap themselves in
        // Insert mode.
        if code == KeyCode::Esc
            && self.focus == Focus::Editor
            && self.query_editor.vim_enabled()
            && self.query_editor.mode != EditorMode::Normal
        {
            self.query_editor
                .forward_key(KeyEvent::new(code, modifiers));
            self.completion = None;
            return None;
        }

        // While typing, a plain character is text -- never a command. Only
        // modified keys (`ctrl-s`, `f5`) reach the keymap from Insert mode,
        // which is what keeps a binding like `?` from being un-typeable.
        if self.focus == Focus::Editor
            && self.query_editor.mode == EditorMode::Insert
            && matches!(code, KeyCode::Char(_))
            && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.query_editor
                .forward_key(KeyEvent::new(code, modifiers));
            self.refresh_completions();
            return None;
        }

        // Only the focused pane's context is offered, so `l` can mean
        // "next column" in the results without colliding with the
        // navigator's own `l`. With the editor focused, neither the results
        // keys nor list navigation apply -- those are the editor's own vim
        // motions.
        let contexts: &[Context] = match self.focus {
            Focus::Editor => &[Context::QueryScreen, Context::Editor],
            Focus::Results => &[Context::QueryScreen, Context::Results, Context::List],
            Focus::Browse => &[Context::QueryScreen, Context::Browse, Context::List],
        };
        let key = KeyPress::new(code, modifiers);
        let command = match keymap().resolve_in(contexts, &mut self.pending, key) {
            Resolution::Command(command) => command,
            // Mid-sequence (the first `g` of `gg`): swallow it so the
            // editor doesn't also see it.
            Resolution::Pending => return None,
            Resolution::None => return self.forward_to_editor(code, modifiers),
        };

        if let Some(mv) = command.as_vim_move() {
            match self.focus {
                Focus::Results => self.results.apply_move(mv),
                Focus::Browse => {
                    if let Some(browse) = self.browse.as_mut() {
                        browse.apply_move(mv);
                    }
                }
                Focus::Editor => {}
            }
            return None;
        }

        self.dispatch_command(command)
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        // A context menu is its own small overlay: a left click either
        // hits one of its items or dismisses it (clicking away closes a
        // popup, standard behavior) -- either way nothing behind it should
        // also react to the same click.
        if let Some(menu) = self.context_menu.take() {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(command) = menu.click(self.screen_area, event.column, event.row)
            {
                return self.dispatch_command(command);
            }
            return None;
        }

        if let Some(history_picker) = self.history_picker.as_mut() {
            let outcome = history_picker.handle_mouse_event(event);
            self.handle_history_outcome(outcome);
            return None;
        }

        if let Some(snippet_picker) = self.snippet_picker.as_mut() {
            let outcome = snippet_picker.handle_mouse_event(event);
            self.handle_snippet_picker_outcome(outcome);
            return None;
        }

        // An overlay covers the screen, so a click behind it would act on
        // something the user can't even see.
        if self.prompt.is_some()
            || self.picker.is_some()
            || self.row_edit.is_some()
            || self.snippet_prompt.is_some()
        {
            return None;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Clicking a pane focuses it, which is the main thing a
                // mouse is for here -- and then selects the row hit.
                if self.results.click(event.column, event.row) {
                    self.focus = Focus::Results;
                } else if let Some(browse) = self.browse.as_mut() {
                    match browse.click(event.column, event.row) {
                        BrowseClick::Missed => {}
                        BrowseClick::Selected => self.focus = Focus::Browse,
                        // A double-click opens the key, same as
                        // `Command::BrowseOpen` -- `open_selected_key`
                        // reads `self.browse`'s own `selected_entry()`
                        // (which `click` just updated) and moves focus to
                        // `Results` itself, same as the keyboard shortcut.
                        BrowseClick::Activated => self.open_selected_key(),
                    }
                } else if ui::contains(self.editor_area, event.column, event.row) {
                    self.focus = Focus::Editor;
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // Same hit test as a left click (selects the row/cell too),
                // then offers whatever a keyboard shortcut could already do
                // to it -- see `dispatch_command`'s doc comment for why
                // confirming a menu item runs through the exact same code.
                if self.results.click(event.column, event.row) {
                    self.focus = Focus::Results;
                    let items = vec![
                        ("Edit cell".to_string(), Command::EditCell),
                        ("Delete row".to_string(), Command::DeleteRow),
                        ("Yank".to_string(), Command::Yank),
                        ("Toggle preview".to_string(), Command::TogglePreview),
                        ("Toggle table/JSON".to_string(), Command::ToggleResultView),
                    ];
                    self.context_menu =
                        Some(ui::ContextMenu::new((event.column, event.row), items));
                }
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                // The X11 middle-click-pastes convention, applied to the
                // one text-entry surface this screen has -- the query
                // editor. `insert_at_cursor` is the same method the
                // navigator uses to insert a table/column name, so a paste
                // lands exactly where typing would.
                if ui::contains(self.editor_area, event.column, event.row) {
                    self.focus = Focus::Editor;
                    if let Some(text) = ui::paste_from_clipboard() {
                        self.query_editor.insert_at_cursor(&text);
                    }
                }
            }
            MouseEventKind::ScrollDown => self.scroll_under_cursor(event, VimMove::Down),
            MouseEventKind::ScrollUp => self.scroll_under_cursor(event, VimMove::Up),
            _ => {}
        }
        None
    }

    fn restore_state(&self) -> Option<String> {
        let text = self.query_editor.text();
        // An empty editor has nothing worth carrying to the next run.
        (!text.trim().is_empty()).then_some(text)
    }

    /// This connection's schema, flattened for the navigator -- computed
    /// once in `new()` (see `outline` field) and just handed back here,
    /// not re-derived on every call.
    fn outline(&self) -> Vec<OutlineEntry> {
        self.outline.clone()
    }

    fn outline_error(&self) -> Option<String> {
        self.engine.schema().as_ref().err().cloned()
    }

    fn connection_alive(&self) -> Option<bool> {
        Some(self.engine.alive())
    }

    fn status_hints(&self) -> Vec<ui::Hint> {
        let mut hints = Vec::new();
        // Only worth advertising while there's actually an error on
        // screen to retry/edit/copy -- the keys do nothing otherwise.
        if self.results.last_error.is_some() {
            hints.extend(ui::hint(Context::Results, Command::RetryQuery, "retry"));
            hints.extend(ui::hint(Context::Results, Command::EditQuery, "edit"));
            hints.extend(ui::hint(Context::Results, Command::CopyError, "copy"));
        }
        hints.extend(ui::hint(Context::QueryScreen, Command::RunQuery, "run"));
        hints.extend(ui::hint(Context::QueryScreen, Command::CycleFocus, "focus"));
        hints.extend(ui::hint(Context::QueryScreen, Command::History, "history"));
        hints.extend(ui::hint(Context::QueryScreen, Command::Back, "back"));
        hints
    }

    fn crud_snippet(&self, name: &str, op: tradar_core::action::CrudOp) -> Option<String> {
        self.engine.crud_snippet(name, op)
    }

    fn insert_text(&mut self, text: &str) {
        self.query_editor.insert_at_cursor(text);
        // The navigator inserts into the editor, so make sure it's actually
        // on screen -- a no-op for every connector but Redis, which is the
        // only one `mode` ever leaves `Console` for.
        self.mode = ScreenMode::Console;
        self.focus = Focus::Editor;
    }

    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }

    fn tick(&mut self) -> bool {
        // `true` here covers two unrelated things settling: a query outcome
        // (handled below) and the periodic ping's alive/dead flip (nothing
        // more to do for that one -- `draw()` reads `engine.alive()` fresh
        // every time).
        let changed = self.engine.tick();
        if let Some(outcome) = self.engine.take_outcome() {
            let refreshing = std::mem::take(&mut self.refreshing);
            match outcome {
                QueryOutcome::Completed { result } if refreshing => {
                    self.results.set_result_keeping_cursor(result)
                }
                QueryOutcome::Completed { result } => self.results.set_result(result),
                QueryOutcome::Failed { error } => self.results.set_error(error),
            }
            // Recomputed only now that the result actually changed, not
            // every `draw()` frame -- nothing it depends on (`last_query`,
            // the just-set result, the schema) moves between outcomes, and
            // a query left running redraws ~20x/second for the spinner
            // alone (see `QueryEngine::tick`), which would otherwise mean
            // rebuilding this for no reason on every one of those frames.
            self.results.set_column_types(self.column_types());
        }
        changed
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        self.screen_area = area;
        // The schema tree used to live here as a sidebar; it's now the app
        // shell's navigator, which can show every connection rather than
        // only this screen's own. In console mode the screen is just
        // editor + results (stacked); in Redis browse mode it's the key
        // sidebar + results (side by side), with no editor at all.
        let mut browse_command_area = None;
        let content_area = if self.mode == ScreenMode::Browse {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(20)])
                .split(area);
            if let Some(browse) = self.browse.as_mut() {
                browse.draw(frame, columns[0], self.focus == Focus::Browse);
            }
            self.editor_area = Rect::ZERO;
            if self.last_browse_command.is_some() {
                let (content, bar) = ui::split_bottom_bar(columns[1], 1);
                browse_command_area = Some(bar);
                content
            } else {
                columns[1]
            }
        } else {
            // Stacked or side by side, and how much of the split the
            // editor gets, per `self.split` -- `Ctrl+Up`/`Ctrl+Down` zooms
            // whichever pane has focus, `F6` flips the orientation. The
            // buffer-search bar, when open, is carved off the bottom of
            // whatever the editor got, not off a separately reserved row.
            let (editor_rect, main_rest) = self.split.split(area);
            let (editor_draw_rect, search_bar_rect) = if self.buffer_search.is_some() {
                let (top, bar) = ui::split_bottom_bar(editor_rect, 1);
                (top, Some(bar))
            } else {
                (editor_rect, None)
            };

            let connection_name = self.active_connection().name.clone();
            self.editor_area = editor_draw_rect;
            self.query_editor.draw(
                frame,
                editor_draw_rect,
                &connection_name,
                self.focus == Focus::Editor,
                self.engine.alive(),
                self.engine.in_transaction(),
            );
            if let (Some(buffer_search), Some(bar)) = (&self.buffer_search, search_bar_rect) {
                let theme = tradar_core::theme::theme();
                let mut spans = vec![ratatui::text::Span::styled(
                    "/",
                    ratatui::style::Style::default().fg(theme.accent),
                )];
                spans.extend(buffer_search.spans(true));
                frame.render_widget(
                    ratatui::widgets::Paragraph::new(ratatui::text::Line::from(spans)),
                    bar,
                );
            }
            main_rest
        };

        self.results.draw_running(self.engine.elapsed_running());
        let results_area = match &self.search {
            Some(_) => Rect {
                height: content_area.height.saturating_sub(1),
                ..content_area
            },
            None => content_area,
        };
        self.results
            .draw(frame, results_area, self.focus == Focus::Results);
        if let (Some(area), Some(line)) = (browse_command_area, &self.last_browse_command) {
            // `line` (e.g. `127.0.0.1:6379> HGETALL user:1`) is already
            // fully formatted by the driver -- see `QueryDriver::
            // browse_command`'s doc comment for why this component never
            // builds that string itself.
            let theme = tradar_core::theme::theme();
            frame.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Span::styled(
                    line.as_str(),
                    ratatui::style::Style::default().fg(theme.text),
                )),
                area,
            );
        }
        if let Some(search) = &self.search {
            let bar = Rect {
                y: results_area.y.saturating_add(results_area.height),
                height: 1,
                ..content_area
            };
            let theme = tradar_core::theme::theme();
            let mut spans = vec![ratatui::text::Span::styled(
                "/",
                ratatui::style::Style::default().fg(theme.accent),
            )];
            spans.extend(search.spans(true));
            frame.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Line::from(spans)),
                bar,
            );
        }

        if let Some(completion) = &mut self.completion {
            let cursor = self.query_editor.cursor_screen_position(self.editor_area);
            completion.draw(frame, area, cursor);
        }

        if let Some(picker) = &mut self.picker {
            let popup = ui::centered_rect(70, 60, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            picker.draw(frame, popup);
        }

        if let Some(prompt) = &self.prompt {
            let popup = ui::centered_rect(60, 20, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            let queries_dir = tradar_core::storage::query_files().map(|files| files.dir());
            prompt.draw(frame, popup, queries_dir);
        }

        if let Some(history_picker) = &mut self.history_picker {
            let popup = ui::centered_rect(70, 60, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            history_picker.draw(frame, popup);
        }

        if let Some(prompt) = &self.snippet_prompt {
            let popup = ui::centered_rect(60, 20, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            let block = ui::panel("Save snippet as", true);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);
            frame.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Line::from(prompt.spans(true))),
                inner,
            );
        }

        if let Some(snippet_picker) = &mut self.snippet_picker {
            let popup = ui::centered_rect(70, 60, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            snippet_picker.draw(frame, popup);
        }

        if let Some(row_edit) = &self.row_edit {
            let popup = ui::centered_rect(70, 30, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            row_edit.draw(frame, popup);
        }

        if let Some(menu) = &self.context_menu {
            menu.draw(frame, area);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tokio::sync::mpsc;

    use super::*;
    use crate::query_driver::{QueryDriver, QueryResult, SchemaInfo};

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn last_used_dir_is_the_most_recent_file_s_parent() {
        let queries_dir = std::path::Path::new("/home/x/.config/tradar/queries");
        let recent = vec![
            "/home/x/.config/tradar/queries/reports/q1.sql".to_string(),
            "/home/x/.config/tradar/queries/old.sql".to_string(),
        ];

        assert_eq!(
            last_used_dir(&recent, queries_dir),
            queries_dir.join("reports")
        );
    }

    #[test]
    fn last_used_dir_falls_back_to_the_root_with_no_recent_files() {
        let queries_dir = std::path::Path::new("/home/x/.config/tradar/queries");

        assert_eq!(last_used_dir(&[], queries_dir), queries_dir);
    }

    #[test]
    fn last_used_dir_falls_back_to_the_root_when_the_recent_file_is_outside_it() {
        let queries_dir = std::path::Path::new("/home/x/.config/tradar/queries");
        let recent = vec!["/somewhere/else/report.sql".to_string()];

        assert_eq!(last_used_dir(&recent, queries_dir), queries_dir);
    }

    fn connection() -> SavedConnection {
        SavedConnection {
            name: "local-sqlite".to_string(),
            driver: "sqlite".to_string(),
            target: "test.db".to_string(),
        }
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo::new("users".to_string()),
            SchemaInfo::new("orders".to_string()),
        ]
    }

    struct FakeDriver {
        result: QueryResult,
    }

    #[async_trait]
    impl QueryDriver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn keywords(&self) -> &'static [&'static str] {
            &["ORDER BY", "OR", "SELECT"]
        }
        fn split_statements(&self, text: &str) -> Vec<crate::query_driver::Statement> {
            crate::query_driver::split_sql_statements(text)
        }
        fn edit_sql(&self, edit: &crate::query_driver::RowEdit) -> Option<String> {
            Some(crate::query_driver::build_sql_edit(edit))
        }
        fn edit_source(&self, query: &str) -> Option<String> {
            crate::query_driver::single_table_source(query)
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
        async fn browse_entry(&self, _entry: &SchemaInfo) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
        fn browse_command(&self, entry: &SchemaInfo) -> Option<String> {
            Some(format!("HGETALL {}", entry.name))
        }
    }

    fn fake_engine_with_schema(
        result: QueryResult,
        schema: Result<Vec<SchemaInfo>, String>,
    ) -> QueryEngine {
        QueryEngine::new(Arc::new(FakeDriver { result }), connection(), schema)
    }

    fn fake_engine(result: QueryResult) -> QueryEngine {
        fake_engine_with_schema(result, Ok(Vec::new()))
    }

    fn empty_result() -> QueryResult {
        QueryResult::Table {
            columns: vec![],
            rows: vec![],
            truncated: false,
        }
    }

    fn screen_with(engine: QueryEngine) -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (QueryScreenComponent::new(engine, tx), rx)
    }

    fn screen() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        screen_with(fake_engine(empty_result()))
    }

    #[test]
    fn starts_focused_on_the_editor_with_the_connection_and_schema_loaded() {
        let (screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));

        assert_eq!(screen.focus, Focus::Editor);
        assert_eq!(screen.active_connection(), &connection());
    }

    #[test]
    fn the_outline_offered_to_the_navigator_is_this_connection_s_schema() {
        let (screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));

        let labels: Vec<String> = screen.outline().into_iter().map(|e| e.label).collect();

        assert_eq!(labels, vec!["users", "orders"]);
    }

    #[test]
    fn a_column_is_offered_under_its_table_with_its_type() {
        let schema = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![crate::query_driver::ColumnInfo {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
                primary_key: true,
            }],
            kind: None,
            ttl: None,
        }];
        let (screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema)));

        let outline = screen.outline();

        assert_eq!(outline[0].depth, 0);
        assert!(outline[0].has_children);
        assert_eq!(outline[1].depth, 1);
        assert_eq!(outline[1].detail, "INTEGER pk");
    }

    #[test]
    fn a_schema_error_is_reported_to_the_navigator_rather_than_looking_empty() {
        let (screen, _rx) = screen_with(fake_engine_with_schema(
            empty_result(),
            Err("scan failed".to_string()),
        ));

        assert!(screen.outline().is_empty());
        assert_eq!(screen.outline_error().as_deref(), Some("scan failed"));
    }

    #[test]
    fn tab_cycles_editor_and_results() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(screen.focus, Focus::Results);

        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(screen.focus, Focus::Editor);
    }

    fn type_chars(screen: &mut QueryScreenComponent, chars: &str) {
        for c in chars.chars() {
            screen
                .query_editor
                .forward_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn a_name_from_the_navigator_lands_at_the_cursor_not_at_the_end() {
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));
        // Type "ab" then leave Insert mode -- vim leaves the cursor sitting
        // on the last-typed character ("b"), so the inserted name must land
        // between "a" and "b", not appended at the buffer's end.
        type_chars(&mut screen, "iab");
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        screen.focus = Focus::Results;

        screen.insert_text("orders");

        assert_eq!(screen.query_editor.text(), "aordersb");
        assert_eq!(screen.focus, Focus::Editor, "typing has to go somewhere");
        assert_eq!(screen.query_editor.mode, EditorMode::Insert);
    }

    #[tokio::test]
    async fn submitting_a_query_runs_it_and_shows_the_result_after_a_tick() {
        let (mut screen, _rx) = screen_with(fake_engine(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        }));
        screen.query_editor.insert_at_cursor("x");

        let action = screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);
        assert!(action.is_none());

        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if screen.results.last_result.is_some() {
                break;
            }
        }

        assert_eq!(
            screen.results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            })
        );
    }

    #[tokio::test]
    async fn f8_commits_without_overwriting_last_query() {
        let (mut screen, _rx) = screen_with(fake_engine(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        }));
        screen.query_editor.insert_at_cursor("SELECT 1");
        screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if screen.results.last_result.is_some() {
                break;
            }
        }
        assert_eq!(screen.last_query.as_deref(), Some("SELECT 1"));

        screen.handle_key_event(KeyCode::F(8), KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(
            screen.last_query.as_deref(),
            Some("SELECT 1"),
            "a commit must not change which query the results grid edits through"
        );
        assert_eq!(
            screen.engine.history().last().map(String::as_str),
            Some("COMMIT")
        );
    }

    #[tokio::test]
    async fn f8_and_f9_do_nothing_while_a_query_is_already_running() {
        let (mut screen, _rx) = screen_with(fake_engine(QueryResult::Affected { rows: 0 }));
        screen.query_editor.insert_at_cursor("SELECT 1");
        screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);
        assert!(screen.engine.is_pending());
        let history_before = screen.engine.history().len();

        screen.handle_key_event(KeyCode::F(8), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::F(9), KeyModifiers::NONE);

        assert_eq!(
            screen.engine.history().len(),
            history_before,
            "commit/rollback must not queue up behind the running query"
        );
    }

    #[test]
    fn g_is_forwarded_to_the_editor_rather_than_being_a_list_motion() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        let first = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        let second = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);

        assert!(first.is_none());
        assert!(
            second.is_none(),
            "editor-focused 'g'/'gg' is QueryEditorComponent's own vim handling, not a list motion"
        );
    }

    #[test]
    fn ctrl_y_exports_curl_even_while_the_results_pane_has_focus() {
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));
        screen.focus = Focus::Results;
        screen.query_editor.insert_at_cursor("select 1");

        // The fake driver's `export_curl` defaults to `None`, so this is
        // just confirming the key is consumed here rather than falling
        // through to the results pane's own key handling.
        let action = screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::CONTROL);

        assert!(action.is_none());
        assert_eq!(screen.query_editor.text(), "select 1");
    }

    #[test]
    fn typing_a_plain_character_in_insert_mode_reaches_the_editor_not_a_command() {
        let (mut screen, _rx) = screen();
        // `y` (yank) and `?` (help) are both bound on this screen -- while
        // typing they must be plain text, or they'd be un-typeable.
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

        for c in "y?".chars() {
            let action = screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
            assert!(action.is_none(), "'{c}' must not raise an action");
        }

        assert_eq!(screen.query_editor.text(), "y?");
    }

    #[test]
    fn a_modified_key_still_works_from_insert_mode() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

        screen.handle_key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);

        assert!(
            screen.prompt.is_some(),
            "ctrl-s can't be confused with typing, so it stays available"
        );
        assert_eq!(screen.query_editor.text(), "");
    }

    #[test]
    fn yank_and_insert_name_only_fire_from_the_pane_they_belong_to() {
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));
        assert_eq!(screen.focus, Focus::Editor);

        // In Normal mode with the editor focused, `y` is the editor's key
        // (starts a `yy`, not the results pane's yank)...
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);
        // ...completed here so the pending `yy` doesn't swallow the `i`
        // below as "any other key cancels a pending y", the same rule
        // `dd`/`za` already follow.
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(screen.query_editor.text(), "");

        // ...and `enter` inserts a newline rather than a schema name.
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "\n");
    }

    #[tokio::test]
    async fn l_moves_the_cell_cursor_when_the_results_pane_has_focus() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
            truncated: false,
        });

        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);

        assert_eq!(screen.results.selected_cell(), Some(("b", "2")));
    }

    #[test]
    fn space_toggles_the_cell_preview_when_the_results_pane_has_focus() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        screen.results.set_result(QueryResult::Table {
            columns: vec!["metadata".to_string()],
            rows: vec![vec![r#"{"theme":"dark"}"#.to_string()]],
            truncated: false,
        });
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();

        screen.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("full value"), "buffer was: {text}");
        assert!(text.contains("\"theme\""), "buffer was: {text}");

        screen.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(!text.contains("full value"), "buffer was: {text}");
    }

    /// A screen whose schema has a table worth completing.
    fn screen_with_completions() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        screen_with(fake_engine_with_schema(empty_result(), Ok(schema())))
    }

    fn type_in_insert(screen: &mut QueryScreenComponent, text: &str) {
        screen.handle_key_event(KeyCode::Char('i'), KeyModifiers::NONE);
        for c in text.chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn typing_offers_matching_suggestions_and_tab_accepts_one() {
        let (mut screen, _rx) = screen_with_completions();

        type_in_insert(&mut screen, "use");

        assert!(screen.completion.is_some(), "'use' should match 'users'");
        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "users");
        assert!(
            screen.completion.is_none(),
            "the word is complete, so there is nothing left to suggest"
        );
    }

    #[test]
    fn tab_cycles_focus_when_no_suggestion_is_showing() {
        let (mut screen, _rx) = screen_with_completions();
        assert!(screen.completion.is_none());

        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);

        assert_eq!(
            screen.focus,
            Focus::Results,
            "tab keeps its usual meaning when there's nothing to accept"
        );
    }

    #[test]
    fn a_word_with_no_matches_shows_no_popup() {
        let (mut screen, _rx) = screen_with_completions();

        type_in_insert(&mut screen, "zzz");

        assert!(screen.completion.is_none());
    }

    #[test]
    fn leaving_insert_mode_dismisses_the_suggestions() {
        let (mut screen, _rx) = screen_with_completions();
        type_in_insert(&mut screen, "use");
        assert!(screen.completion.is_some());

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.completion.is_none());
        assert_eq!(screen.query_editor.mode, EditorMode::Normal);
    }

    #[test]
    fn deleting_back_to_a_shorter_prefix_reopens_the_suggestions() {
        let (mut screen, _rx) = screen_with_completions();
        type_in_insert(&mut screen, "users");
        assert!(
            screen.completion.is_none(),
            "an exact match has nothing left to complete"
        );

        screen.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert!(screen.completion.is_some(), "'user' matches 'users' again");
    }

    #[test]
    fn suggestions_can_be_stepped_through_before_accepting() {
        let (mut screen, _rx) = screen_with_completions();
        // "o" matches the `orders` table plus the driver's OR / ORDER BY.
        type_in_insert(&mut screen, "o");
        let first = screen
            .completion
            .as_ref()
            .and_then(|p| p.selected_text())
            .map(str::to_string);
        assert!(first.is_some());

        screen.handle_key_event(KeyCode::Char('n'), KeyModifiers::CONTROL);
        let second = screen
            .completion
            .as_ref()
            .and_then(|p| p.selected_text())
            .map(str::to_string);

        assert_ne!(first, second, "ctrl-n should move to another suggestion");
    }

    #[tokio::test]
    async fn clicking_the_editor_focuses_it() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        // The editor pane is the top strip of the screen now that the
        // schema tree has moved out to the navigator.
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(screen.focus, Focus::Editor);
    }

    #[test]
    fn f6_toggles_the_split_between_stacked_and_side_by_side() {
        let (mut screen, _rx) = screen();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let stacked_editor_area = screen.editor_area;

        screen.handle_key_event(KeyCode::F(6), KeyModifiers::NONE);
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        assert_eq!(stacked_editor_area.width, 80, "stacked: full width");
        assert!(
            screen.editor_area.width < 80,
            "side by side: editor is now only part of the width"
        );
        assert_eq!(
            screen.editor_area.height, 24,
            "side by side: editor spans the full height"
        );
    }

    #[test]
    fn ctrl_up_grows_the_focused_editor_pane() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Editor;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let before = screen.editor_area.height;

        screen.handle_key_event(KeyCode::Up, KeyModifiers::CONTROL);
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        assert!(
            screen.editor_area.height > before,
            "zooming in on the focused editor should grow it"
        );
    }

    #[test]
    fn ctrl_down_shrinks_the_focused_results_pane_back() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        screen.handle_key_event(KeyCode::Up, KeyModifiers::CONTROL); // grow results first
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let grown = screen.editor_area.height;

        screen.handle_key_event(KeyCode::Down, KeyModifiers::CONTROL);
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        assert!(
            screen.editor_area.height > grown,
            "zooming the focused results pane back out should shrink results, growing editor"
        );
    }

    #[test]
    fn right_clicking_a_result_row_opens_a_context_menu() {
        let (mut screen, _rx) = screen();
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 5,
            row: 20,
            modifiers: KeyModifiers::NONE,
        });

        assert!(screen.context_menu.is_some());
        assert_eq!(screen.focus, Focus::Results);
    }

    #[test]
    fn confirming_a_context_menu_item_runs_the_same_command_a_key_would() {
        let (mut screen, _rx) = screen();
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 5,
            row: 20,
            modifiers: KeyModifiers::NONE,
        });
        assert!(screen.context_menu.is_some());

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.context_menu.is_none(), "the menu closes on confirm");
    }

    #[test]
    fn esc_closes_the_context_menu_without_running_anything() {
        let (mut screen, _rx) = screen();
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 5,
            row: 20,
            modifiers: KeyModifiers::NONE,
        });

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.context_menu.is_none());
        assert!(action.is_none(), "esc must not also trigger BackToPicker");
    }

    #[test]
    fn middle_clicking_the_editor_focuses_it() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Middle),
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(screen.focus, Focus::Editor);
    }

    /// The statement `run-query` would send for the current cursor
    /// position, without needing the engine to actually run it.
    fn statement_at_cursor(screen: &QueryScreenComponent) -> Option<String> {
        let text = screen.query_editor.text();
        let cursor = screen.query_editor.cursor_offset();
        let statements = screen.engine.split_statements(&text);
        statements
            .iter()
            .find(|s| cursor >= s.start && cursor <= s.end)
            .or_else(|| statements.last())
            .map(|s| s.text.clone())
    }

    #[tokio::test]
    async fn run_query_picks_the_statement_the_cursor_is_in() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .set_text("SELECT 1;\nSELECT 2;\nSELECT 3;");

        // Cursor starts on line 1.
        assert_eq!(statement_at_cursor(&screen).as_deref(), Some("SELECT 1"));

        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(statement_at_cursor(&screen).as_deref(), Some("SELECT 2"));

        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(statement_at_cursor(&screen).as_deref(), Some("SELECT 3"));
    }

    #[tokio::test]
    async fn a_statement_spanning_several_lines_is_picked_whole() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .set_text("SELECT id,\n  name\nFROM users;\nSELECT 2;");

        // Cursor on the middle line of the first statement.
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));

        assert_eq!(
            statement_at_cursor(&screen).as_deref(),
            Some("SELECT id,\n  name\nFROM users"),
            "a line break must not cut the statement short"
        );
    }

    #[tokio::test]
    async fn a_cursor_past_the_last_statement_runs_the_last_one() {
        let (mut screen, _rx) = screen();
        // A trailing newline is where the cursor sits after typing.
        screen.query_editor.set_text("SELECT 1;\nSELECT 2;\n");
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));

        assert_eq!(statement_at_cursor(&screen).as_deref(), Some("SELECT 2"));
    }

    #[tokio::test]
    async fn run_all_sends_every_statement_and_records_them_in_history() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_text("SELECT 1;\nSELECT 2;");

        screen.handle_key_event(KeyCode::Char('a'), KeyModifiers::CONTROL);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(screen.engine.history(), &["SELECT 1", "SELECT 2"]);
    }

    #[test]
    fn question_mark_in_normal_mode_asks_for_the_help_overlay() {
        let (mut screen, _rx) = screen();

        let action = screen.handle_key_event(KeyCode::Char('?'), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::ShowHelp)));
    }

    #[test]
    fn ctrl_s_opens_a_save_prompt_prefilled_with_the_current_text() {
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("select 1");

        screen.handle_key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);

        let prompt = screen.prompt.as_ref().expect("prompt should be open");
        assert_eq!(prompt.kind, PromptKind::Save);
        // The editor content stays untouched while the prompt only holds the
        // (empty, on a first save) target path.
        assert_eq!(screen.query_editor.text(), "select 1");
    }

    #[test]
    fn typing_a_path_and_enter_saves_the_editor_text_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("query.sql");
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("select 1");

        screen.handle_key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);
        for c in path.to_str().unwrap().chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(
            screen.prompt.is_none(),
            "a successful save closes the prompt"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "select 1");
        assert_eq!(screen.last_path.as_deref(), path.to_str());
    }

    #[test]
    fn esc_cancels_the_save_prompt_without_touching_the_editor() {
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("select 1");
        screen.handle_key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.prompt.is_none());
        assert_eq!(screen.query_editor.text(), "select 1");
    }

    #[test]
    fn confirming_an_empty_path_keeps_the_prompt_open_with_an_error() {
        let (mut screen, _rx) = screen();
        screen.handle_key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let prompt = screen.prompt.as_ref().expect("prompt stays open on error");
        assert!(prompt.error.is_some());
    }

    /// The picker as `Ctrl+O` builds it, minus the process-global queries
    /// directory -- which tests must not initialise, since a `OnceLock` set
    /// by one test would leak into every other.
    fn open_picker_on(screen: &mut QueryScreenComponent, dir: &std::path::Path) {
        screen.picker = Some(FilePickerComponent::new(&[], dir, dir));
    }

    #[test]
    fn choosing_a_file_in_the_picker_loads_it_and_closes_the_overlay() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.sql"), "select * from users").unwrap();
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("stale content");
        open_picker_on(&mut screen, dir.path());

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.picker.is_none());
        assert_eq!(screen.query_editor.text(), "select * from users");
        assert_eq!(
            screen.last_path.as_deref(),
            dir.path().join("a.sql").to_str()
        );
    }

    #[test]
    fn a_picked_file_that_cannot_be_read_leaves_the_overlay_up() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, _rx) = screen();
        open_picker_on(&mut screen, dir.path());
        // Nothing in the directory, so the typed text is opened as a path.
        for c in "/does/not/exist.sql".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(
            screen.picker.is_some(),
            "closing it would leave no sign the open failed"
        );
    }

    #[test]
    fn esc_closes_the_picker_without_touching_the_editor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.sql"), "select 1").unwrap();
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("draft");
        open_picker_on(&mut screen, dir.path());

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.picker.is_none());
        assert_eq!(screen.query_editor.text(), "draft");
    }

    #[test]
    fn keys_do_not_reach_the_editor_while_the_picker_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("draft");
        open_picker_on(&mut screen, dir.path());

        screen.handle_key_event(KeyCode::Char('x'), KeyModifiers::NONE);

        assert_eq!(
            screen.query_editor.text(),
            "draft",
            "'x' filters, not edits"
        );
    }

    #[test]
    fn ctrl_o_loads_a_file_into_the_editor() {
        // Without a queries directory configured (as in tests) `Ctrl+O`
        // falls back to asking for a path outright.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("query.sql");
        std::fs::write(&path, "select * from users").unwrap();
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("stale content");

        screen.handle_key_event(KeyCode::Char('o'), KeyModifiers::CONTROL);
        for c in path.to_str().unwrap().chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.prompt.is_none());
        assert_eq!(screen.query_editor.text(), "select * from users");
    }

    #[test]
    fn opening_a_missing_file_keeps_the_prompt_open_with_an_error() {
        let (mut screen, _rx) = screen();
        screen.handle_key_event(KeyCode::Char('o'), KeyModifiers::CONTROL);
        for c in "/does/not/exist.sql".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let prompt = screen.prompt.as_ref().expect("prompt stays open on error");
        assert!(prompt.error.is_some());
    }

    async fn submit_and_settle(screen: &mut QueryScreenComponent, query: &str) {
        screen.query_editor.set_text(query);
        screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }
    }

    #[tokio::test]
    async fn column_types_resolves_the_result_s_columns_against_the_source_table() {
        let result = QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![vec!["1".to_string(), "Ada".to_string()]],
            truncated: false,
        };
        let schema = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![
                crate::query_driver::ColumnInfo {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    primary_key: true,
                },
                crate::query_driver::ColumnInfo::new("name", "TEXT"),
            ],
            kind: None,
            ttl: None,
        }];
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(result, Ok(schema)));
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;

        assert_eq!(
            screen.column_types(),
            vec![Some("INTEGER".to_string()), Some("TEXT".to_string())]
        );
    }

    #[tokio::test]
    async fn column_types_is_all_none_for_a_join() {
        let result = QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        };
        let schema = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![crate::query_driver::ColumnInfo::new("id", "INTEGER")],
            kind: None,
            ttl: None,
        }];
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(result, Ok(schema)));
        submit_and_settle(
            &mut screen,
            "SELECT id FROM users JOIN orders ON orders.user_id = users.id",
        )
        .await;

        assert_eq!(screen.column_types(), vec![None]);
    }

    /// Always fails, counting how many times it was actually asked to run
    /// -- what tells `r` (retry) apart from a no-op.
    struct CountingFailingDriver {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl QueryDriver for CountingFailingDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(anyhow::anyhow!("syntax error"))
        }
    }

    #[tokio::test]
    async fn r_retries_the_exact_query_that_failed() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let engine = QueryEngine::new(
            Arc::new(CountingFailingDriver {
                calls: calls.clone(),
            }),
            connection(),
            Ok(Vec::new()),
        );
        let (mut screen, _rx) = screen_with(engine);
        submit_and_settle(&mut screen, "SELECT 1").await;
        assert!(screen.results.last_error.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "r must resubmit the failed query"
        );
    }

    #[tokio::test]
    async fn r_is_a_no_op_without_an_error_showing() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        assert!(!screen.engine.is_pending(), "nothing to retry, nothing ran");
    }

    #[tokio::test]
    async fn e_moves_focus_to_the_editor_only_when_an_error_is_showing() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Results;
        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::NONE);
        assert_eq!(
            screen.focus,
            Focus::Results,
            "e does nothing without an error to fix"
        );

        let engine = QueryEngine::new(
            Arc::new(CountingFailingDriver {
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }),
            connection(),
            Ok(Vec::new()),
        );
        let (mut screen, _rx) = screen_with(engine);
        submit_and_settle(&mut screen, "SELECT 1").await;
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::NONE);

        assert_eq!(screen.focus, Focus::Editor);
    }

    #[tokio::test]
    async fn ctrl_r_opens_history_with_the_most_recent_query_selected() {
        let (mut screen, _rx) = screen();
        submit_and_settle(&mut screen, "select 1").await;
        submit_and_settle(&mut screen, "select 2").await;

        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);

        let picker = screen.history_picker.as_ref().expect("history should open");
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[tokio::test]
    async fn ctrl_r_on_empty_history_is_a_no_op() {
        let (mut screen, _rx) = screen();

        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);

        assert!(screen.history_picker.is_none());
    }

    #[tokio::test]
    async fn enter_on_a_history_entry_loads_it_into_the_editor() {
        let (mut screen, _rx) = screen();
        submit_and_settle(&mut screen, "select 1").await;
        submit_and_settle(&mut screen, "select 2").await;
        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.history_picker.is_none());
        assert_eq!(screen.query_editor.text(), "select 1");
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[tokio::test]
    async fn esc_cancels_history_without_touching_the_editor() {
        let (mut screen, _rx) = screen();
        submit_and_settle(&mut screen, "select 1").await;
        screen.query_editor.set_text("unsaved edit");
        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.history_picker.is_none());
        assert_eq!(screen.query_editor.text(), "unsaved edit");
    }

    #[test]
    fn ctrl_k_opens_the_snippet_name_prompt() {
        let (mut screen, _rx) = screen();

        screen.handle_key_event(KeyCode::Char('k'), KeyModifiers::CONTROL);

        assert!(screen.snippet_prompt.is_some());
    }

    #[test]
    fn esc_cancels_the_snippet_name_prompt() {
        let (mut screen, _rx) = screen();
        screen.handle_key_event(KeyCode::Char('k'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.snippet_prompt.is_none());
    }

    #[test]
    fn enter_on_the_snippet_name_prompt_closes_it() {
        // No global `Snippets` store is initialized in tests (same
        // limitation `query_files()` has) -- confirming a name with
        // nowhere to persist it must still close the prompt cleanly, not
        // panic or leave it stuck open.
        let (mut screen, _rx) = screen();
        screen.handle_key_event(KeyCode::Char('k'), KeyModifiers::CONTROL);

        for c in "my-query".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.snippet_prompt.is_none());
    }

    #[test]
    fn ctrl_l_opens_the_snippet_picker() {
        let (mut screen, _rx) = screen();

        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::CONTROL);

        assert!(screen.snippet_picker.is_some());
    }

    #[test]
    fn esc_cancels_the_snippet_picker() {
        let (mut screen, _rx) = screen();
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::CONTROL);

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.snippet_picker.is_none());
    }

    #[test]
    fn clicking_while_the_snippet_picker_is_open_does_not_leak_through_to_the_results_pane() {
        let (mut screen, _rx) = screen();
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });
        screen.focus = Focus::Editor;
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert!(screen.snippet_picker.is_some());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        // Inside the results pane -- if the click leaked through, this
        // would switch focus there.
        screen.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 20,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(
            screen.focus,
            Focus::Editor,
            "a click behind the open snippet picker must not reach the results pane"
        );
    }

    #[test]
    fn choosing_a_snippet_loads_it_into_the_editor() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_text("unrelated");
        // Constructed directly rather than via `Ctrl+L`, since that reads
        // the process-global store (empty in tests) -- this exercises the
        // `SnippetOutcome::Insert` wiring on its own.
        screen.snippet_picker = Some(
            crate::components::snippet_picker::SnippetPickerComponent::new(
                "sqlite",
                vec![tradar_core::storage::SavedSnippet {
                    name: "active-users".to_string(),
                    driver: "sqlite".to_string(),
                    text: "SELECT * FROM users WHERE active;".to_string(),
                }],
            ),
        );

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.snippet_picker.is_none());
        assert_eq!(
            screen.query_editor.text(),
            "SELECT * FROM users WHERE active;"
        );
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[test]
    fn draw_shows_active_connection_and_input() {
        let (mut screen, _rx) = screen();
        screen.query_editor.insert_at_cursor("x");
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }

    #[test]
    fn esc_returns_back_to_picker() {
        let (mut screen, _rx) = screen();

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    #[test]
    fn esc_in_insert_mode_returns_to_normal_mode_instead_of_the_picker() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert_eq!(screen.query_editor.mode, EditorMode::Insert);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(
            action.is_none(),
            "Esc must be consumed by the editor, not bubble to BackToPicker"
        );
        assert_eq!(screen.query_editor.mode, EditorMode::Normal);
    }

    #[test]
    fn esc_in_normal_mode_returns_back_to_picker_even_after_leaving_insert_mode() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(screen.query_editor.mode, EditorMode::Normal);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    #[test]
    fn with_vim_mode_off_esc_goes_straight_back_to_the_picker() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_vim_enabled(false);
        assert_eq!(screen.focus, Focus::Editor);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(
            matches!(action, Some(Action::BackToPicker)),
            "there's no Normal mode to leave first, so Esc must not be swallowed by the editor"
        );
    }

    #[test]
    fn with_vim_mode_off_typing_works_without_ever_pressing_i() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_vim_enabled(false);

        screen.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('b'), KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "ab");
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_j_redoes_regardless_of_vim_mode() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_vim_enabled(false);
        screen.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(screen.query_editor.text(), "a");

        screen.handle_key_event(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(
            screen.query_editor.text(),
            "",
            "ctrl-z must undo the typed edit"
        );

        screen.handle_key_event(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(screen.query_editor.text(), "a", "ctrl-j must redo it");
    }

    fn screen_showing_cities() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let (mut screen, rx) = screen();
        screen.focus = Focus::Results;
        screen.results.set_result(QueryResult::Table {
            columns: vec!["id".to_string(), "city".to_string()],
            rows: vec![
                vec!["1".to_string(), "Hanoi".to_string()],
                vec!["2".to_string(), "Da Nang".to_string()],
            ],
            truncated: false,
        });
        (screen, rx)
    }

    #[test]
    fn slash_filters_the_results_as_you_type() {
        let (mut screen, _rx) = screen_showing_cities();

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "nang".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert_eq!(screen.results.filter(), "nang");
        assert_eq!(
            screen.results.selected_row().map(|r| r[1].clone()),
            Some("Da Nang".to_string())
        );
    }

    #[test]
    fn enter_keeps_the_filter_and_esc_undoes_it() {
        let (mut screen, _rx) = screen_showing_cities();
        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('h'), KeyModifiers::NONE);

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        assert!(screen.search.is_none(), "the bar closes");
        assert_eq!(screen.results.filter(), "h", "but the filter stays");

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(
            screen.results.filter(),
            "",
            "esc puts the grid back the way it was"
        );
    }

    #[test]
    fn reopening_the_filter_starts_from_what_is_already_applied() {
        let (mut screen, _rx) = screen_showing_cities();
        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('h'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);

        assert_eq!(
            screen.results.filter(),
            "ha",
            "the second `/` refines rather than starting over"
        );
    }

    #[test]
    fn a_key_bound_elsewhere_is_plain_text_while_the_filter_bar_is_open() {
        let (mut screen, _rx) = screen_showing_cities();

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        // `d` deletes a row and `y` yanks one when Results has focus; while
        // typing a filter they have to be letters.
        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(screen.results.filter(), "dy");
        assert!(screen.row_edit.is_none(), "no delete may have started");
    }

    #[test]
    fn slash_opens_buffer_search_and_previews_matches_incrementally() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_text("select * from users");

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "from".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert!(screen.buffer_search.is_some());
        assert!(
            screen.search.is_none(),
            "the editor's own search must not touch the results filter"
        );
        assert_eq!(screen.query_editor.cursor(), (0, 9));
    }

    #[test]
    fn esc_cancels_buffer_search_and_restores_the_original_cursor() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_text("select * from users");
        screen.query_editor.set_cursor(0, 2);

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "from".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(screen.query_editor.cursor(), (0, 9), "preview jumped");

        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.buffer_search.is_none());
        assert_eq!(
            screen.query_editor.cursor(),
            (0, 2),
            "esc undoes the whole search, cursor included"
        );
    }

    #[test]
    fn enter_confirms_a_buffer_search_and_n_repeats_it_wrapping_around() {
        let (mut screen, _rx) = screen();
        screen.query_editor.set_text("aaa bbb aaa");

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "aaa".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.buffer_search.is_none(), "the bar closes");
        assert_eq!(
            screen.query_editor.cursor(),
            (0, 8),
            "confirming keeps the cursor on the match, the second \"aaa\""
        );

        screen.handle_key_event(KeyCode::Char('n'), KeyModifiers::NONE);

        assert_eq!(
            screen.query_editor.cursor(),
            (0, 0),
            "n wraps back around to the first occurrence"
        );
    }

    #[test]
    fn slash_while_focus_is_on_results_still_filters_the_grid_not_the_buffer() {
        let (mut screen, _rx) = screen_showing_cities();

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);

        assert!(
            screen.search.is_some(),
            "Results focus keeps the existing filter-bar behavior"
        );
        assert!(screen.buffer_search.is_none());
    }

    #[test]
    fn slash_in_insert_mode_types_a_literal_character_instead_of_opening_search() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

        screen.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);

        assert!(screen.buffer_search.is_none());
        assert_eq!(screen.query_editor.text(), "/");
    }

    /// A screen whose driver returns a two-column `users` table and whose
    /// schema says `id` is the primary key -- the setup every row edit
    /// needs.
    fn editable_screen() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let result = QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec!["1".to_string(), "Ada".to_string()],
                vec!["2".to_string(), "Lin".to_string()],
            ],
            truncated: false,
        };
        let schema = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![
                crate::query_driver::ColumnInfo {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    primary_key: true,
                },
                crate::query_driver::ColumnInfo::new("name", "TEXT"),
            ],
            kind: None,
            ttl: None,
        }];
        screen_with(fake_engine_with_schema(result, Ok(schema)))
    }

    #[tokio::test]
    async fn editing_a_cell_generates_an_update_keyed_on_the_primary_key() {
        let (mut screen, _rx) = editable_screen();
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;
        screen.focus = Focus::Results;
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('!'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert!(screen.row_edit.is_none(), "the overlay closes once it runs");
        assert_eq!(
            screen.engine.history().last().map(String::as_str),
            Some("SELECT id, name FROM users"),
            "the grid is re-read after the edit, so you see what changed"
        );
        assert_eq!(
            screen.engine.history()[1],
            "UPDATE \"users\" SET \"name\" = 'Ada!' WHERE \"id\" = '1'"
        );
    }

    #[tokio::test]
    async fn nothing_runs_until_the_statement_is_approved() {
        let (mut screen, _rx) = editable_screen();
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(screen.row_edit.is_some(), "a delete has to be confirmed");
        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(screen.row_edit.is_none());
        assert_eq!(
            screen.engine.history(),
            &["SELECT id, name FROM users"],
            "a cancelled delete must not have run anything"
        );
    }

    #[tokio::test]
    async fn deleting_a_row_generates_a_delete_for_the_selected_row() {
        let (mut screen, _rx) = editable_screen();
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;
        screen.focus = Focus::Results;
        screen.results.move_down();

        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(
            screen.engine.history()[1],
            "DELETE FROM \"users\" WHERE \"id\" = '2'"
        );
    }

    #[tokio::test]
    async fn a_result_that_is_not_one_table_refuses_the_edit_and_says_why() {
        let (mut screen, _rx) = editable_screen();
        submit_and_settle(
            &mut screen,
            "SELECT id, name FROM users JOIN orders ON orders.user_id = users.id",
        )
        .await;
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("single table"),
            "the refusal must explain itself: {text}"
        );
        assert_eq!(
            screen.engine.history().len(),
            1,
            "nothing may have been run"
        );
    }

    #[tokio::test]
    async fn a_table_with_no_primary_key_cannot_be_edited() {
        let result = QueryResult::Table {
            columns: vec!["name".to_string()],
            rows: vec![vec!["Ada".to_string()]],
            truncated: false,
        };
        let schema = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![crate::query_driver::ColumnInfo::new("name", "TEXT")],
            kind: None,
            ttl: None,
        }];
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(result, Ok(schema)));
        submit_and_settle(&mut screen, "SELECT name FROM users").await;
        screen.focus = Focus::Results;

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        // Any key dismisses the explanation; none of them may run anything.
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(screen.engine.history().len(), 1);
    }

    #[tokio::test]
    async fn a_refresh_after_an_edit_leaves_the_cell_cursor_where_it_was() {
        let (mut screen, _rx) = editable_screen();
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;
        screen.focus = Focus::Results;
        screen.results.move_down();
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);

        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(
            (screen.results.selected, screen.results.selected_col),
            (1, 1),
            "a re-read is not a new result: the cursor must not jump home"
        );
    }

    fn redis_connection() -> SavedConnection {
        SavedConnection {
            name: "local-redis".to_string(),
            driver: "redis".to_string(),
            target: "redis://127.0.0.1".to_string(),
        }
    }

    fn redis_schema() -> Vec<SchemaInfo> {
        vec![SchemaInfo {
            name: "user:1".to_string(),
            columns: Vec::new(),
            kind: Some("hash".to_string()),
            ttl: None,
        }]
    }

    fn redis_screen_with(
        result: QueryResult,
        schema: Result<Vec<SchemaInfo>, String>,
    ) -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let engine = QueryEngine::new(Arc::new(FakeDriver { result }), redis_connection(), schema);
        screen_with(engine)
    }

    #[test]
    fn a_redis_connection_starts_in_browse_mode_focused_on_the_sidebar() {
        let (screen, _rx) = redis_screen_with(empty_result(), Ok(redis_schema()));

        assert_eq!(screen.mode, ScreenMode::Browse);
        assert_eq!(screen.focus, Focus::Browse);
        assert!(screen.browse.is_some());
    }

    #[test]
    fn a_non_redis_connection_never_gets_a_browse_sidebar() {
        let (screen, _rx) = screen();

        assert_eq!(screen.mode, ScreenMode::Console);
        assert_eq!(screen.focus, Focus::Editor);
        assert!(screen.browse.is_none());
    }

    #[test]
    fn f2_is_a_no_op_without_a_browse_sidebar() {
        let (mut screen, _rx) = screen();

        screen.handle_key_event(KeyCode::F(2), KeyModifiers::NONE);

        assert_eq!(screen.mode, ScreenMode::Console);
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[test]
    fn f2_toggles_a_redis_screen_between_browse_and_console() {
        let (mut screen, _rx) = redis_screen_with(empty_result(), Ok(redis_schema()));

        screen.handle_key_event(KeyCode::F(2), KeyModifiers::NONE);
        assert_eq!(screen.mode, ScreenMode::Console);
        assert_eq!(screen.focus, Focus::Editor);

        screen.handle_key_event(KeyCode::F(2), KeyModifiers::NONE);
        assert_eq!(screen.mode, ScreenMode::Browse);
        assert_eq!(screen.focus, Focus::Browse);
    }

    #[tokio::test]
    async fn enter_on_the_sidebar_fetches_the_key_and_moves_focus_to_results() {
        let fetched = QueryResult::Table {
            columns: vec!["field".to_string(), "value".to_string()],
            rows: vec![vec!["name".to_string(), "Ada".to_string()]],
            truncated: false,
        };
        let (mut screen, _rx) = redis_screen_with(fetched.clone(), Ok(redis_schema()));

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(screen.focus, Focus::Results);
        assert_eq!(screen.results.selected_row(), Some(&fetched_row(&fetched)));
    }

    fn fetched_row(result: &QueryResult) -> Vec<String> {
        match result {
            QueryResult::Table { rows, .. } => rows[0].clone(),
            other => panic!("expected a Table, got {other:?}"),
        }
    }

    #[test]
    fn clicking_a_key_in_the_sidebar_selects_it_and_focuses_the_pane() {
        let (mut screen, _rx) = redis_screen_with(empty_result(), Ok(redis_schema()));
        screen.focus = Focus::Results;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();

        // Row 0 is the sidebar's border, row 1 the one key ("user:1").
        screen.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(screen.focus, Focus::Browse);
        assert!(
            screen.last_browse_command.is_none(),
            "a single click must only select, not fetch"
        );
    }

    #[tokio::test]
    async fn double_clicking_a_key_in_the_sidebar_fetches_it_same_as_enter() {
        let fetched = QueryResult::Table {
            columns: vec!["field".to_string(), "value".to_string()],
            rows: vec![vec!["name".to_string(), "Ada".to_string()]],
            truncated: false,
        };
        let (mut screen, _rx) = redis_screen_with(fetched.clone(), Ok(redis_schema()));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };

        screen.handle_mouse_event(click);
        screen.handle_mouse_event(click);
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
            screen.tick();
            if !screen.engine.is_pending() {
                break;
            }
        }

        assert_eq!(screen.focus, Focus::Results);
        assert_eq!(screen.results.selected_row(), Some(&fetched_row(&fetched)));
    }

    #[tokio::test]
    async fn enter_on_the_sidebar_echoes_the_command_it_ran() {
        let (mut screen, _rx) = redis_screen_with(empty_result(), Ok(redis_schema()));

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            screen.last_browse_command.as_deref(),
            Some("HGETALL user:1")
        );

        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("HGETALL user:1"),
            "expected the echoed command in the drawn frame: {text}"
        );
    }

    #[test]
    fn insert_text_switches_a_redis_screen_back_to_console_mode() {
        let (mut screen, _rx) = redis_screen_with(empty_result(), Ok(redis_schema()));

        screen.insert_text("user:1");

        assert_eq!(screen.mode, ScreenMode::Console);
        assert_eq!(screen.focus, Focus::Editor);
    }

    fn table_result() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![vec!["1".to_string(), "Ada".to_string()]],
            truncated: false,
        }
    }

    fn type_path(screen: &mut QueryScreenComponent, path: &std::path::Path) {
        for c in path.to_str().unwrap().chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[tokio::test]
    async fn ctrl_e_exports_the_result_as_csv_when_the_path_ends_in_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let (mut screen, _rx) = screen_with(fake_engine(table_result()));
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::CONTROL);
        type_path(&mut screen, &path);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(
            screen.prompt.is_none(),
            "a successful export closes the prompt"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "id,name\n1,Ada\n");
    }

    #[tokio::test]
    async fn ctrl_e_exports_the_result_as_json_when_the_path_ends_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let (mut screen, _rx) = screen_with(fake_engine(table_result()));
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::CONTROL);
        type_path(&mut screen, &path);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.prompt.is_none());
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed, serde_json::json!([{"id": "1", "name": "Ada"}]));
    }

    #[tokio::test]
    async fn ctrl_e_with_an_unrecognized_extension_keeps_the_prompt_open_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        let (mut screen, _rx) = screen_with(fake_engine(table_result()));
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::CONTROL);
        type_path(&mut screen, &path);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let prompt = screen.prompt.as_ref().expect("prompt stays open on error");
        assert!(prompt.error.as_deref().unwrap().contains(".csv"));
        assert!(!path.exists());
    }

    #[test]
    fn ctrl_e_with_no_result_keeps_the_prompt_open_with_an_error() {
        let (mut screen, _rx) = screen();

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::CONTROL);
        for c in "out.csv".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let prompt = screen.prompt.as_ref().expect("prompt stays open on error");
        assert_eq!(prompt.error.as_deref(), Some("nothing to export"));
    }

    #[tokio::test]
    async fn a_successful_export_does_not_touch_the_recent_query_files_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let (mut screen, _rx) = screen_with(fake_engine(table_result()));
        submit_and_settle(&mut screen, "SELECT id, name FROM users").await;

        screen.handle_key_event(KeyCode::Char('e'), KeyModifiers::CONTROL);
        type_path(&mut screen, &path);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            screen.last_path, None,
            "export must not be mistaken for a saved/opened query file"
        );
    }
}
