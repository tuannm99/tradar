//! The component tree: `RootComponent` is a lightweight workspace of tabs,
//! each independently either on the connection picker or an active
//! connector `Screen`, routing keys/actions/ticks to whichever tab is
//! active. This module -- like every file under `components/` -- must never
//! depend on a concrete driver module; only `main.rs` and `connectors.rs`
//! may.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::{ConnectionStore, SavedConnection, SessionState};
use tradar_core::theme::theme;
use tradar_core::ui::{self, HelpOverlay};
use tradar_core::vim_list::VimMove;

use crate::components::connection_picker::ConnectionPickerComponent;
use crate::components::navigator::{NavConnection, NavOutcome, NavigatorComponent};

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
    fn new(connections: Vec<SavedConnection>, editing: Option<&Editing>) -> Self {
        let mut connection_picker = ConnectionPickerComponent::new(connections);
        if let Some(editing) = editing {
            connection_picker =
                connection_picker.with_editing(editing.drivers.clone(), editing.store.clone());
        }
        Self {
            screen: ScreenSlot::ConnectionPicker,
            connection_picker,
            title: None,
        }
    }

    fn label(&self) -> String {
        self.title.clone().unwrap_or_else(|| "picker".to_string())
    }
}

/// What a picker needs to edit the connection list: the connector ids
/// compiled into this build (from `main.rs`'s registry) and where to save.
/// Absent in tests that only exercise navigation, which leaves the picker
/// read-only.
#[derive(Clone)]
pub struct Editing {
    pub drivers: Vec<String>,
    pub store: ConnectionStore,
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
    /// Present once `with_editing` is called; every tab's picker gets it.
    editing: Option<Editing>,
    /// Where the tab bar was last drawn, so a click can switch tabs.
    /// Zero-sized while a single tab means there's no bar to click.
    tab_bar_area: Rect,
    /// The tab bar's per-tab label widths, in order -- what turns a click
    /// column into a tab index.
    tab_label_widths: Vec<u16>,
    /// The cross-connection tree down the left side. Lives here because
    /// only this component knows what other connections exist and which tab
    /// each is open on.
    navigator: NavigatorComponent,
    /// Whether the navigator panel is on screen, and whether it has the
    /// keys. Both off by default: the screen gets the whole terminal until
    /// you ask for the tree.
    navigator_open: bool,
    navigator_focused: bool,
}

impl RootComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            tabs: vec![Tab::new(connections.clone(), None)],
            active_tab: 0,
            should_quit: false,
            connections,
            help: None,
            pending: None,
            editing: None,
            tab_bar_area: Rect::ZERO,
            tab_label_widths: Vec::new(),
            navigator: NavigatorComponent::new(),
            navigator_open: false,
            navigator_focused: false,
        }
    }

    /// Lets every picker add/edit/delete connections. `main.rs` calls this
    /// with the connector registry's ids and the store it loaded from.
    pub fn with_editing(mut self, drivers: Vec<String>, store: ConnectionStore) -> Self {
        let editing = Editing { drivers, store };
        for tab in &mut self.tabs {
            tab.connection_picker = std::mem::replace(
                &mut tab.connection_picker,
                ConnectionPickerComponent::new(Vec::new()),
            )
            .with_editing(editing.drivers.clone(), editing.store.clone());
        }
        self.editing = Some(editing);
        self
    }

    fn new_tab(&mut self) {
        self.tabs
            .push(Tab::new(self.connections.clone(), self.editing.as_ref()));
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
        for tab in &session.tabs {
            let Some(index) = self
                .connections
                .iter()
                .position(|c| c.name == tab.connection)
            else {
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

    /// The navigator's view of the world: every saved connection, plus what
    /// the tab holding it (if any) has to browse. Rebuilt each time it's
    /// needed -- the alternative is a copy of every schema that has to be
    /// invalidated whenever a tab connects, disconnects or closes.
    fn nav_connections(&self) -> Vec<NavConnection> {
        self.connections
            .iter()
            .map(|connection| {
                let tab = self.tabs.iter().position(|tab| {
                    tab.title.as_deref() == Some(connection.name.as_str())
                        && matches!(tab.screen, ScreenSlot::Active(_))
                });
                let screen = tab.and_then(|index| match &self.tabs[index].screen {
                    ScreenSlot::Active(screen) => Some(screen),
                    ScreenSlot::ConnectionPicker => None,
                });
                NavConnection {
                    name: connection.name.clone(),
                    tab,
                    outline: screen.map(|s| s.outline()).unwrap_or_default(),
                    error: screen.and_then(|s| s.outline_error()),
                    alive: screen.and_then(|s| s.connection_alive()),
                }
            })
            .collect()
    }

    /// One key cycles the navigator through its three states: hidden,
    /// showing but not taking keys, and focused. Pressing it again from
    /// focused hides it, so the same key always gets you back to where you
    /// were.
    fn toggle_navigator(&mut self) {
        match (self.navigator_open, self.navigator_focused) {
            (false, _) => {
                self.navigator_open = true;
                self.navigator_focused = true;
            }
            (true, false) => self.navigator_focused = true,
            (true, true) => {
                self.navigator_open = false;
                self.navigator_focused = false;
            }
        }
    }

    /// Connects `connection` on a tab of its own -- reusing the active tab
    /// when it's still sitting on the picker, since an empty tab is exactly
    /// what a new connection wants.
    fn open_connection(&mut self, index: usize) -> Option<Action> {
        let connection = self.connections.get(index)?.clone();
        if matches!(self.tabs[self.active_tab].screen, ScreenSlot::Active(_)) {
            self.new_tab();
        }
        let tab = self.active_tab;
        // Go through the picker rather than building the request here: it
        // owns `connect_epoch`, and a request that doesn't match the epoch
        // the picker will compare against would be dropped on arrival.
        self.tabs[tab].connection_picker.selected = index;
        let action = self.tabs[tab]
            .connection_picker
            .handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        match action {
            Some(Action::OpenRequested {
                connection: _,
                epoch,
                ..
            }) => Some(Action::OpenRequested {
                connection,
                epoch,
                tab,
            }),
            _ => None,
        }
    }

    fn handle_navigator_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let connections = self.nav_connections();
        let key = KeyPress::new(code, modifiers);
        let command =
            match keymap().resolve_in(&[Context::Navigator, Context::List], &mut self.pending, key)
            {
                Resolution::Command(command) => command,
                Resolution::Pending => return None,
                // Anything the navigator doesn't claim hands focus back rather
                // than being swallowed -- a panel that eats every key you press
                // is a panel you can't get out of.
                Resolution::None => {
                    self.navigator_focused = false;
                    return None;
                }
            };

        if let Some(mv) = command.as_vim_move() {
            self.navigator.apply_move(mv, &connections);
            return None;
        }

        let outcome = match command {
            Command::Expand => self.navigator.expand(&connections),
            Command::Collapse => {
                self.navigator.collapse(&connections);
                None
            }
            Command::InsertName | Command::Open => self.navigator.choose(&connections),
            Command::ToggleNavigator => {
                self.toggle_navigator();
                None
            }
            Command::Back => {
                self.navigator_focused = false;
                None
            }
            Command::Help => return Some(Action::ShowHelp),
            _ => None,
        };

        match outcome? {
            NavOutcome::Open(index) => self.open_connection(index),
            NavOutcome::Focus(tab) => {
                self.active_tab = tab;
                self.navigator_focused = false;
                None
            }
            NavOutcome::Insert { tab, name } => {
                self.active_tab = tab;
                if let ScreenSlot::Active(screen) = &mut self.tabs[tab].screen {
                    screen.insert_text(&name);
                }
                self.navigator_focused = false;
                None
            }
        }
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
                Command::ToggleNavigator => self.toggle_navigator(),
                _ => {}
            }
            return None;
        }

        // The navigator takes the keys while it has focus, ahead of the
        // tab's own screen.
        if self.navigator_open && self.navigator_focused {
            return self.handle_navigator_key(code, modifiers);
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

    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        // The help overlay covers everything, so nothing behind it is a
        // legitimate click target.
        if self.help.is_some() {
            return None;
        }

        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && ui::contains(self.tab_bar_area, event.column, event.row)
        {
            if let Some(index) = self.tab_at(event.column) {
                self.active_tab = index;
            }
            return None;
        }

        if self.navigator_open && self.navigator.contains(event.column, event.row) {
            let connections = self.nav_connections();
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.navigator.click(event.column, event.row, &connections);
                    self.navigator_focused = true;
                }
                MouseEventKind::ScrollDown => {
                    self.navigator.apply_move(VimMove::Down, &connections)
                }
                MouseEventKind::ScrollUp => self.navigator.apply_move(VimMove::Up, &connections),
                _ => {}
            }
            return None;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            self.navigator_focused = false;
        }

        let tab = &mut self.tabs[self.active_tab];
        match &mut tab.screen {
            ScreenSlot::ConnectionPicker => tab.connection_picker.handle_mouse_event(event),
            ScreenSlot::Active(screen) => screen.handle_mouse_event(event),
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
            self.tab_bar_area = chunks[0];
            self.tab_label_widths = draw_tab_bar(frame, chunks[0], &self.tabs, self.active_tab);
            (chunks[1], chunks[2])
        } else {
            self.tab_bar_area = Rect::ZERO;
            self.tab_label_widths.clear();
            (chunks[0], chunks[1])
        };

        // The navigator takes a fixed column on the left, so the screen
        // beside it doesn't reflow every time a longer table name shows up.
        let screen_area = if self.navigator_open {
            let width = 30.min(content_area.width / 2);
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(width), Constraint::Min(1)])
                .split(content_area);
            let connections = self.nav_connections();
            self.navigator
                .draw(frame, split[0], self.navigator_focused, &connections);
            split[1]
        } else {
            content_area
        };

        let tab = &mut self.tabs[self.active_tab];
        match &mut tab.screen {
            ScreenSlot::ConnectionPicker => tab.connection_picker.draw(frame, screen_area),
            ScreenSlot::Active(screen) => screen.draw(frame, screen_area),
        }

        self.draw_status_bar(frame, status_area);

        // Drawn last so it sits above everything, including the tab bar.
        if let Some(help) = &mut self.help {
            help.draw(frame, area);
        }
    }
}

impl RootComponent {
    /// Which tab's label covers screen column `column`.
    fn tab_at(&self, column: u16) -> Option<usize> {
        let mut x = self.tab_bar_area.x;
        for (index, width) in self.tab_label_widths.iter().enumerate() {
            if column >= x && column < x + width {
                return Some(index);
            }
            x += width;
        }
        None
    }

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
            hints.extend(ui::hint(Context::QueryScreen, Command::RunQuery, "run"));
            hints.extend(ui::hint(Context::QueryScreen, Command::CycleFocus, "focus"));
            hints.extend(ui::hint(Context::QueryScreen, Command::History, "history"));
            hints.extend(ui::hint(Context::QueryScreen, Command::Back, "back"));
        }
        hints.extend(ui::hint(
            Context::Global,
            Command::ToggleNavigator,
            "navigator",
        ));
        hints.extend(ui::hint(Context::Global, Command::NewTab, "tab"));
        hints.extend(ui::hint(screen_context, Command::Help, "help"));

        let right = self.tabs[self.active_tab].title.clone();
        ui::draw_status_bar(frame, area, &hints, right.as_deref());
    }
}

/// Draws the bar and reports each tab label's width, so clicks can be
/// mapped back to a tab without recomputing the labels.
fn draw_tab_bar(frame: &mut Frame, area: Rect, tabs: &[Tab], active: usize) -> Vec<u16> {
    let theme = theme();
    let mut widths = Vec::with_capacity(tabs.len());
    let spans: Vec<Span> = tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let label = format!(" {}: {} ", i + 1, tab.label());
            widths.push(label.chars().count() as u16);
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
    widths
}

pub mod connection_form;
pub mod connection_picker;
pub mod navigator;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use tradar_core::storage::TabState;

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
    fn show_help_raises_the_overlay_and_the_next_key_dismisses_it() {
        let mut root = root();

        root.update(Action::ShowHelp);
        assert!(root.help.is_some());

        root.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(root.help.is_none());
    }

    #[test]
    fn the_help_overlay_swallows_keys_while_it_is_open() {
        let mut root = root();
        root.update(Action::ShowHelp);

        // A global binding that would otherwise open a tab.
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);

        assert_eq!(root.tabs.len(), 1, "ctrl-t must not reach the tab handling");
    }

    #[test]
    fn navigation_keys_scroll_the_help_overlay_without_closing_it() {
        let mut root = root();
        root.update(Action::ShowHelp);
        // `draw` gives the overlay a viewport height to scroll within.
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| root.draw(frame, frame.area()))
            .unwrap();

        root.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        assert!(root.help.is_some(), "j scrolls the overlay, not closes it");
    }

    #[test]
    fn draw_renders_a_status_bar_with_the_current_bindings() {
        let mut root = root();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| root.draw(frame, frame.area()))
            .unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        // The picker's own hints, plus the global ones -- taken from the
        // live keymap, not hardcoded here or there.
        assert!(text.contains("connect"), "buffer was: {text}");
        assert!(text.contains("ctrl-t"), "buffer was: {text}");
        assert!(text.contains("help"), "buffer was: {text}");
    }

    fn click_at(column: u16, row: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicking_the_tab_bar_switches_tabs() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(root.active_tab, 2);
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| root.draw(frame, frame.area()))
            .unwrap();

        // Labels are " 1: picker " etc, laid out left to right on row 0.
        root.handle_mouse_event(click_at(2, 0));

        assert_eq!(root.active_tab, 0);
    }

    #[test]
    fn a_click_past_the_last_tab_label_changes_nothing() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| root.draw(frame, frame.area()))
            .unwrap();
        assert_eq!(root.active_tab, 1);

        root.handle_mouse_event(click_at(79, 0));

        assert_eq!(root.active_tab, 1);
    }

    #[test]
    fn the_help_overlay_swallows_clicks_too() {
        let mut root = root();
        root.handle_key_event(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| root.draw(frame, frame.area()))
            .unwrap();
        root.update(Action::ShowHelp);

        root.handle_mouse_event(click_at(2, 0));

        assert_eq!(root.active_tab, 1, "the click must not reach the tab bar");
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
            tabs: vec![
                TabState::new("local-sqlite"),
                TabState::new("local-postgres"),
            ],
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
            tabs: vec![
                TabState::new("renamed-or-deleted"),
                TabState::new("local-sqlite"),
            ],
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
            tabs: vec![TabState::new("renamed-or-deleted")],
        });

        assert!(requests.is_empty());
        assert_eq!(root.tabs.len(), 1);
        assert_eq!(root.active_tab, 0);
        assert!(matches!(root.tabs[0].screen, ScreenSlot::ConnectionPicker));
    }

    /// A screen that reports one table, so the navigator has something to
    /// show under a connection, and records what got inserted into it.
    struct OutlineScreen {
        inserted: Rc<std::cell::RefCell<String>>,
    }

    impl Component for OutlineScreen {
        fn handle_key_event(&mut self, _code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
            None
        }
        fn update(&mut self, _action: Action) -> Option<Action> {
            None
        }
        fn outline(&self) -> Vec<tradar_core::action::OutlineEntry> {
            vec![tradar_core::action::OutlineEntry {
                depth: 0,
                label: "users".to_string(),
                detail: String::new(),
                has_children: false,
            }]
        }
        fn insert_text(&mut self, text: &str) {
            *self.inserted.borrow_mut() = text.to_string();
        }
        fn connection_alive(&self) -> Option<bool> {
            Some(true)
        }
        fn draw(&mut self, _frame: &mut Frame, _area: Rect) {}
    }

    /// A root with `local-sqlite` connected on tab 0 and the navigator up
    /// and focused.
    fn root_with_navigator() -> (RootComponent, Rc<std::cell::RefCell<String>>) {
        let mut root = root();
        let inserted = Rc::new(std::cell::RefCell::new(String::new()));
        root.tabs[0].screen = ScreenSlot::Active(Box::new(OutlineScreen {
            inserted: Rc::clone(&inserted),
        }));
        root.tabs[0].title = Some("local-sqlite".to_string());
        root.handle_key_event(KeyCode::Char('b'), KeyModifiers::CONTROL);
        (root, inserted)
    }

    #[test]
    fn the_navigator_key_cycles_hidden_focused_and_back() {
        let mut root = root();
        assert!(!root.navigator_open);

        root.handle_key_event(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert!(root.navigator_open && root.navigator_focused);

        root.handle_key_event(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert!(
            !root.navigator_open && !root.navigator_focused,
            "the same key has to get you back where you were"
        );
    }

    #[test]
    fn the_navigator_lists_every_saved_connection_not_just_the_open_one() {
        let (root, _) = root_with_navigator();

        let connections = root.nav_connections();

        assert_eq!(connections.len(), 2);
        assert_eq!(connections[0].tab, Some(0), "open on tab 0");
        assert_eq!(connections[1].tab, None, "never connected");
    }

    #[test]
    fn nav_connections_carries_the_open_screen_s_alive_status() {
        let (root, _) = root_with_navigator();

        let connections = root.nav_connections();

        assert_eq!(
            connections[0].alive,
            Some(true),
            "the open screen reports it"
        );
        assert_eq!(
            connections[1].alive, None,
            "nothing to be alive or dead without a tab"
        );
    }

    #[test]
    fn choosing_a_table_in_the_navigator_puts_it_into_that_tab_s_editor() {
        let (mut root, inserted) = root_with_navigator();
        // Open the connection, move onto its table, choose it.
        root.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);
        root.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(inserted.borrow().as_str(), "users");
        assert!(
            !root.navigator_focused,
            "focus goes back to where the text landed"
        );
    }

    #[test]
    fn opening_an_unconnected_connection_from_the_navigator_requests_a_connect() {
        let (mut root, _) = root_with_navigator();
        root.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);

        let action = root.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        let Some(Action::OpenRequested {
            connection, tab, ..
        }) = action
        else {
            panic!("expected a connect request");
        };
        assert_eq!(connection.name, "local-postgres");
        assert_eq!(tab, 1, "the connected tab 0 is left alone");
        assert_eq!(root.tabs.len(), 2);
    }

    #[test]
    fn a_key_the_navigator_does_not_claim_hands_focus_back_to_the_screen() {
        let (mut root, _) = root_with_navigator();

        root.handle_key_event(KeyCode::Char('z'), KeyModifiers::NONE);

        assert!(!root.navigator_focused, "must not trap the keys");
        assert!(root.navigator_open, "but the panel itself stays up");
    }
}
