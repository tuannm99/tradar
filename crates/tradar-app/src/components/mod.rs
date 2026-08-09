//! The component tree: `RootComponent` switches between the connection
//! picker and whatever `Screen` a connector's `Session` builds, routing
//! keys/actions/ticks to whichever is active. This module — like every
//! file under `components/` — must never depend on a concrete driver
//! module; only `main.rs` and `connectors.rs` may.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use tradar_core::action::{Action, Component};
use tradar_core::storage::SavedConnection;

use crate::components::connection_picker::ConnectionPickerComponent;

pub enum ScreenSlot {
    ConnectionPicker,
    Active(Box<dyn Component>),
}

pub struct RootComponent {
    pub screen: ScreenSlot,
    pub connection_picker: ConnectionPickerComponent,
    pub should_quit: bool,
}

impl RootComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            screen: ScreenSlot::ConnectionPicker,
            connection_picker: ConnectionPickerComponent::new(connections),
            should_quit: false,
        }
    }
}

impl Component for RootComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match &mut self.screen {
            ScreenSlot::ConnectionPicker => {
                self.connection_picker.handle_key_event(code, modifiers)
            }
            ScreenSlot::Active(screen) => screen.handle_key_event(code, modifiers),
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Quit => {
                self.should_quit = true;
                None
            }
            Action::Opened { screen, epoch, .. } => {
                // A reply from a connect attempt that's been superseded by a
                // newer one -- drop it instead of silently switching the
                // active screen back.
                if epoch == self.connection_picker.connect_epoch {
                    self.screen = ScreenSlot::Active(screen);
                }
                None
            }
            Action::OpenFailed { error, epoch } => {
                if epoch == self.connection_picker.connect_epoch {
                    self.connection_picker
                        .update(Action::OpenFailed { error, epoch });
                }
                None
            }
            Action::BackToPicker => {
                self.screen = ScreenSlot::ConnectionPicker;
                None
            }
            other => match &mut self.screen {
                ScreenSlot::ConnectionPicker => self.connection_picker.update(other),
                ScreenSlot::Active(screen) => screen.update(other),
            },
        }
    }

    fn tick(&mut self) {
        if let ScreenSlot::Active(screen) = &mut self.screen {
            screen.tick();
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        match &mut self.screen {
            ScreenSlot::ConnectionPicker => self.connection_picker.draw(frame, area),
            ScreenSlot::Active(screen) => screen.draw(frame, area),
        }
    }
}

pub mod connection_picker;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    struct FakeScreen {
        dropped: Rc<Cell<bool>>,
    }

    impl Drop for FakeScreen {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }

    impl Component for FakeScreen {
        fn handle_key_event(&mut self, _code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
            None
        }
        fn update(&mut self, _action: Action) -> Option<Action> {
            None
        }
        fn draw(&mut self, _frame: &mut Frame, _area: Rect) {}
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

    fn root() -> RootComponent {
        RootComponent::new(connections())
    }

    #[test]
    fn starts_on_the_connection_picker_with_nothing_selected() {
        let root = root();

        assert!(matches!(root.screen, ScreenSlot::ConnectionPicker));
        assert_eq!(root.connection_picker.selected, 0);
    }

    #[test]
    fn quit_sets_should_quit() {
        let mut root = root();

        assert!(!root.should_quit);
        root.update(Action::Quit);

        assert!(root.should_quit);
    }

    #[test]
    fn opened_switches_to_the_active_screen() {
        let mut root = root();
        let connection = connections()[1].clone();
        let dropped = Rc::new(Cell::new(false));

        root.update(Action::Opened {
            connection,
            screen: Box::new(FakeScreen { dropped }),
            epoch: 0,
        });

        assert!(matches!(root.screen, ScreenSlot::Active(_)));
    }

    #[test]
    fn back_to_picker_returns_to_the_connection_picker() {
        let mut root = root();
        root.update(Action::Opened {
            connection: connections()[0].clone(),
            screen: Box::new(FakeScreen {
                dropped: Rc::new(Cell::new(false)),
            }),
            epoch: 0,
        });

        root.update(Action::BackToPicker);

        assert!(matches!(root.screen, ScreenSlot::ConnectionPicker));
    }

    #[test]
    fn open_failed_while_on_the_picker_sets_its_error() {
        let mut root = root();

        root.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 0,
        });

        assert_eq!(
            root.connection_picker.last_error.as_deref(),
            Some("connection refused")
        );
    }

    #[test]
    fn a_stale_opened_from_a_superseded_connect_attempt_is_ignored() {
        let mut root = root();
        let conn_a = connections()[0].clone();
        let conn_b = connections()[1].clone();

        // Connect to A, then connect to B before A resolves -- mirrors the
        // real race: both connect attempts are in flight at once.
        let request_a = root
            .connection_picker
            .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        root.connection_picker
            .handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        let request_b = root
            .connection_picker
            .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested { epoch: epoch_a, .. }) = request_a else {
            panic!("expected OpenRequested for A");
        };
        let Some(Action::OpenRequested { epoch: epoch_b, .. }) = request_b else {
            panic!("expected OpenRequested for B");
        };

        let dropped_a = Rc::new(Cell::new(false));
        let dropped_b = Rc::new(Cell::new(false));

        // B resolves first.
        root.update(Action::Opened {
            connection: conn_b,
            screen: Box::new(FakeScreen {
                dropped: dropped_b.clone(),
            }),
            epoch: epoch_b,
        });
        assert!(matches!(root.screen, ScreenSlot::Active(_)));

        // A's stale reply arrives after -- it must not override B.
        root.update(Action::Opened {
            connection: conn_a,
            screen: Box::new(FakeScreen {
                dropped: dropped_a.clone(),
            }),
            epoch: epoch_a,
        });

        assert!(matches!(root.screen, ScreenSlot::Active(_)));
        assert!(
            dropped_a.get(),
            "a stale Opened for a superseded connect attempt must be dropped immediately, \
             never installed"
        );
        assert!(
            !dropped_b.get(),
            "the active (B) screen must not be replaced by a stale reply"
        );
    }

    #[test]
    fn a_stale_open_failed_from_a_superseded_connect_attempt_is_ignored() {
        let mut root = root();

        let request_a = root
            .connection_picker
            .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        root.connection_picker
            .handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        root.connection_picker
            .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested { epoch: epoch_a, .. }) = request_a else {
            panic!("expected OpenRequested for A");
        };

        root.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: epoch_a,
        });

        assert_eq!(
            root.connection_picker.last_error, None,
            "a stale OpenFailed for a superseded connect attempt must not surface an error"
        );
    }

    #[test]
    fn handle_key_event_routes_to_the_active_screen() {
        let mut root = root();

        let action = root.handle_key_event(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );

        assert!(matches!(action, Some(Action::Quit)));
    }
}
