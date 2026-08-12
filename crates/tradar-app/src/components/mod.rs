//! The component tree: `RootComponent` is a lightweight workspace of tabs,
//! each independently either on the connection picker or an active
//! connector `Screen`, routing keys/actions/ticks to whichever tab is
//! active. This module -- like every file under `components/` -- must never
//! depend on a concrete driver module; only `main.rs` and `connectors.rs`
//! may.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::{SavedConnection, SessionState};
use tradar_core::theme::theme;
use tradar_core::ui::{self, HelpOverlay};

use crate::components::connection_picker::ConnectionPickerComponent;

pub enum ScreenSlot {
    ConnectionPicker,
    Active(Box<dyn Component>),
}

pub struct Tab {
    pub screen: ScreenSlot,
    pub connection_picker: ConnectionPickerComponent,
    /// The name of the connection backing `screen` while it's `Active` --
    /// kept here (rather than read off `screen`) purely so the tab bar has
    /// something to label the tab with; `Component` has no generic "title"
    /// method and adding one just for this would leak a UI concern into
    /// every connector's `Screen`.
    pub title: Option<String>,
}

impl Tab {
    fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            screen: ScreenSlot::ConnectionPicker,
            connection_picker: ConnectionPickerComponent::new(connections),
            title: None,
        }
    }

    fn label(&self) -> String {
        self.title.clone().unwrap_or_else(|| "picker".to_string())
    }
}

pub struct RootComponent {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub should_quit: bool,
    /// The saved connections every new tab's picker starts out showing.
    connections: Vec<SavedConnection>,
    /// The key-bindings overlay, when raised. It sits above every tab, so
    /// it lives here rather than on a `Tab`.
    help: Option<HelpOverlay>,
    /// Half-finished two-key global binding.
    pending: Option<KeyPress>,
}

impl RootComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            tabs: vec![Tab::new(connections.clone())],
            active_tab: 0,
            should_quit: false,
            connections,
            help: None,
            pending: None,
        }
    }

    fn new_tab(&mut self) {
        self.tabs.push(Tab::new(self.connections.clone()));
        self.active_tab = self.tabs.len() - 1;
    }

    /// A no-op when only one tab is left -- closing the last tab has no
    /// sensible target to fall back to, and `q` from the picker is already
    /// the way to quit the app.
    fn close_active_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1).min(self.tabs.len() - 1);
    }

    fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.saturating_sub(1);
    }

    /// Recreates tabs for a previously-saved session, each immediately
    /// requesting a connect to its saved connection -- synthesizing the
    /// exact `Action::OpenRequested` a real `Enter` keypress on that tab's
    /// picker would produce, so `main.rs` can hand it to `spawn_connect`
    /// the same way it does for user-driven connects. A name from `session`
    /// that no longer matches any saved connection is silently skipped.
    pub fn restore_tabs(&mut self, session: &SessionState) -> Vec<Action> {
        let mut requests = Vec::new();
        let mut restored_any = false;
        for name in &session.tabs {
            let Some(index) = self.connections.iter().position(|c| &c.name == name) else {
                continue;
            };
            let tab_index = if restored_any {
                self.new_tab();
                self.tabs.len() - 1
            } else {
                0
            };
            restored_any = true;
            self.tabs[tab_index].connection_picker.selected = index;
            let action = self.tabs[tab_index]
                .connection_picker
                .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
            if let Some(Action::OpenRequested {
                connection, epoch, ..
            }) = action
            {
                requests.push(Action::OpenRequested {
                    connection,
                    epoch,
                    tab: tab_index,
                });
            }
        }
        if restored_any {
            self.active_tab = session.active_tab.min(self.tabs.len() - 1);
        }
        requests
    }
}

impl Component for RootComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        // The help overlay swallows everything while it's up.
        if let Some(help) = &mut self.help {
            if help.handle_key_event(code, modifiers) {
                self.help = None;
            }
            return None;
        }

        // Global bindings win everywhere -- including `quit`, which is why
        // quitting works while a tab is `Active` instead of needing `Esc`
        // back to the picker first (that would clear the tab's title, and
        // with it whether the session save remembers it was connected).
        let key = KeyPress::new(code, modifiers);
        if let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Global], &mut self.pending, key)
        {
            match command {
                Command::Quit => self.should_quit = true,
                Command::NewTab => self.new_tab(),
                Command::CloseTab => self.close_active_tab(),
                Command::NextTab => self.next_tab(),
                Command::PrevTab => self.prev_tab(),
                _ => {}
            }
            return None;
        }

        let tab_index = self.active_tab;
        let action = match &mut self.tabs[tab_index].screen {
            ScreenSlot::ConnectionPicker => self.tabs[tab_index]
                .connection_picker
                .handle_key_event(code, modifiers),
            ScreenSlot::Active(screen) => screen.handle_key_event(code, modifiers),
        };

        // `ConnectionPickerComponent` has no notion of tabs, so it fills
        // `tab` with a placeholder -- overwrite it with the real index here,
        // the one place that actually knows it.
        match action {
            Some(Action::OpenRequested {
                connection, epoch, ..
            }) => Some(Action::OpenRequested {
                connection,
                epoch,
                tab: tab_index,
            }),
            other => other,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Quit => {
                self.should_quit = true;
                None
            }
            Action::Opened {
                connection,
                screen,
                epoch,
                tab,
            } => {
                // A reply from a connect attempt that's been superseded by a
                // newer one on the same tab -- drop it instead of silently
                // switching that tab's screen back. `tab` may also no
                // longer exist at all if the user closed it while this
                // connect was in flight; `get_mut` makes that a no-op too.
                if let Some(t) = self.tabs.get_mut(tab)
                    && epoch == t.connection_picker.connect_epoch
                {
                    t.title = Some(connection.name.clone());
                    t.screen = ScreenSlot::Active(screen);
                }
                None
            }
            Action::OpenFailed { error, epoch, tab } => {
                if let Some(t) = self.tabs.get_mut(tab)
                    && epoch == t.connection_picker.connect_epoch
                {
                    t.connection_picker.last_error = Some(error);
                }
                None
            }
            Action::BackToPicker => {
                let tab = &mut self.tabs[self.active_tab];
                tab.screen = ScreenSlot::ConnectionPicker;
                tab.title = None;
                None
            }
            Action::ShowHelp => {
                self.help = Some(HelpOverlay::new());
                None
            }
            other => {
                let tab = &mut self.tabs[self.active_tab];
                match &mut tab.screen {
                    ScreenSlot::ConnectionPicker => tab.connection_picker.update(other),
                    ScreenSlot::Active(screen) => screen.update(other),
                }
            }
        }
    }

    fn tick(&mut self) -> bool {
        if let ScreenSlot::Active(screen) = &mut self.tabs[self.active_tab].screen {
            screen.tick()
        } else {
            false
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        // A tab bar only when there's more than one tab (nothing to switch
        // between otherwise), and always a one-line hint bar at the bottom.
        let show_tab_bar = self.tabs.len() > 1;
        let constraints = if show_tab_bar {
            vec![
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
        } else {
            vec![Constraint::Min(1), Constraint::Length(1)]
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        let (content_area, status_area) = if show_tab_bar {
            draw_tab_bar(frame, chunks[0], &self.tabs, self.active_tab);
            (chunks[1], chunks[2])
        } else {
            (chunks[0], chunks[1])
        };

        let tab = &mut self.tabs[self.active_tab];
        match &mut tab.screen {
            ScreenSlot::ConnectionPicker => tab.connection_picker.draw(frame, content_area),
            ScreenSlot::Active(screen) => screen.draw(frame, content_area),
        }

        self.draw_status_bar(frame, status_area);

        // Drawn last so it sits above everything, including the tab bar.
        if let Some(help) = &mut self.help {
            help.draw(frame, area);
        }
    }
}

impl RootComponent {
    /// The bottom hint bar. Hints come from the live keymap, so a remapped
    /// key shows its new binding here without this code knowing.
    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let on_picker = matches!(
            self.tabs[self.active_tab].screen,
            ScreenSlot::ConnectionPicker
        );
        let screen_context = if on_picker {
            Context::Picker
        } else {
            Context::QueryScreen
        };

        let mut hints = Vec::new();
        if on_picker {
            hints.extend(ui::hint(Context::Picker, Command::Open, "connect"));
            hints.extend(ui::hint(Context::List, Command::MoveDown, "move"));
        } else {
            hints.extend(ui::hint(
                Context::QueryScreen,
                Command::RunQuery,
                "run",
            ));
            hints.extend(ui::hint(
                Context::QueryScreen,
                Command::CycleFocus,
                "focus",
            ));
            hints.extend(ui::hint(
                Context::QueryScreen,
                Command::History,
                "history",
            ));
            hints.extend(ui::hint(Context::QueryScreen, Command::Back, "back"));
        }
        hints.extend(ui::hint(Context::Global, Command::NewTab, "tab"));
        hints.extend(ui::hint(screen_context, Command::Help, "help"));

        let right = self.tabs[self.active_tab].title.clone();
        ui::draw_status_bar(frame, area, &hints, right.as_deref());
    }
}

fn draw_tab_bar(frame: &mut Frame, area: Rect, tabs: &[Tab], active: usize) {
    let theme = theme();
    let spans: Vec<Span> = tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let label = format!(" {}: {} ", i + 1, tab.label());
            let style = if i == active {
                Style::default()
                    .bg(theme.tab_active_bg)
                    .fg(theme.tab_active_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.tab_inactive)
            };
            Span::styled(label, style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
    fn starts_with_a_single_tab_on_the_connection_picker() {
        let root = root();

        assert_eq!(root.tabs.len(), 1);
        assert_eq!(root.active_tab, 0);
        assert!(matches!(root.tabs[0].screen, ScreenSlot::ConnectionPicker));
        assert_eq!(root.tabs[0].connection_picker.selected, 0);
    }

    #[test]
    fn quit_sets_should_quit() {
        let mut root = root();

        assert!(!root.should_quit);
        root.update(Action::Quit);

        assert!(root.should_quit);
    }

    #[test]
    fn opened_switches_to_the_active_screen_and_sets_the_tab_title() {
        let mut root = root();
        let connection = connections()[1].clone();
        let dropped = Rc::new(Cell::new(false));

        root.update(Action::Opened {
            connection,
            screen: Box::new(FakeScreen { dropped }),
            epoch: 0,
            tab: 0,
        });

        assert!(matches!(root.tabs[0].screen, ScreenSlot::Active(_)));
        assert_eq!(root.tabs[0].title.as_deref(), Some("local-postgres"));
    }

    #[test]
    fn back_to_picker_returns_the_active_tab_to_the_connection_picker() {
        let mut root = root();
        root.update(Action::Opened {
            connection: connections()[0].clone(),
            screen: Box::new(FakeScreen {
                dropped: Rc::new(Cell::new(false)),
            }),
            epoch: 0,
            tab: 0,
        });

        root.update(Action::BackToPicker);

        assert!(matches!(root.tabs[0].screen, ScreenSlot::ConnectionPicker));
        assert!(root.tabs[0].title.is_none());
    }

    #[test]
    fn open_failed_while_on_the_picker_sets_its_error() {
        let mut root = root();

        root.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 0,
            tab: 0,
        });

        assert_eq!(
            root.tabs[0].connection_picker.last_error.as_deref(),
            Some("connection refused")
        );
    }

    #[test]
    fn a_stale_opened_from_a_superseded_connect_attempt_is_ignored() {
        let mut root = root();
        let conn_a = connections()[0].clone();
        let conn_b = connections()[1].clone();

        // Connect to A, then connect to B before A resolves -- mirrors the
        // real race: both connect attempts are in flight at once, both on
        // the same tab.
        let request_a = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        root.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        let request_b = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

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
            tab: 0,
        });
        assert!(matches!(root.tabs[0].screen, ScreenSlot::Active(_)));

        // A's stale reply arrives after -- it must not override B.
        root.update(Action::Opened {
            connection: conn_a,
            screen: Box::new(FakeScreen {
                dropped: dropped_a.clone(),
            }),
            epoch: epoch_a,
            tab: 0,
        });

        assert!(matches!(root.tabs[0].screen, ScreenSlot::Active(_)));
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

        let request_a = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        root.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested { epoch: epoch_a, .. }) = request_a else {
            panic!("expected OpenRequested for A");
        };

        root.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: epoch_a,
            tab: 0,
        });

        assert_eq!(
            root.tabs[0].connection_picker.last_error, None,
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

    #[test]
    fn handle_key_event_stamps_open_requested_with_the_active_tab_index() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 1);

        let action = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested { tab, .. }) = action else {
            panic!("expected OpenRequested");
        };
        assert_eq!(tab, 1);
    }

    #[test]
    fn ctrl_q_quits_even_while_a_tab_is_active_not_just_from_the_picker() {
        let mut root = root();
        root.update(Action::Opened {
            connection: connections()[0].clone(),
            screen: Box::new(FakeScreen {
                dropped: Rc::new(Cell::new(false)),
            }),
            epoch: 0,
            tab: 0,
        });
        assert!(matches!(root.tabs[0].screen, ScreenSlot::Active(_)));

        root.handle_key_event(KeyCode::Char('q'), KeyModifiers::CONTROL);

        assert!(root.should_quit);
        assert!(
            matches!(root.tabs[0].screen, ScreenSlot::Active(_)),
            "quitting must not first bounce the tab back to the picker"
        );
    }

    #[test]
    fn ctrl_t_opens_a_new_tab_and_makes_it_active() {
        let mut root = root();

        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);

        assert_eq!(root.tabs.len(), 2);
        assert_eq!(root.active_tab, 1);
        assert!(matches!(root.tabs[1].screen, ScreenSlot::ConnectionPicker));
    }

    #[test]
    fn ctrl_w_closes_the_active_tab_but_never_the_last_one() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.tabs.len(), 2);

        root.handle_key_event(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(root.tabs.len(), 1);
        assert_eq!(root.active_tab, 0);

        root.handle_key_event(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(root.tabs.len(), 1, "must never close the last tab");
    }

    #[test]
    fn ctrl_left_and_ctrl_right_switch_between_tabs() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 2);

        root.handle_key_event(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 1);

        root.handle_key_event(KeyCode::Left, KeyModifiers::CONTROL);
        root.handle_key_event(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 0, "should clamp, not go negative");

        root.handle_key_event(KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 1);
    }

    #[test]
    fn opened_for_a_background_tab_does_not_switch_the_active_tab() {
        let mut root = root();
        // Tab 0 starts connecting to A ...
        let request_a = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        let Some(Action::OpenRequested { epoch: epoch_a, .. }) = request_a else {
            panic!("expected OpenRequested for A");
        };
        // ... then the user opens a second tab and stays there while A is
        // still connecting in the background.
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 1);

        root.update(Action::Opened {
            connection: connections()[0].clone(),
            screen: Box::new(FakeScreen {
                dropped: Rc::new(Cell::new(false)),
            }),
            epoch: epoch_a,
            tab: 0,
        });

        assert_eq!(root.active_tab, 1, "resolving tab 0 must not steal focus");
        assert!(matches!(root.tabs[0].screen, ScreenSlot::Active(_)));
        assert!(matches!(root.tabs[1].screen, ScreenSlot::ConnectionPicker));
    }

    #[test]
    fn closing_a_tab_while_its_connect_is_in_flight_drops_the_reply_harmlessly() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 2);

        let request = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        let Some(Action::OpenRequested {
            epoch,
            tab: tab_idx,
            ..
        }) = request
        else {
            panic!("expected OpenRequested");
        };
        assert_eq!(tab_idx, 2);

        root.handle_key_event(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(
            root.tabs.len(),
            2,
            "tab 2 (the one still connecting) is gone"
        );

        // `tab: 2` no longer refers to any tab at all -- must not panic.
        root.update(Action::Opened {
            connection: connections()[0].clone(),
            screen: Box::new(FakeScreen {
                dropped: Rc::new(Cell::new(false)),
            }),
            epoch,
            tab: tab_idx,
        });

        assert_eq!(root.tabs.len(), 2);
    }

    #[test]
    fn restore_tabs_creates_one_tab_per_known_connection_and_returns_requests() {
        let mut root = root();

        let requests = root.restore_tabs(&SessionState {
            active_tab: 1,
            tabs: vec!["local-sqlite".to_string(), "local-postgres".to_string()],
        });

        assert_eq!(root.tabs.len(), 2);
        assert_eq!(root.active_tab, 1);
        assert_eq!(requests.len(), 2);
        let Action::OpenRequested {
            connection, tab, ..
        } = &requests[0]
        else {
            panic!("expected OpenRequested");
        };
        assert_eq!(connection.name, "local-sqlite");
        assert_eq!(*tab, 0);
        let Action::OpenRequested {
            connection, tab, ..
        } = &requests[1]
        else {
            panic!("expected OpenRequested");
        };
        assert_eq!(connection.name, "local-postgres");
        assert_eq!(*tab, 1);
    }

    #[test]
    fn restore_tabs_skips_names_that_no_longer_match_a_saved_connection() {
        let mut root = root();

        let requests = root.restore_tabs(&SessionState {
            active_tab: 0,
            tabs: vec!["renamed-or-deleted".to_string(), "local-sqlite".to_string()],
        });

        assert_eq!(root.tabs.len(), 1);
        assert_eq!(requests.len(), 1);
        let Action::OpenRequested { connection, .. } = &requests[0] else {
            panic!("expected OpenRequested");
        };
        assert_eq!(connection.name, "local-sqlite");
    }

    #[test]
    fn restore_tabs_with_no_matches_leaves_the_single_default_tab_untouched() {
        let mut root = root();

        let requests = root.restore_tabs(&SessionState {
            active_tab: 0,
            tabs: vec!["renamed-or-deleted".to_string()],
        });

        assert!(requests.is_empty());
        assert_eq!(root.tabs.len(), 1);
        assert_eq!(root.active_tab, 0);
        assert!(matches!(root.tabs[0].screen, ScreenSlot::ConnectionPicker));
    }
}
