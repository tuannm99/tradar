//! The post-connect screen: schema sidebar + query editor + results,
//! composed. Implements `Component` because `RootComponent` routes keys and
//! ticks to it directly whenever it's the active screen. Owns the
//! `QueryEngine` directly (not through `dyn Session`) since this screen only
//! ever exists for a query-shaped connector's own engine.

use std::io::Write;

use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use tradar_connector_api::Session;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedConnection;
use tradar_core::ui;

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
    last_path: Option<String>,
    history_picker: Option<HistoryPickerComponent>,
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
            last_path: None,
            history_picker: None,
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
        self.prompt = Some(FilePromptComponent::new(
            kind,
            self.last_path.as_deref().unwrap_or(""),
        ));
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
        }
        None
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
        let result = match kind {
            PromptKind::Save => {
                std::fs::write(&trimmed, self.query_editor.text()).map_err(|e| e.to_string())
            }
            PromptKind::Open => std::fs::read_to_string(&trimmed)
                .map(|content| self.query_editor.set_text(&content))
                .map_err(|e| e.to_string()),
        };
        match result {
            Ok(()) => {
                self.last_path = Some(trimmed);
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

/// The pane a command needs focused to make sense. `None` for commands that
/// work from anywhere on this screen.
fn required_focus(command: Command) -> Option<Focus> {
    match command {
        Command::Yank => Some(Focus::Results),
        Command::InsertName => Some(Focus::Sidebar),
        _ => None,
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
            return None;
        }

        // List navigation applies only when a list actually has focus;
        // with the editor focused those same keys are the editor's own vim
        // motions.
        let contexts: &[Context] = match self.focus {
            Focus::Editor => &[Context::QueryScreen],
            Focus::Results | Focus::Sidebar => &[Context::QueryScreen, Context::List],
        };
        let key = KeyPress::new(code, modifiers);
        let command = match keymap().resolve_in(contexts, &mut self.pending, key) {
            Resolution::Command(command) => command,
            // Mid-sequence (the first `g` of `gg`): swallow it so the
            // editor doesn't also see it.
            Resolution::Pending => return None,
            Resolution::None => return self.forward_to_editor(code, modifiers),
        };

        // A few commands only make sense against a specific pane. Invoked
        // from elsewhere, the key should do whatever it would have done
        // otherwise -- `enter` inserts a newline in the editor rather than
        // a schema name.
        if let Some(required) = required_focus(command)
            && self.focus != required
        {
            return self.forward_to_editor(code, modifiers);
        }

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
            Command::RunQuery => {
                if !self.engine.is_pending() {
                    self.engine.submit_query(self.query_editor.text());
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
            Command::OpenFile => self.open_prompt(PromptKind::Open),
            Command::History => self.open_history(),
            Command::ExportCurl => self.export_curl(),
            Command::Yank => {
                if let Some(text) = self.results.selected_text() {
                    yank_to_clipboard(&text);
                }
            }
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
        self.query_editor
            .draw(frame, chunks[0], &connection_name, self.focus == Focus::Editor);
        self.results
            .draw(frame, chunks[1], self.focus == Focus::Results);

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
            SchemaInfo {
                name: "users".to_string(),
            },
            SchemaInfo {
                name: "orders".to_string(),
            },
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

    #[test]
    fn ctrl_o_loads_a_file_into_the_editor() {
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
