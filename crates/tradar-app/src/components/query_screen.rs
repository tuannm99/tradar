//! The post-connect screen: schema sidebar + query editor + results,
//! composed. Implements `Component` because `RootComponent` routes keys
//! to it directly whenever it's the active screen. Owns the `QueryEngine`
//! and is the only place besides `main.rs` that spawns async work — safe
//! because it only touches the `Driver` trait via `QueryEngine`, never a
//! concrete driver module.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use edtui::EditorMode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use tradar_core::storage::SavedConnection;

use crate::action::{Action, Component};
use crate::components::query_editor::QueryEditorComponent;
use crate::components::results::ResultsComponent;
use crate::components::schema_sidebar::SchemaSidebarComponent;
use crate::query_engine::QueryEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
    Sidebar,
}

pub struct QueryScreenComponent {
    pub focus: Focus,
    pub active_connection: Option<SavedConnection>,
    pub schema_sidebar: SchemaSidebarComponent,
    pub query_editor: QueryEditorComponent,
    pub results: ResultsComponent,
    pub engine: Option<QueryEngine>,
    action_tx: UnboundedSender<Action>,
    epoch: u64,
    pending_g: bool,
}

fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::F(5))
        || (code == KeyCode::Enter && modifiers.contains(KeyModifiers::CONTROL))
}

impl QueryScreenComponent {
    pub fn new(action_tx: UnboundedSender<Action>) -> Self {
        Self {
            focus: Focus::Editor,
            active_connection: None,
            schema_sidebar: SchemaSidebarComponent::new(),
            query_editor: QueryEditorComponent::new(),
            results: ResultsComponent::new(),
            engine: None,
            action_tx,
            epoch: 0,
            pending_g: false,
        }
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
            KeyCode::Tab => Some(Action::ToggleFocus),
            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => self
                .active_connection
                .clone()
                .map(|connection| Action::ExportCurl {
                    connection,
                    query: self.query_editor.text(),
                }),
            _ if is_submit(code, modifiers) => Some(Action::SubmitQuery),
            KeyCode::Char('g') if self.focus == Focus::Sidebar && had_pending_g => {
                Some(Action::SchemaMoveTop)
            }
            KeyCode::Char('g') if self.focus == Focus::Results && had_pending_g => {
                Some(Action::ResultsMoveTop)
            }
            KeyCode::Char('g') if self.focus == Focus::Sidebar || self.focus == Focus::Results => {
                self.pending_g = true;
                None
            }
            KeyCode::Char('G') if self.focus == Focus::Sidebar => Some(Action::SchemaMoveBottom),
            KeyCode::Char('G') if self.focus == Focus::Results => Some(Action::ResultsMoveBottom),
            KeyCode::Char('d')
                if self.focus == Focus::Sidebar && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Some(Action::SchemaHalfPageDown)
            }
            KeyCode::Char('d')
                if self.focus == Focus::Results && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Some(Action::ResultsHalfPageDown)
            }
            KeyCode::Char('u')
                if self.focus == Focus::Sidebar && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Some(Action::SchemaHalfPageUp)
            }
            KeyCode::Char('u')
                if self.focus == Focus::Results && modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Some(Action::ResultsHalfPageUp)
            }
            _ if self.focus == Focus::Sidebar => match code {
                KeyCode::Down | KeyCode::Char('j') => Some(Action::SchemaMoveDown),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::SchemaMoveUp),
                KeyCode::Enter => Some(Action::InsertSchemaSelection),
                _ => None,
            },
            _ if self.focus == Focus::Results => match code {
                KeyCode::Down | KeyCode::Char('j') => Some(Action::ResultsMoveDown),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::ResultsMoveUp),
                KeyCode::Char('y') => self
                    .results
                    .selected_text()
                    .map(|text| Action::Yank { text }),
                _ => None,
            },
            _ => {
                self.query_editor
                    .forward_key(KeyEvent::new(code, modifiers));
                None
            }
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Connected {
                connection,
                engine,
                schema,
                ..
            } => {
                self.active_connection = Some(connection);
                self.engine = Some(engine);
                match schema {
                    Ok(schema) => self.schema_sidebar.set_schema(schema),
                    Err(e) => self.schema_sidebar.set_schema_error(e),
                }
                self.epoch += 1;
                None
            }
            Action::BackToPicker => {
                self.active_connection = None;
                self.engine = None;
                self.schema_sidebar.reset();
                self.focus = Focus::Editor;
                self.epoch += 1;
                None
            }
            Action::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Results,
                    Focus::Results => Focus::Sidebar,
                    Focus::Sidebar => Focus::Editor,
                };
                None
            }
            Action::SchemaMoveDown => {
                self.schema_sidebar.move_down();
                None
            }
            Action::SchemaMoveUp => {
                self.schema_sidebar.move_up();
                None
            }
            Action::SchemaMoveTop => {
                self.schema_sidebar.move_to_top();
                None
            }
            Action::SchemaMoveBottom => {
                self.schema_sidebar.move_to_bottom();
                None
            }
            Action::SchemaHalfPageDown => {
                self.schema_sidebar.move_half_page_down();
                None
            }
            Action::SchemaHalfPageUp => {
                self.schema_sidebar.move_half_page_up();
                None
            }
            Action::ResultsMoveDown => {
                self.results.move_down();
                None
            }
            Action::ResultsMoveUp => {
                self.results.move_up();
                None
            }
            Action::ResultsMoveTop => {
                self.results.move_to_top();
                None
            }
            Action::ResultsMoveBottom => {
                self.results.move_to_bottom();
                None
            }
            Action::ResultsHalfPageDown => {
                self.results.move_half_page_down();
                None
            }
            Action::ResultsHalfPageUp => {
                self.results.move_half_page_up();
                None
            }
            Action::InsertSchemaSelection => {
                if let Some(name) = self.schema_sidebar.selected_name() {
                    let name = name.to_string();
                    self.query_editor.insert_at_cursor(&name);
                    self.focus = Focus::Editor;
                }
                None
            }
            Action::SubmitQuery => {
                let engine = self.engine.take()?;
                let query = self.query_editor.text();
                let tx = self.action_tx.clone();
                let epoch = self.epoch;
                tokio::spawn(async move {
                    let mut engine = engine;
                    match engine.run(&query).await {
                        Ok(result) => {
                            let _ = tx.send(Action::QueryCompleted {
                                engine,
                                result,
                                epoch,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(Action::QueryFailed {
                                engine,
                                error: e.to_string(),
                                epoch,
                            });
                        }
                    }
                });
                None
            }
            Action::QueryCompleted {
                engine,
                result,
                epoch,
            } => {
                if epoch != self.epoch {
                    return None;
                }
                self.engine = Some(engine);
                self.results.set_result(result);
                None
            }
            Action::QueryFailed {
                engine,
                error,
                epoch,
            } => {
                if epoch != self.epoch {
                    return None;
                }
                self.engine = Some(engine);
                self.results.set_error(error);
                None
            }
            _ => None,
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

        let connection_name = self
            .active_connection
            .as_ref()
            .map(|c| c.name.as_str())
            .unwrap_or("");
        self.query_editor.draw(frame, chunks[0], connection_name);
        self.results
            .draw(frame, chunks[1], self.focus == Focus::Results);
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tokio::sync::mpsc;

    use tradar_core::storage::DriverKind;

    use super::*;
    use crate::drivers::{Driver, QueryResult, SchemaInfo};

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn connection() -> SavedConnection {
        SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
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
    impl Driver for FakeDriver {
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

    fn fake_engine(result: QueryResult) -> QueryEngine {
        QueryEngine::new(Box::new(FakeDriver { result }))
    }

    fn screen() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (QueryScreenComponent::new(tx), rx)
    }

    #[test]
    fn toggle_focus_cycles_editor_results_sidebar() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        screen.update(Action::ToggleFocus);
        assert_eq!(screen.focus, Focus::Results);

        screen.update(Action::ToggleFocus);
        assert_eq!(screen.focus, Focus::Sidebar);

        screen.update(Action::ToggleFocus);
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
    fn insert_schema_selection_inserts_the_selected_name_at_the_cursor() {
        let (mut screen, _rx) = screen();
        screen.schema_sidebar.set_schema(schema());
        screen.schema_sidebar.move_down();
        // Type "ab" then leave Insert mode -- vim leaves the cursor sitting
        // on the last-typed character ("b"), so the inserted name must land
        // between "a" and "b", not appended at the buffer's end.
        type_chars(&mut screen, "iab");
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        screen.focus = Focus::Sidebar;

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.text(), "aordersb");
        assert_eq!(screen.focus, Focus::Editor);
        assert_eq!(screen.query_editor.state.mode, EditorMode::Insert);
    }

    #[test]
    fn insert_schema_selection_is_a_no_op_when_schema_is_empty() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Sidebar;

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.text(), "");
        assert_eq!(
            screen.focus,
            Focus::Sidebar,
            "no-op must not change focus either"
        );
    }

    #[test]
    fn connected_stores_the_connection_engine_and_schema() {
        let (mut screen, _rx) = screen();

        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            schema: Ok(schema()),
            epoch: 0,
        });

        assert_eq!(screen.active_connection, Some(connection()));
        assert!(screen.engine.is_some());
        assert_eq!(screen.schema_sidebar.schema, schema());
    }

    #[test]
    fn connected_with_a_schema_error_sets_the_sidebar_error() {
        let (mut screen, _rx) = screen();

        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            schema: Err("scan failed".to_string()),
            epoch: 0,
        });

        assert_eq!(
            screen.schema_sidebar.schema_error.as_deref(),
            Some("scan failed")
        );
    }

    #[test]
    fn back_to_picker_clears_connection_engine_schema_and_focus() {
        let (mut screen, _rx) = screen();
        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            schema: Ok(schema()),
            epoch: 0,
        });
        screen.focus = Focus::Sidebar;

        screen.update(Action::BackToPicker);

        assert_eq!(screen.active_connection, None);
        assert!(screen.engine.is_none());
        assert_eq!(screen.schema_sidebar.schema, Vec::new());
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[tokio::test]
    async fn submit_query_runs_the_query_and_reports_query_completed() {
        let (mut screen, mut rx) = screen();
        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            }),
            schema: Ok(Vec::new()),
            epoch: 0,
        });
        screen.query_editor.insert_at_cursor("x");

        screen.update(Action::SubmitQuery);
        assert!(
            screen.engine.is_none(),
            "engine is taken while the query runs"
        );

        let action = rx.recv().await.expect("expected a completion action");
        match action {
            Action::QueryCompleted { result, .. } => {
                assert_eq!(
                    result,
                    QueryResult::Table {
                        columns: vec!["id".to_string()],
                        rows: vec![vec!["1".to_string()]],
                    }
                );
            }
            _ => panic!("expected QueryCompleted"),
        }
    }

    #[test]
    fn query_completed_puts_the_engine_back_and_sets_the_result() {
        let (mut screen, _rx) = screen();

        screen.update(Action::QueryCompleted {
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            result: QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            },
            epoch: 0,
        });

        assert!(screen.engine.is_some());
        assert_eq!(
            screen.results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }

    #[test]
    fn query_failed_puts_the_engine_back_and_sets_the_error() {
        let (mut screen, _rx) = screen();

        screen.update(Action::QueryFailed {
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            error: "syntax error".to_string(),
            epoch: 0,
        });

        assert!(screen.engine.is_some());
        assert_eq!(screen.results.last_error.as_deref(), Some("syntax error"));
    }

    #[test]
    fn stale_query_completed_from_a_previous_connection_is_dropped() {
        let (mut screen, _rx) = screen();

        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            schema: Ok(Vec::new()),
            epoch: 0,
        });
        assert!(screen.engine.is_some());

        // A reply from a query submitted before the `Connected` above (epoch 0)
        // arrives after the connect finished (which bumped epoch to 1). It
        // must be dropped rather than overwriting the freshly connected engine
        // and result state.
        screen.update(Action::QueryCompleted {
            engine: fake_engine(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            }),
            result: QueryResult::Table {
                columns: vec!["stale".to_string()],
                rows: vec![vec!["should-not-appear".to_string()]],
            },
            epoch: 0,
        });

        assert!(
            screen.engine.is_some(),
            "the connected engine must still be present, not overwritten by the stale reply"
        );
        assert_eq!(
            screen.results.last_result, None,
            "a stale reply must not populate results"
        );
    }

    fn sidebar_focused_screen_with_schema() -> QueryScreenComponent {
        let (mut screen, _rx) = screen();
        screen.schema_sidebar.set_schema(vec![SchemaInfo {
            name: "users".to_string(),
        }]);
        screen.focus = Focus::Sidebar;
        screen
    }

    #[test]
    fn ctrl_enter_runs_the_query_instead_of_inserting_the_schema_selection_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.insert_at_cursor("x");

        let action = screen.handle_key_event(KeyCode::Enter, KeyModifiers::CONTROL);

        assert!(matches!(action, Some(Action::SubmitQuery)));
        assert_eq!(screen.query_editor.text(), "x");
    }

    #[test]
    fn f5_runs_the_query_instead_of_being_swallowed_by_the_sidebar_guard() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.insert_at_cursor("x");

        let action = screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::SubmitQuery)));
        assert_eq!(screen.query_editor.text(), "x");
    }

    #[test]
    fn gg_returns_schema_move_top_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        let first = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(first.is_none(), "a lone 'g' should not act yet");

        let second = screen.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(matches!(second, Some(Action::SchemaMoveTop)));
    }

    #[test]
    fn shift_g_returns_schema_move_bottom_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        let action = screen.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::SchemaMoveBottom)));
    }

    #[test]
    fn ctrl_d_and_ctrl_u_return_schema_half_page_actions_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        let down = screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(matches!(down, Some(Action::SchemaHalfPageDown)));

        let up = screen.handle_key_event(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert!(matches!(up, Some(Action::SchemaHalfPageUp)));
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
    fn plain_enter_still_returns_insert_schema_selection_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        let action = screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::InsertSchemaSelection)));
    }

    #[test]
    fn ctrl_enter_submits() {
        assert!(is_submit(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    #[test]
    fn plain_enter_does_not_submit() {
        assert!(!is_submit(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn f5_submits_regardless_of_modifiers() {
        assert!(is_submit(KeyCode::F(5), KeyModifiers::NONE));
    }

    #[test]
    fn plain_characters_do_not_submit() {
        assert!(!is_submit(KeyCode::Char('a'), KeyModifiers::NONE));
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

    #[test]
    fn ctrl_y_exports_curl_even_while_the_sidebar_has_focus() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.active_connection = Some(connection());
        screen.query_editor.insert_at_cursor("select 1");

        let action = screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::CONTROL);

        match action {
            Some(Action::ExportCurl {
                connection: c,
                query,
            }) => {
                assert_eq!(c, connection());
                assert_eq!(query, "select 1");
            }
            other => panic!(
                "expected ExportCurl, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn draw_shows_active_connection_and_input() {
        let (mut screen, _rx) = screen();
        screen.active_connection = Some(connection());
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
}
