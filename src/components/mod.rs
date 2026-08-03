//! The component tree: `RootComponent` switches between the connection
//! picker and the query screen, routing keys and actions to whichever is
//! active. This module — like every file under `components/` — must
//! never depend on a concrete driver module; only `main.rs` may.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::{Action, Component};
use crate::components::connection_picker::ConnectionPickerComponent;
use crate::components::query_screen::QueryScreenComponent;
use crate::storage::SavedConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    ConnectionPicker,
    Query,
}

pub struct RootComponent {
    pub screen: Screen,
    pub connection_picker: ConnectionPickerComponent,
    pub query_screen: QueryScreenComponent,
    pub should_quit: bool,
}

impl RootComponent {
    pub fn new(connections: Vec<SavedConnection>, action_tx: UnboundedSender<Action>) -> Self {
        Self {
            screen: Screen::ConnectionPicker,
            connection_picker: ConnectionPickerComponent::new(connections),
            query_screen: QueryScreenComponent::new(action_tx),
            should_quit: false,
        }
    }
}

impl Component for RootComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.handle_key_event(code, modifiers),
            Screen::Query => self.query_screen.handle_key_event(code, modifiers),
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return None;
        }
        if matches!(action, Action::Connected { .. }) {
            self.query_screen.update(action);
            self.screen = Screen::Query;
            return None;
        }
        if matches!(action, Action::BackToPicker) {
            self.query_screen.update(action);
            self.screen = Screen::ConnectionPicker;
            return None;
        }
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.update(action),
            Screen::Query => self.query_screen.update(action),
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.draw(frame, area),
            Screen::Query => self.query_screen.draw(frame, area),
        }
    }
}

pub mod connection_picker;
pub mod query_editor;
pub mod query_screen;
pub mod results;
pub mod schema_sidebar;

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use super::*;
    use crate::drivers::{Driver, QueryResult, SchemaInfo};
    use crate::query_engine::QueryEngine;
    use crate::storage::DriverKind;

    struct FakeDriver;

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(QueryResult::Table {
                columns: vec![],
                rows: vec![],
            })
        }
    }

    fn connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                name: "local-sqlite".to_string(),
                driver: DriverKind::Sqlite,
                target: "test.db".to_string(),
            },
            SavedConnection {
                name: "local-postgres".to_string(),
                driver: DriverKind::Postgres,
                target: "postgres://localhost/test".to_string(),
            },
        ]
    }

    fn root() -> (RootComponent, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (RootComponent::new(connections(), tx), rx)
    }

    #[test]
    fn starts_on_the_connection_picker_with_nothing_selected() {
        let (root, _rx) = root();

        assert_eq!(root.screen, Screen::ConnectionPicker);
        assert_eq!(root.connection_picker.selected, 0);
    }

    #[test]
    fn quit_sets_should_quit() {
        let (mut root, _rx) = root();

        assert!(!root.should_quit);
        root.update(Action::Quit);

        assert!(root.should_quit);
    }

    #[test]
    fn connected_switches_to_the_query_screen() {
        let (mut root, _rx) = root();
        let connection = connections()[1].clone();

        root.update(Action::Connected {
            connection: connection.clone(),
            engine: QueryEngine::new(Box::new(FakeDriver)),
            schema: Ok(Vec::new()),
        });

        assert_eq!(root.screen, Screen::Query);
        assert_eq!(root.query_screen.active_connection, Some(connection));
    }

    #[test]
    fn back_to_picker_returns_to_the_connection_picker() {
        let (mut root, _rx) = root();
        root.update(Action::Connected {
            connection: connections()[0].clone(),
            engine: QueryEngine::new(Box::new(FakeDriver)),
            schema: Ok(Vec::new()),
        });

        root.update(Action::BackToPicker);

        assert_eq!(root.screen, Screen::ConnectionPicker);
        assert_eq!(root.query_screen.active_connection, None);
    }

    #[test]
    fn connect_failed_while_on_the_picker_sets_its_error() {
        let (mut root, _rx) = root();

        root.update(Action::ConnectFailed("connection refused".to_string()));

        assert_eq!(
            root.connection_picker.last_error.as_deref(),
            Some("connection refused")
        );
    }

    #[test]
    fn handle_key_event_routes_to_the_active_screen() {
        let (mut root, _rx) = root();

        let action = root.handle_key_event(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );

        assert!(matches!(action, Some(Action::Quit)));
    }
}
