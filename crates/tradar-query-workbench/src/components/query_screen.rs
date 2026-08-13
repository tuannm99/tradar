//! The post-connect screen: schema sidebar + query editor + results,
//! composed. Implements `Component` because `RootComponent` routes keys and
//! ticks to it directly whenever it's the active screen. Owns the
//! `QueryEngine` directly (not through `dyn Session`) since this screen only
//! ever exists for a query-shaped connector's own engine.

use std::io::Write;

use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use tradar_connector_api::Session;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedConnection;
use tradar_core::ui;
use tradar_core::vim_list::VimMove;

use crate::components::completion::{CompletionPopup, CompletionSource};
use crate::components::file_picker::{FilePickerComponent, PickerOutcome};
use crate::components::file_prompt::{FilePromptComponent, PromptKind, PromptOutcome};
use crate::components::history_picker::{HistoryOutcome, HistoryPickerComponent};
use crate::components::query_editor::{Dialect, EditorMode, QueryEditorComponent};
use crate::components::results::ResultsComponent;
use crate::components::schema_sidebar::SchemaSidebarComponent;
use crate::query_engine::{QueryEngine, QueryOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
    Sidebar,
}

pub struct QueryScreenComponent {
    pub focus: Focus,
    pub schema_sidebar: SchemaSidebarComponent,
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
    /// Where the editor was last drawn, so a click there can focus it.
    editor_area: Rect,
    /// Everything completable for this connection, built once on connect.
    completions: CompletionSource,
    /// The suggestion list, present only while there is something to
    /// suggest -- so "no popup" and "no matches" are one state.
    completion: Option<CompletionPopup>,
}

/// Copies `text` to the system clipboard via an OSC52 escape sequence,
/// which the terminal emulator itself intercepts -- no clipboard crate
/// needed, and it works through SSH/tmux as long as the terminal supports
/// OSC52 (most modern ones do: iTerm2, kitty, Alacritty, WezTerm, Windows
/// Terminal, ...).
fn yank_to_clipboard(text: &str) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let sequence = format!("\x1b]52;c;{encoded}\x07");
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(sequence.as_bytes());
    let _ = stdout.flush();
}

impl QueryScreenComponent {
    /// `_action_tx` is part of `Session::build_screen`'s contract (a screen
    /// backed by a firehose-shaped connector may need to push an `Action`
    /// proactively, outside a key press) but this screen has no use for it
    /// yet -- every state change here already goes through `tick()` or a
    /// direct key-driven method call.
    pub fn new(mut engine: QueryEngine, _action_tx: UnboundedSender<Action>) -> Self {
        let mut schema_sidebar = SchemaSidebarComponent::new();
        match engine.schema().clone() {
            Ok(schema) => schema_sidebar.set_schema(schema),
            Err(e) => schema_sidebar.set_schema_error(e),
        }
        // `tick()` may already have a query outcome queued up (not possible
        // right after `Connector::connect`, but keeps `engine` in a
        // consistent state regardless of how it was constructed).
        engine.tick();

        let completions =
            CompletionSource::new(engine.keywords(), engine.schema().as_deref().unwrap_or(&[]));

        let mut query_editor = QueryEditorComponent::new();
        // Only Postgres/SQLite speak real SQL -- Mongo/Elasticsearch/Redis
        // use their own hand-rolled query shapes with no tree-sitter
        // grammar to match, so they stay plain text.
        if matches!(engine.connection().driver.as_str(), "postgres" | "sqlite") {
            query_editor.set_dialect(Dialect::Sql);
        }

        Self {
            focus: Focus::Editor,
            schema_sidebar,
            query_editor,
            results: ResultsComponent::new(),
            engine,
            pending: None,
            prompt: None,
            picker: None,
            last_path: None,
            history_picker: None,
            editor_area: Rect::ZERO,
            completions,
            completion: None,
        }
    }

    pub fn active_connection(&self) -> &SavedConnection {
        self.engine.connection()
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

    /// What the save/open prompt starts with: the file you last worked on,
    /// shown by bare name when it's in the queries directory. The full path
    /// would be technically the same target but fills the prompt with noise
    /// you then have to delete to save under another name.
    fn prompt_prefill(&self) -> String {
        let Some(last) = self.last_path.as_deref() else {
            return String::new();
        };
        tradar_core::storage::query_files()
            .and_then(|files| {
                std::path::Path::new(last)
                    .strip_prefix(files.dir())
                    .ok()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| last.to_string())
    }

    /// Opens the file picker, or falls back to a typed path when there's
    /// no queries directory configured (tests, mainly).
    fn open_file_picker(&mut self) {
        match tradar_core::storage::query_files() {
            Some(files) => {
                self.picker = Some(FilePickerComponent::new(&files.recent(), files.dir()));
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
            self.engine.submit_query(statement.text.clone());
        }
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
            self.engine.submit_all(statements);
        }
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
        if self.schema_sidebar.contains(event.column, event.row) {
            self.schema_sidebar.apply_move(mv);
        } else if ui::contains(self.editor_area, event.column, event.row) {
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
        // is used as typed.
        let path = match tradar_core::storage::query_files() {
            Some(files) => tradar_core::storage::resolve_query_path(&trimmed, files.dir()),
            None => std::path::PathBuf::from(&trimmed),
        };
        let result = match kind {
            PromptKind::Save => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, self.query_editor.text()).map_err(|e| e.to_string())
            }
            PromptKind::Open => self.open_query_file(&path),
        };
        match result {
            Ok(()) => {
                self.remember(&path);
                self.prompt = None;
            }
            Err(e) => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.error = Some(e);
                }
            }
        }
    }
}

impl Component for QueryScreenComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(prompt) = self.prompt.as_mut() {
            match prompt.handle_key_event(code, modifiers) {
                Some(PromptOutcome::Cancelled) => self.prompt = None,
                Some(PromptOutcome::Confirmed(path)) => self.handle_prompt_confirmed(path),
                None => {}
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
            match history_picker.handle_key_event(code, modifiers) {
                Some(HistoryOutcome::Cancelled) => self.history_picker = None,
                Some(HistoryOutcome::Selected(query)) => {
                    self.query_editor.set_text(&query);
                    self.focus = Focus::Editor;
                    self.history_picker = None;
                }
                None => {}
            }
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
        // (Insert), and only means "back to the picker" from Normal mode.
        // Checked before the keymap so a user rebinding `back` can't
        // accidentally trap themselves in Insert mode.
        if code == KeyCode::Esc
            && self.focus == Focus::Editor
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

        // Only the focused pane's context is offered, which is what lets
        // `l` mean "expand this table" in the sidebar and "scroll right" in
        // the results without the two bindings colliding. With the editor
        // focused, neither pane's keys (nor list navigation) apply -- those
        // are the editor's own vim motions.
        let contexts: &[Context] = match self.focus {
            Focus::Editor => &[Context::QueryScreen],
            Focus::Sidebar => &[Context::QueryScreen, Context::Sidebar, Context::List],
            Focus::Results => &[Context::QueryScreen, Context::Results, Context::List],
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
                Focus::Sidebar => self.schema_sidebar.apply_move(mv),
                Focus::Results => self.results.apply_move(mv),
                Focus::Editor => {}
            }
            return None;
        }

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
            Command::CycleFocus => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Results,
                    Focus::Results => Focus::Sidebar,
                    Focus::Sidebar => Focus::Editor,
                };
            }
            Command::SaveFile => self.open_prompt(PromptKind::Save),
            Command::OpenFile => self.open_file_picker(),
            Command::History => self.open_history(),
            Command::ExportCurl => self.export_curl(),
            Command::Yank => {
                if let Some(text) = self.results.selected_text() {
                    yank_to_clipboard(&text);
                }
            }
            Command::ScrollLeft => self.results.scroll_left(),
            Command::ScrollRight => self.results.scroll_right(),
            Command::Expand => self.schema_sidebar.expand(),
            Command::Collapse => self.schema_sidebar.collapse(),
            Command::InsertName => {
                if let Some(name) = self.schema_sidebar.selected_name() {
                    let name = name.to_string();
                    self.query_editor.insert_at_cursor(&name);
                    self.focus = Focus::Editor;
                }
            }
            _ => {}
        }
        None
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        // An overlay covers the screen, so a click behind it would act on
        // something the user can't even see.
        if self.prompt.is_some() || self.history_picker.is_some() || self.picker.is_some() {
            return None;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Clicking a pane focuses it, which is the main thing a
                // mouse is for here -- and then selects the row hit.
                if self.schema_sidebar.click(event.column, event.row) {
                    self.focus = Focus::Sidebar;
                } else if self.results.click(event.column, event.row) {
                    self.focus = Focus::Results;
                } else if ui::contains(self.editor_area, event.column, event.row) {
                    self.focus = Focus::Editor;
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

    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }

    fn tick(&mut self) -> bool {
        let outcome_arrived = self.engine.tick();
        if outcome_arrived {
            match self.engine.take_outcome() {
                Some(QueryOutcome::Completed { result }) => self.results.set_result(result),
                Some(QueryOutcome::Failed { error }) => self.results.set_error(error),
                None => {}
            }
        }
        outcome_arrived
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        // Sidebar wide enough for a typical table name, but never more than
        // a third of a narrow terminal.
        let sidebar_width = 26.min(area.width / 3);
        let outer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(1)])
            .split(area);

        self.schema_sidebar
            .draw(frame, outer[0], self.focus == Focus::Sidebar);

        // The editor gets a third of the height (min 5 rows, so a short
        // query still has room), results take the rest.
        let editor_height = (area.height / 3).clamp(5, 12);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(editor_height), Constraint::Min(3)])
            .split(outer[1]);

        let connection_name = self.active_connection().name.clone();
        self.editor_area = chunks[0];
        self.query_editor.draw(
            frame,
            chunks[0],
            &connection_name,
            self.focus == Focus::Editor,
        );
        self.results.draw_running(self.engine.is_pending());
        self.results
            .draw(frame, chunks[1], self.focus == Focus::Results);

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
            prompt.draw(frame, popup);
        }

        if let Some(history_picker) = &mut self.history_picker {
            let popup = ui::centered_rect(70, 60, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            history_picker.draw(frame, popup);
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
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
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
        assert_eq!(screen.schema_sidebar.schema, schema());
    }

    #[test]
    fn a_schema_error_is_shown_in_the_sidebar() {
        let (screen, _rx) = screen_with(fake_engine_with_schema(
            empty_result(),
            Err("scan failed".to_string()),
        ));

        assert_eq!(
            screen.schema_sidebar.schema_error.as_deref(),
            Some("scan failed")
        );
    }

    #[test]
    fn tab_cycles_editor_results_sidebar() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(screen.focus, Focus::Results);

        screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(screen.focus, Focus::Sidebar);

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
    fn enter_on_the_sidebar_inserts_the_selected_name_at_the_cursor() {
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));
        screen.schema_sidebar.move_down();
        // Type "ab" then leave Insert mode -- vim leaves the cursor sitting
        // on the last-typed character ("b"), so the inserted name must land
        // between "a" and "b", not appended at the buffer's end.
        type_chars(&mut screen, "iab");
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        screen.focus = Focus::Sidebar;

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "aordersb");
        assert_eq!(screen.focus, Focus::Editor);
        assert_eq!(screen.query_editor.mode, EditorMode::Insert);
    }

    #[test]
    fn enter_on_the_sidebar_is_a_no_op_when_schema_is_empty() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Sidebar;

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "");
        assert_eq!(
            screen.focus,
            Focus::Sidebar,
            "no-op must not change focus either"
        );
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

    fn sidebar_focused_screen_with_schema() -> QueryScreenComponent {
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(empty_result(), Ok(schema())));
        screen.focus = Focus::Sidebar;
        screen
    }

    #[tokio::test]
    async fn ctrl_enter_submits_instead_of_inserting_the_schema_selection_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.insert_at_cursor("x");

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::CONTROL);

        assert_eq!(screen.query_editor.text(), "x");
    }

    #[tokio::test]
    async fn f5_submits_instead_of_being_swallowed_by_the_sidebar_guard() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.insert_at_cursor("x");

        screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "x");
    }

    #[test]
    fn gg_moves_the_schema_selection_to_the_top_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.schema_sidebar.move_down();
        assert_eq!(screen.schema_sidebar.schema_selected, 1);

        let first = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(first.is_none(), "a lone 'g' should not act yet");
        assert_eq!(screen.schema_sidebar.schema_selected, 1);

        screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(screen.schema_sidebar.schema_selected, 0);
    }

    #[test]
    fn shift_g_moves_the_schema_selection_to_the_bottom_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        screen.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);

        assert_eq!(screen.schema_sidebar.schema_selected, 1);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_scroll_the_schema_sidebar_when_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(screen.schema_sidebar.schema_selected, 1);

        screen.handle_key_event(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(screen.schema_sidebar.schema_selected, 0);
    }

    #[test]
    fn g_is_forwarded_to_the_editor_instead_of_the_sidebar_when_editor_focused() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        let first = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        let second = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);

        assert!(first.is_none());
        assert!(
            second.is_none(),
            "editor-focused 'g'/'gg' is QueryEditorComponent's own vim handling, not a schema action"
        );
    }

    #[test]
    fn ctrl_y_exports_curl_even_while_the_sidebar_has_focus() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.insert_at_cursor("select 1");

        // The fake driver's `export_curl` defaults to `None`, so this is
        // just confirming the key is consumed here rather than falling
        // through to the sidebar's Enter/movement handling.
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
        // (a no-op there), not the results pane's yank...
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
    async fn l_expands_a_table_in_the_sidebar_but_scrolls_the_table_in_results() {
        let schema_with_columns = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![crate::query_driver::ColumnInfo {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
            }],
        }];
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(
            empty_result(),
            Ok(schema_with_columns),
        ));

        // Sidebar focused: `l` opens the table's columns.
        screen.focus = Focus::Sidebar;
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);
        screen.schema_sidebar.move_down();
        assert_eq!(screen.schema_sidebar.selected_name(), Some("id"));

        // Results focused: the same key scrolls the results table instead.
        screen.focus = Focus::Results;
        screen.results.set_result(QueryResult::Table {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
            truncated: false,
        });
        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);

        let text = {
            let backend = ratatui::backend::TestBackend::new(40, 12);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| screen.results.draw(frame, frame.area(), true))
                .unwrap();
            buffer_text(terminal.backend().buffer())
        };
        assert!(
            !text.contains(" a "),
            "column 'a' should have scrolled off: {text}"
        );
    }

    #[tokio::test]
    async fn enter_on_a_column_inserts_the_column_name() {
        let schema_with_columns = vec![SchemaInfo {
            name: "users".to_string(),
            columns: vec![crate::query_driver::ColumnInfo {
                name: "email".to_string(),
                type_name: "TEXT".to_string(),
            }],
        }];
        let (mut screen, _rx) = screen_with(fake_engine_with_schema(
            empty_result(),
            Ok(schema_with_columns),
        ));
        screen.focus = Focus::Sidebar;

        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(screen.query_editor.text(), "email");
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

        // The editor pane starts right of the 26-wide sidebar, at the top.
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 40,
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
        screen.picker = Some(FilePickerComponent::new(&[], dir));
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
}
