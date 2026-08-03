//! The connection-picker screen: list saved connections, select one,
//! request a connect. Implements `Component` because `RootComponent`
//! routes keys to it directly whenever it's the active screen.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::action::{Action, Component};
use crate::storage::SavedConnection;

pub struct ConnectionPickerComponent {
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub last_error: Option<String>,
}

impl ConnectionPickerComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected: 0,
            last_error: None,
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected + 1 < self.connections.len() {
            self.selected += 1;
        }
    }

    fn move_selection_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

impl Component for ConnectionPickerComponent {
    fn handle_key_event(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection_down();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection_up();
                None
            }
            KeyCode::Enter => self
                .connections
                .get(self.selected)
                .cloned()
                .map(Action::ConnectRequested),
            _ => None,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if let Action::ConnectFailed(error) = action {
            self.last_error = Some(error);
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .connections
            .iter()
            .enumerate()
            .map(|(i, connection)| {
                let item = ListItem::new(connection.name.clone());
                if i == self.selected {
                    item.style(Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    item
                }
            })
            .collect();

        let list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Connections"));

        let Some(error) = &self.last_error else {
            frame.render_widget(list, area);
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        frame.render_widget(list, chunks[0]);

        let error_box = Paragraph::new(error.as_str())
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error_box, chunks[1]);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;
    use crate::storage::DriverKind;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
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

    #[test]
    fn starts_with_nothing_selected() {
        let picker = ConnectionPickerComponent::new(connections());
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn move_selection_down_advances_and_stops_at_the_last_connection() {
        let mut picker = ConnectionPickerComponent::new(connections());

        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected, 1);

        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(
            picker.selected, 1,
            "should stop at the last connection, not wrap"
        );
    }

    #[test]
    fn move_selection_up_retreats_and_stops_at_zero() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0);

        picker.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0, "should stop at zero, not go negative");
    }

    #[test]
    fn q_returns_quit() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Char('q'), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn enter_returns_connect_requested_for_the_selected_connection() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match action {
            Some(Action::ConnectRequested(connection)) => {
                assert_eq!(connection.name, "local-postgres");
            }
            other => panic!(
                "expected ConnectRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn connect_failed_sets_the_last_error() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let next = picker.update(Action::ConnectFailed("connection refused".to_string()));

        assert_eq!(picker.last_error.as_deref(), Some("connection refused"));
        assert!(next.is_none());
    }

    #[test]
    fn draw_lists_saved_connection_names() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_a_connection_error() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.update(Action::ConnectFailed("connection refused".to_string()));
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("connection refused"), "buffer was: {text}");
    }
}
