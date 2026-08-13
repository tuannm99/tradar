//! The connection-picker screen: list saved connections, select one,
//! request a connect. Implements `Component` because `RootComponent`
//! routes keys to it directly whenever it's the active screen.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedConnection;
use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list;

pub struct ConnectionPickerComponent {
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub last_error: Option<String>,
    pub connect_epoch: u64,
    /// Half-finished two-key binding (the first `g` of `gg`), owned here
    /// because it's per-list state -- see `tradar_core::keymap`.
    pending: Option<KeyPress>,
    visible_height: usize,
}

impl ConnectionPickerComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected: 0,
            last_error: None,
            connect_epoch: 0,
            pending: None,
            visible_height: 0,
        }
    }

    fn open_selected(&mut self) -> Option<Action> {
        let connection = self.connections.get(self.selected).cloned()?;
        self.connect_epoch += 1;
        // A stale error from a previous failed attempt must not keep
        // showing once a new attempt is underway.
        self.last_error = None;
        Some(Action::OpenRequested {
            connection,
            epoch: self.connect_epoch,
            // Placeholder -- RootComponent overwrites this with the real tab
            // index right after this call returns, since a lone picker has
            // no notion of which tab it belongs to.
            tab: 0,
        })
    }
}

impl Component for ConnectionPickerComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Picker, Context::List], &mut self.pending, key)
        else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            vim_list::apply(
                mv,
                &mut self.selected,
                self.connections.len(),
                self.visible_height,
            );
            return None;
        }

        match command {
            Command::Quit => Some(Action::Quit),
            Command::Open => self.open_selected(),
            Command::Help => Some(Action::ShowHelp),
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
        let theme = theme();

        // The error box, when there is one, takes the bottom of the area.
        let (list_area, error_area) = match &self.last_error {
            None => (area, None),
            Some(_) => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(4)])
                    .split(area);
                (chunks[0], Some(chunks[1]))
            }
        };

        let items: Vec<ListItem> = self
            .connections
            .iter()
            .map(|connection| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {}", connection.name),
                        Style::default().fg(theme.text),
                    ),
                    Span::styled(
                        format!("  {}", connection.driver),
                        Style::default().fg(theme.text_dim),
                    ),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        if !self.connections.is_empty() {
            state.select(Some(self.selected));
        }
        self.visible_height = list_area.height.saturating_sub(2) as usize;
        let list = List::new(items)
            .block(ui::panel("Connections", true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, list_area, &mut state);

        if let (Some(error_area), Some(error)) = (error_area, &self.last_error) {
            let error_box = Paragraph::new(Span::styled(
                error.as_str(),
                Style::default().fg(theme.error),
            ))
            .wrap(Wrap { trim: true })
            .block(ui::panel("Error", false));
            frame.render_widget(error_box, error_area);
        }
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
    fn question_mark_asks_for_the_help_overlay() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Char('?'), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::ShowHelp)));
    }

    #[test]
    fn an_unbound_key_does_nothing() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Char('z'), KeyModifiers::NONE);

        assert!(action.is_none());
        assert_eq!(picker.selected, 0);
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
