//! The post-connect screen: schema sidebar + query editor + results,
//! composed. Implements `Component` because `RootComponent` routes keys and
//! ticks to it directly whenever it's the active screen. Owns the
//! `QueryEngine` directly (not through `dyn Session`) since this screen only
//! ever exists for a query-shaped connector's own engine.

use std::io::Write;

use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use edtui::EditorMode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use tradar_connector_api::Session;
use tradar_core::action::{Action, Component};
use tradar_core::storage::SavedConnection;

use crate::components::query_editor::QueryEditorComponent;
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
    pending_g: bool,
}

fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::F(5))
        || (code == KeyCode::Enter && modifiers.contains(KeyModifiers::CONTROL))
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

        Self {
            focus: Focus::Editor,
            schema_sidebar,
            query_editor: QueryEditorComponent::new(),
            results: ResultsComponent::new(),
            engine,
            pending_g: false,
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
}

impl Component for QueryScreenComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let had_pending_g = std::mem::take(&mut self.pending_g);
        match code {
            KeyCode::Esc if self.query_editor.state.mode != EditorMode::Normal => {
                self.query_editor
                    .forward_key(KeyEvent::new(code, modifiers));
                None
            }
            KeyCode::Esc => Some(Action::BackToPicker),
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Results,
                    Focus::Results => Focus::Sidebar,
                    Focus::Sidebar => Focus::Editor,
                };
                None
            }
            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.export_curl();
                None
            }
            _ if is_submit(code, modifiers) => {
                if !self.engine.is_pending() {
                    self.engine.submit_query(self.query_editor.text());
                }
                None
            }
            KeyCode::Char('g') if self.focus == Focus::Sidebar && had_pending_g => {
                self.schema_sidebar.move_to_top();
                None
            }
            KeyCode::Char('g') if self.focus == Focus::Results && had_pending_g => {
                self.results.move_to_top();
                None
            }
            KeyCode::Char('g') if self.focus == Focus::Sidebar || self.focus == Focus::Results => {
                self.pending_g = true;
                None
            }
            KeyCode::Char('G') if self.focus == Focus::Sidebar => {
                self.schema_sidebar.move_to_bottom();
                None
            }
            KeyCode::Char('G') if self.focus == Focus::Results => {
                self.results.move_to_bottom();
                None
            }
            KeyCode::Char('d')
                if self.focus == Focus::Sidebar && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.schema_sidebar.move_half_page_down();
                None
            }
            KeyCode::Char('d')
                if self.focus == Focus::Results && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.results.move_half_page_down();
                None
            }
            KeyCode::Char('u')
                if self.focus == Focus::Sidebar && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.schema_sidebar.move_half_page_up();
                None
            }
            KeyCode::Char('u')
                if self.focus == Focus::Results && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.results.move_half_page_up();
                None
            }
            _ if self.focus == Focus::Sidebar => match code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.schema_sidebar.move_down();
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.schema_sidebar.move_up();
                    None
                }
                KeyCode::Enter => {
                    if let Some(name) = self.schema_sidebar.selected_name() {
                        let name = name.to_string();
                        self.query_editor.insert_at_cursor(&name);
                        self.focus = Focus::Editor;
                    }
                    None
                }
                _ => None,
            },
            _ if self.focus == Focus::Results => match code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.results.move_down();
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.results.move_up();
                    None
                }
                KeyCode::Char('y') => {
                    if let Some(text) = self.results.selected_text() {
                        yank_to_clipboard(&text);
                    }
                    None
                }
                _ => None,
            },
            _ => {
                self.query_editor
                    .forward_key(KeyEvent::new(code, modifiers));
                None
            }
        }
    }

    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }

    fn tick(&mut self) {
        self.engine.tick();
        match self.engine.take_outcome() {
            Some(QueryOutcome::Completed { result }) => self.results.set_result(result),
            Some(QueryOutcome::Failed { error }) => self.results.set_error(error),
            None => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let outer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(1)])
            .split(area);

        self.schema_sidebar
            .draw(frame, outer[0], self.focus == Focus::Sidebar);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Min(1)])
            .split(outer[1]);

        let connection_name = self.active_connection().name.clone();
        self.query_editor.draw(frame, chunks[0], &connection_name);
        self.results
            .draw(frame, chunks[1], self.focus == Focus::Results);
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
        assert_eq!(screen.query_editor.state.mode, EditorMode::Insert);
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
            "editor-focused 'g'/'gg' is edtui's own vim handling, not a schema action"
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
        assert_eq!(screen.query_editor.state.mode, EditorMode::Insert);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(
            action.is_none(),
            "Esc must be consumed by the editor, not bubble to BackToPicker"
        );
        assert_eq!(screen.query_editor.state.mode, EditorMode::Normal);
    }

    #[test]
    fn esc_in_normal_mode_returns_back_to_picker_even_after_leaving_insert_mode() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(screen.query_editor.state.mode, EditorMode::Normal);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }
}
