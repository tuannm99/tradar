//! The connection-picker screen: list saved connections, select one,
//! request a connect. Implements `Component` because `RootComponent`
//! routes keys to it directly whenever it's the active screen.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use tradar_core::action::{Action, Component};
use tradar_core::storage::SavedConnection;
use tradar_core::vim_list;

pub struct ConnectionPickerComponent {
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub last_error: Option<String>,
    pub connect_epoch: u64,
    pending_g: bool,
    visible_height: usize,
}

impl ConnectionPickerComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected: 0,
            last_error: None,
            connect_epoch: 0,
            pending_g: false,
            visible_height: 0,
        }
    }
}

impl Component for ConnectionPickerComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(mv) = vim_list::recognize(code, modifiers, &mut self.pending_g) {
            vim_list::apply(
                mv,
                &mut self.selected,
                self.connections.len(),
                self.visible_height,
            );
            return None;
        }
        match code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Enter => {
                let connection = self.connections.get(self.selected).cloned()?;
                self.connect_epoch += 1;
                // A stale error from a previous failed attempt must not
                // keep showing once a new attempt is underway.
                self.last_error = None;
                Some(Action::OpenRequested {
                    connection,
                    epoch: self.connect_epoch,
                    // Placeholder -- RootComponent overwrites this with the
                    // real tab index right after this call returns, since a
                    // lone picker has no notion of which tab it belongs to.
                    tab: 0,
                })
            }
            _ => None,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if let Action::OpenFailed { error, .. } = action {
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
            self.visible_height = area.height.saturating_sub(2) as usize;
            frame.render_widget(list, area);
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        self.visible_height = chunks[0].height.saturating_sub(2) as usize;
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

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                name: "local-sqlite".to_string(),
                driver: "sqlite".to_string(),
                target: "test.db".to_string(),
            },
            SavedConnection {
                name: "local-postgres".to_string(),
                driver: "postgres".to_string(),
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
    fn gg_moves_to_the_top() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected, 1);

        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);

        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn a_single_g_does_not_move_the_selection() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);

        assert_eq!(
            picker.selected, 1,
            "a lone 'g' should not move anything yet"
        );
    }

    #[test]
    fn g_followed_by_a_different_key_cancels_the_pending_g() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('k'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);

        assert_eq!(
            picker.selected, 0,
            "the second 'g' here starts a fresh pair, not a leftover one"
        );
    }

    #[test]
    fn shift_g_moves_to_the_bottom() {
        let mut picker = ConnectionPickerComponent::new(connections());

        picker.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);

        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_scroll_by_half_the_visible_height() {
        let mut picker = ConnectionPickerComponent::new(vec![
            connections()[0].clone(),
            connections()[1].clone(),
            connections()[0].clone(),
            connections()[1].clone(),
            connections()[0].clone(),
            connections()[1].clone(),
        ]);
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();
        // 12-row area minus 2 border rows = 10 visible rows -> half page = 5.

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 5, "should clamp to the last connection");

        picker.handle_key_event(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 0);
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
            Some(Action::OpenRequested {
                connection, epoch, ..
            }) => {
                assert_eq!(connection.name, "local-postgres");
                assert_eq!(epoch, 1);
            }
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn enter_bumps_the_connect_epoch_on_every_request() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let first = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        let second = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested {
            epoch: first_epoch, ..
        }) = first
        else {
            panic!("expected OpenRequested");
        };
        let Some(Action::OpenRequested {
            epoch: second_epoch,
            ..
        }) = second
        else {
            panic!("expected OpenRequested");
        };
        assert_eq!(first_epoch, 1);
        assert_eq!(second_epoch, 2);
    }

    #[test]
    fn a_new_connect_attempt_clears_a_stale_error_from_a_previous_one() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 1,
            tab: 0,
        });
        assert!(picker.last_error.is_some());

        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(picker.last_error, None);
    }

    #[test]
    fn connect_failed_sets_the_last_error() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let next = picker.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 1,
            tab: 0,
        });

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
        picker.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 1,
            tab: 0,
        });
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("connection refused"), "buffer was: {text}");
    }
}
