//! The connection-picker screen: list saved connections, select one,
//! request a connect. Implements `Component` because `RootComponent`
//! routes keys to it directly whenever it's the active screen.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list::{self, VimMove};

use crate::components::connection_form::{ConnectionFormComponent, FormMode, FormOutcome};

/// A second `Down(Left)` on the same row inside this window counts as a
/// double-click (open) rather than two separate single clicks (select).
/// No platform-standard value exists in a terminal; 500ms matches what
/// most desktop environments default to.
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);

pub struct ConnectionPickerComponent {
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub last_error: Option<String>,
    pub connect_epoch: u64,
    /// The connection currently being connected to, if any -- `main.rs`'s
    /// connect timeout means this never sits unresolved for more than a
    /// few seconds, but without *some* feedback the picker looks identical
    /// whether a connect is in flight or nothing has happened at all.
    /// Cleared by `RootComponent` once the matching `Opened`/`OpenFailed`
    /// lands (see its `update`).
    pub connecting: Option<String>,
    /// Which tab (if any) already has each connection open, parallel to
    /// `connections` -- refreshed by `RootComponent` right before every
    /// `draw`, since only it knows about tabs. Drives both the "already
    /// open" badge and the right-click menu's wording; empty (the default,
    /// including in every test that doesn't call `set_open_status`) just
    /// means nothing is known to be open, which is a safe thing to assume.
    open_in_tab: Vec<Option<usize>>,
    /// Half-finished two-key binding (the first `g` of `gg`), owned here
    /// because it's per-list state -- see `tradar_core::keymap`.
    pending: Option<KeyPress>,
    /// Kept between frames so the scroll offset survives, which is what
    /// makes a click land on the row that was actually pointed at.
    list_state: ListState,
    list_area: Rect,
    visible_height: usize,
    /// The add/edit form, when open. It takes over key handling entirely.
    form: Option<ConnectionFormComponent>,
    /// Set while a delete is waiting to be confirmed -- deleting a
    /// connection is one keystroke otherwise, and it edits a file.
    confirming_delete: bool,
    /// Right-click menu: connect/switch, open a new session, edit, delete
    /// -- whatever's relevant to the row that was clicked. See
    /// `dispatch_command`'s doc comment for why confirming a menu item runs
    /// through the exact same code a keyboard shortcut would.
    context_menu: Option<ui::ContextMenu>,
    /// The row and time of the last `Down(Left)`, to recognize a second one
    /// on the same row within `DOUBLE_CLICK_WINDOW` as "open" rather than
    /// "select (again)".
    last_click: Option<(usize, Instant)>,
    /// Connector ids compiled into this build, for the form's driver
    /// picker. Comes from `main.rs`'s registry via `RootComponent`.
    drivers: Vec<String>,
    /// Where edits are persisted. Every change writes the whole file
    /// immediately: the alternative is an unsaved-changes state to explain
    /// and get wrong.
    store: Option<ConnectionStore>,
}

impl ConnectionPickerComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected: 0,
            last_error: None,
            connect_epoch: 0,
            connecting: None,
            open_in_tab: Vec::new(),
            pending: None,
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            visible_height: 0,
            form: None,
            confirming_delete: false,
            context_menu: None,
            last_click: None,
            drivers: Vec::new(),
            store: None,
        }
    }

    /// Called by `RootComponent` right before every `draw`, since it's the
    /// only thing that knows what's open in which tab. `status` is parallel
    /// to `connections` -- `Some(tab)` when that connection already has an
    /// active tab, `None` when it doesn't.
    pub fn set_open_status(&mut self, status: Vec<Option<usize>>) {
        self.open_in_tab = status;
    }

    /// Wires up editing: without a driver list and a store the picker is
    /// read-only, which is what the tests that don't care about editing
    /// (and any future embedding) get.
    pub fn with_editing(mut self, drivers: Vec<String>, store: ConnectionStore) -> Self {
        self.drivers = drivers;
        self.store = Some(store);
        self
    }

    /// Writes the whole connection list back to disk, surfacing a failure
    /// in the picker's own error box rather than swallowing it -- a save
    /// that silently didn't happen is the worst outcome here.
    fn persist(&mut self) {
        let Some(store) = &self.store else { return };
        if let Err(e) = store.save(&self.connections) {
            self.last_error = Some(format!("could not save connections: {e}"));
        }
    }

    fn open_form(&mut self, mode: FormMode) {
        if self.drivers.is_empty() {
            return;
        }
        let existing = match mode {
            FormMode::Add => None,
            FormMode::Edit(index) => self.connections.get(index),
        };
        self.form = Some(ConnectionFormComponent::new(
            mode,
            self.drivers.clone(),
            existing,
        ));
    }

    fn apply_form_outcome(&mut self, outcome: FormOutcome) {
        match outcome {
            FormOutcome::Cancelled => self.form = None,
            FormOutcome::Saved { connection, mode } => {
                match mode {
                    FormMode::Add => {
                        self.connections.push(connection);
                        self.selected = self.connections.len() - 1;
                    }
                    FormMode::Edit(index) => {
                        if let Some(slot) = self.connections.get_mut(index) {
                            *slot = connection;
                        }
                    }
                }
                self.form = None;
                self.persist();
            }
        }
    }

    fn delete_selected(&mut self) {
        if self.selected >= self.connections.len() {
            return;
        }
        self.connections.remove(self.selected);
        if self.selected >= self.connections.len() {
            self.selected = self.connections.len().saturating_sub(1);
        }
        self.persist();
    }

    fn apply_move(&mut self, mv: VimMove) {
        vim_list::apply(
            mv,
            &mut self.selected,
            self.connections.len(),
            self.visible_height,
        );
    }

    fn open_selected(&mut self, force_new: bool) -> Option<Action> {
        let connection = self.connections.get(self.selected).cloned()?;
        self.connect_epoch += 1;
        // A stale error from a previous failed attempt must not keep
        // showing once a new attempt is underway.
        self.last_error = None;
        self.connecting = Some(connection.name.clone());
        Some(Action::OpenRequested {
            connection,
            epoch: self.connect_epoch,
            // Placeholder -- RootComponent overwrites this with the real tab
            // index right after this call returns, since a lone picker has
            // no notion of which tab it belongs to.
            tab: 0,
            force_new,
        })
    }

    /// The list's inner area (inside its border), for hit-testing a click.
    fn inner_list_area(&self) -> Rect {
        Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        }
    }

    /// Runs whatever `command` means for this screen -- shared by keyboard
    /// dispatch (`handle_key_event`) and a right-click context menu's
    /// confirmed choice (`handle_mouse_event`), so a menu item runs through
    /// the exact same code a keyboard shortcut for it would, not a second
    /// copy that can drift out of sync.
    fn dispatch_command(&mut self, command: Command) -> Option<Action> {
        match command {
            Command::Quit => Some(Action::Quit),
            Command::Open => self.open_selected(false),
            Command::OpenNewSession => self.open_selected(true),
            Command::Help => Some(Action::ShowHelp),
            Command::NewConnection => {
                self.open_form(FormMode::Add);
                None
            }
            Command::EditConnection => {
                if self.selected < self.connections.len() {
                    self.open_form(FormMode::Edit(self.selected));
                }
                None
            }
            Command::DeleteConnection => {
                if self.store.is_some() && self.selected < self.connections.len() {
                    self.confirming_delete = true;
                }
                None
            }
            _ => None,
        }
    }
}

impl Component for ConnectionPickerComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(form) = self.form.as_mut() {
            if let Some(outcome) = form.handle_key_event(code, modifiers) {
                self.apply_form_outcome(outcome);
            }
            return None;
        }

        if let Some(menu) = self.context_menu.as_mut() {
            match menu.handle_key_event(code) {
                ui::ContextMenuOutcome::Open => {}
                ui::ContextMenuOutcome::Closed => self.context_menu = None,
                ui::ContextMenuOutcome::Confirmed(command) => {
                    self.context_menu = None;
                    return self.dispatch_command(command);
                }
            }
            return None;
        }

        // A pending delete answers the next key and nothing else, so the
        // confirmation can't be dismissed by accident into a deletion.
        if self.confirming_delete {
            self.confirming_delete = false;
            if matches!(code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                self.delete_selected();
            }
            return None;
        }

        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Picker, Context::List], &mut self.pending, key)
        else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            self.apply_move(mv);
            return None;
        }

        self.dispatch_command(command)
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        // A context menu is its own small overlay: a left click either hits
        // one of its items or dismisses it (clicking away closes a popup,
        // standard behavior) -- either way nothing behind it should also
        // react to the same click.
        if let Some(menu) = self.context_menu.take() {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(command) = menu.click(self.list_area, event.column, event.row)
            {
                return self.dispatch_command(command);
            }
            return None;
        }

        // The form and the delete confirmation are modal: a click behind
        // them would act on a list the user can't see.
        if self.form.is_some() || self.confirming_delete {
            return None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = self.inner_list_area();
                if let Some(index) = ui::index_at(
                    inner,
                    self.list_state.offset(),
                    event.row,
                    self.connections.len(),
                ) {
                    let now = Instant::now();
                    let is_double_click = self.last_click.is_some_and(|(prev, at)| {
                        prev == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                    });
                    self.selected = index;
                    if is_double_click {
                        self.last_click = None;
                        return self.dispatch_command(Command::Open);
                    }
                    self.last_click = Some((index, now));
                }
                None
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let inner = self.inner_list_area();
                if let Some(index) = ui::index_at(
                    inner,
                    self.list_state.offset(),
                    event.row,
                    self.connections.len(),
                ) {
                    self.selected = index;
                    let mut items = Vec::new();
                    match self.open_in_tab.get(index).copied().flatten() {
                        Some(tab) => {
                            items.push((format!("Switch to tab {}", tab + 1), Command::Open));
                            items.push(("Open new session".to_string(), Command::OpenNewSession));
                        }
                        None => items.push(("Connect".to_string(), Command::Open)),
                    }
                    if self.store.is_some() {
                        items.push(("Edit connection".to_string(), Command::EditConnection));
                        items.push(("Delete connection".to_string(), Command::DeleteConnection));
                    }
                    self.context_menu =
                        Some(ui::ContextMenu::new((event.column, event.row), items));
                }
                None
            }
            MouseEventKind::ScrollDown => {
                self.apply_move(VimMove::Down);
                None
            }
            MouseEventKind::ScrollUp => {
                self.apply_move(VimMove::Up);
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if let Action::OpenFailed { error, .. } = action {
            self.last_error = Some(error);
            self.connecting = None;
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
            .enumerate()
            .map(|(index, connection)| {
                let mut spans = vec![
                    Span::styled(
                        format!("  {}", connection.name),
                        Style::default().fg(theme.text),
                    ),
                    Span::styled(
                        format!("  {}", connection.driver),
                        Style::default().fg(theme.text_dim),
                    ),
                ];
                // Quiet when it's only open here or nowhere -- a badge on
                // every row would just be noise; this calls out the one
                // case that matters, that opening it again would land on an
                // *existing* session elsewhere rather than a fresh one.
                if let Some(Some(tab)) = self.open_in_tab.get(index) {
                    spans.push(Span::styled(
                        format!("  ● tab {}", tab + 1),
                        Style::default().fg(theme.accent),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        if self.connections.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.selected));
        }
        self.visible_height = list_area.height.saturating_sub(2) as usize;
        self.list_area = list_area;
        let title = if let Some(name) = &self.connecting {
            // Without this, the picker looks identical whether a connect
            // is in flight or nothing has happened -- pressing Enter and
            // seeing no change for up to the 5s connect timeout reads as a
            // hung app, not a working one.
            format!("Connections — connecting to '{name}'…")
        } else if self.drivers.is_empty() {
            "Connections".to_string()
        } else {
            // Spell the editing keys out here: without a hint, an
            // add-connection screen nobody can find is the same as not
            // having one.
            let key = |command| {
                keymap()
                    .binding_for(Context::Picker, command)
                    .unwrap_or_default()
            };
            format!(
                "Connections — {} add, {} edit, {} delete",
                key(Command::NewConnection),
                key(Command::EditConnection),
                key(Command::DeleteConnection),
            )
        };
        let list = List::new(items)
            .block(ui::panel(&title, true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        if let (Some(error_area), Some(error)) = (error_area, &self.last_error) {
            let error_box = Paragraph::new(Span::styled(
                error.as_str(),
                Style::default().fg(theme.error),
            ))
            .wrap(Wrap { trim: true })
            .block(ui::panel("Error", false));
            frame.render_widget(error_box, error_area);
        }

        if self.confirming_delete {
            let name = self
                .connections
                .get(self.selected)
                .map(|c| c.name.as_str())
                .unwrap_or_default();
            let popup = ui::centered_rect(60, 20, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("Delete '{name}'? "),
                        Style::default().fg(theme.text),
                    ),
                    Span::styled(
                        "y",
                        Style::default()
                            .fg(theme.error)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " to confirm, any other key to cancel",
                        Style::default().fg(theme.text_dim),
                    ),
                ]))
                .wrap(Wrap { trim: true })
                .block(
                    ui::panel("Confirm delete", true)
                        .border_style(Style::default().fg(theme.warning)),
                ),
                popup,
            );
        }

        // Drawn last so it covers the list it's editing.
        if let Some(form) = &self.form {
            let popup = ui::centered_rect(70, 40, area);
            frame.render_widget(ratatui::widgets::Clear, popup);
            form.draw(frame, popup);
        }

        // Drawn last of all so it sits above everything, including the form.
        if let Some(menu) = &self.context_menu {
            menu.draw(frame, list_area);
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
    fn opening_a_connection_shows_a_connecting_indicator() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE); // local-postgres

        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(picker.connecting.as_deref(), Some("local-postgres"));
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("connecting to 'local-postgres'"),
            "buffer was: {text}"
        );
    }

    #[test]
    fn a_failed_connect_clears_the_connecting_indicator() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        assert!(picker.connecting.is_some());

        picker.update(Action::OpenFailed {
            error: "connection refused".to_string(),
            epoch: 1,
            tab: 0,
        });

        assert_eq!(picker.connecting, None);
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

    /// A picker wired for editing against a throwaway connections file.
    /// The `TempDir` is returned so it outlives the test.
    fn editable_picker() -> (
        ConnectionPickerComponent,
        tempfile::TempDir,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.toml");
        let picker = ConnectionPickerComponent::new(connections()).with_editing(
            vec!["postgres".to_string(), "sqlite".to_string()],
            ConnectionStore::at(path.clone()),
        );
        (picker, dir, path)
    }

    fn type_str(picker: &mut ConnectionPickerComponent, text: &str) {
        for c in text.chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn adding_a_connection_appends_it_selects_it_and_saves_the_file() {
        let (mut picker, _dir, path) = editable_picker();

        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut picker, "new one");
        picker.handle_key_event(KeyCode::Tab, KeyModifiers::NONE); // driver
        picker.handle_key_event(KeyCode::Right, KeyModifiers::NONE); // -> sqlite
        picker.handle_key_event(KeyCode::Tab, KeyModifiers::NONE); // target
        type_str(&mut picker, "new.db");
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(picker.connections.len(), 3);
        assert_eq!(picker.connections[2].name, "new one");
        assert_eq!(picker.connections[2].driver, "sqlite");
        assert_eq!(picker.selected, 2, "the new connection is selected");
        assert!(picker.form.is_none(), "the form closes on save");

        let saved = ConnectionStore::at(path).load().unwrap();
        assert_eq!(saved, picker.connections, "the file is written immediately");
    }

    #[test]
    fn cancelling_the_form_changes_nothing() {
        let (mut picker, _dir, path) = editable_picker();

        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        type_str(&mut picker, "discarded");
        picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(picker.form.is_none());
        assert_eq!(picker.connections, connections());
        assert!(!path.exists(), "nothing should have been written");
    }

    #[test]
    fn editing_replaces_the_selected_connection_in_place() {
        let (mut picker, _dir, path) = editable_picker();
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Char('e'), KeyModifiers::NONE);
        // The name field starts filled, with the cursor at the end.
        type_str(&mut picker, "-renamed");
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(picker.connections.len(), 2, "editing must not append");
        assert_eq!(picker.connections[1].name, "local-postgres-renamed");
        assert_eq!(
            picker.connections[1].target, "postgres://localhost/test",
            "untouched fields survive"
        );
        let saved = ConnectionStore::at(path).load().unwrap();
        assert_eq!(saved, picker.connections);
    }

    #[test]
    fn deleting_asks_first_and_only_y_goes_through() {
        let (mut picker, _dir, path) = editable_picker();

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(picker.confirming_delete);

        // Any other key backs out.
        picker.handle_key_event(KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(!picker.confirming_delete);
        assert_eq!(picker.connections.len(), 2, "nothing deleted");

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(picker.connections.len(), 1);
        assert_eq!(picker.connections[0].name, "local-postgres");
        let saved = ConnectionStore::at(path).load().unwrap();
        assert_eq!(saved, picker.connections);
    }

    #[test]
    fn deleting_the_last_connection_keeps_the_selection_in_range() {
        let (mut picker, _dir, _path) = editable_picker();
        picker.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(picker.selected, 1);

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(picker.connections.len(), 1);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn a_picker_without_a_store_stays_read_only() {
        let mut picker = ConnectionPickerComponent::new(connections());

        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(picker.form.is_none(), "no drivers to offer, so no form");

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(!picker.confirming_delete, "nowhere to save a deletion");
    }

    #[test]
    fn keys_go_to_the_form_while_it_is_open() {
        let (mut picker, _dir, _path) = editable_picker();
        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);

        // `j` would move the selection with the form closed; here it's text.
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        assert_eq!(picker.selected, 0, "the list must not move under the form");
    }

    fn click_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        // Row 0 is the border; the two connections are on rows 1 and 2.
        picker.handle_mouse_event(click_at(5, 2));

        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn clicking_empty_space_below_the_list_keeps_the_selection() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(5, 6));

        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn double_clicking_a_row_opens_it() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(5, 2));
        let second = picker.handle_mouse_event(click_at(5, 2));

        match second {
            Some(Action::OpenRequested {
                connection,
                force_new,
                ..
            }) => {
                assert_eq!(connection.name, "local-postgres");
                assert!(
                    !force_new,
                    "a plain double-click must not force a new session"
                );
            }
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn two_single_clicks_on_the_same_row_far_apart_do_not_open_it() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(5, 2));
        // Backdate the recorded click past the double-click window, standing
        // in for real time having passed between two separate single clicks.
        picker.last_click = picker
            .last_click
            .map(|(index, at)| (index, at - DOUBLE_CLICK_WINDOW));
        let second = picker.handle_mouse_event(click_at(5, 2));

        assert!(second.is_none());
        assert_eq!(picker.selected, 1);
    }

    #[test]
    fn ctrl_enter_requests_a_new_session_even_though_open_selected_normally_would_not() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::CONTROL);

        match action {
            Some(Action::OpenRequested { force_new, .. }) => assert!(force_new),
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn plain_enter_requests_open_without_forcing_a_new_session() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match action {
            Some(Action::OpenRequested { force_new, .. }) => assert!(!force_new),
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    fn right_click_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn right_clicking_a_connection_not_open_elsewhere_offers_only_connect() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(right_click_at(5, 1));
        assert!(picker.context_menu.is_some());
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Connect"), "buffer was: {text}");
        assert!(!text.contains("Switch to tab"), "buffer was: {text}");
        assert!(!text.contains("Open new session"), "buffer was: {text}");

        // The one item present is `Command::Open`, not forcing a new session.
        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        match action {
            Some(Action::OpenRequested { force_new, .. }) => assert!(!force_new),
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }

    #[test]
    fn right_clicking_a_connection_open_elsewhere_offers_switch_and_a_new_session() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.set_open_status(vec![None, Some(2)]);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        // Row 2 is "local-postgres" (index 1), the one marked open on tab 2.
        picker.handle_mouse_event(right_click_at(5, 2));
        assert!(picker.context_menu.is_some());
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Switch to tab 3"), "buffer was: {text}");
        assert!(text.contains("Open new session"), "buffer was: {text}");
    }

    #[test]
    fn confirming_open_new_session_from_the_context_menu_dispatches_it() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.set_open_status(vec![None, Some(2)]);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();
        picker.handle_mouse_event(right_click_at(5, 2));

        // "Switch to tab 3" is first (index 0); move down to "Open new
        // session" and confirm it.
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match action {
            Some(Action::OpenRequested {
                connection,
                force_new,
                ..
            }) => {
                assert_eq!(connection.name, "local-postgres");
                assert!(force_new);
            }
            other => panic!(
                "expected OpenRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
        assert!(
            picker.context_menu.is_none(),
            "the menu should close once a choice is confirmed"
        );
    }

    #[test]
    fn draw_shows_the_already_open_badge_only_for_the_connection_that_has_one() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.set_open_status(vec![None, Some(2)]);
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("tab 3"), "buffer was: {text}");
        // `buffer_text` flattens every cell into one string with no row
        // separators, so isolating "sqlite's row has no badge" means
        // counting rather than searching a single line: exactly one
        // connection (local-postgres, marked open) should carry a badge.
        assert_eq!(
            text.matches("● tab").count(),
            1,
            "expected exactly one badge, buffer was: {text}"
        );
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
