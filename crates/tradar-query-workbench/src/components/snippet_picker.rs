//! The saved-snippet library overlay: browse, insert, rename, or delete a
//! named query the user chose to keep. Not a `Component` -- driven
//! directly by `QueryScreenComponent`, the same way `HistoryPickerComponent`
//! is, and takes over all key input while open. Scoped to one driver at a
//! time (the current connection's) via `tradar_core::storage::Snippets`,
//! the process-global store this reads from and writes back to directly --
//! deletion/rename don't need to round-trip through the caller the way
//! `Insert`/`Cancelled` do, since there's nothing for `QueryScreenComponent`
//! to react to beyond "the list changed".

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedSnippet;
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker, TextInput};
use tradar_core::vim_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetOutcome {
    Insert(String),
    Cancelled,
}

pub struct SnippetPickerComponent {
    driver: String,
    entries: Vec<SavedSnippet>,
    selected: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
    /// Set while a delete is waiting to be confirmed -- same idiom as
    /// `ConnectionPickerComponent::confirming_delete`: deleting is one
    /// keystroke otherwise, and it edits a file.
    confirming_delete: bool,
    /// `Some` while renaming the selected entry, prefilled with its
    /// current name. Takes every key until `Enter`/`Esc`, same as any
    /// other inline text field in this app.
    renaming: Option<TextInput>,
    list_state: ListState,
    list_area: Rect,
    double_click: DoubleClickTracker,
    /// Right-click menu: Insert/Rename/Delete on the row that was clicked.
    /// See `ConnectionPickerComponent::context_menu`'s doc comment for why
    /// confirming a menu item runs through the exact same `dispatch_command`
    /// a keyboard shortcut would.
    context_menu: Option<ui::ContextMenu>,
}

impl SnippetPickerComponent {
    /// `driver` scopes delete/rename to the right connector's snippets --
    /// see the module doc comment. `entries` is supplied rather than
    /// pulled from the global store here, the same way
    /// `HistoryPickerComponent::new` takes its list as a parameter: the
    /// caller already has to read `snippets()` to know whether there's
    /// anything to show, and it keeps this constructor usable in a test
    /// without the process-global store initialized.
    pub fn new(driver: impl Into<String>, entries: Vec<SavedSnippet>) -> Self {
        Self {
            driver: driver.into(),
            entries,
            selected: 0,
            pending: None,
            visible_height: 0,
            confirming_delete: false,
            renaming: None,
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            double_click: DoubleClickTracker::new(),
            context_menu: None,
        }
    }

    fn refresh(&mut self) {
        self.entries = tradar_core::storage::snippets()
            .map(|s| s.for_driver(&self.driver))
            .unwrap_or_default();
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn selected_entry(&self) -> Option<&SavedSnippet> {
        self.entries.get(self.selected)
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<SnippetOutcome> {
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

        if let Some(input) = self.renaming.as_mut() {
            match code {
                KeyCode::Esc => self.renaming = None,
                KeyCode::Enter => {
                    let new_name = input.text().trim().to_string();
                    self.renaming = None;
                    if let (Some(store), Some(old_name)) = (
                        tradar_core::storage::snippets(),
                        self.selected_entry().map(|e| e.name.clone()),
                    ) && !new_name.is_empty()
                    {
                        store.rename(&self.driver, &old_name, new_name);
                        self.refresh();
                    }
                }
                _ => {
                    input.handle_key_event(code, modifiers);
                }
            }
            return None;
        }

        // A pending delete answers the next key and nothing else, so the
        // confirmation can't be dismissed by accident into a deletion.
        if self.confirming_delete {
            self.confirming_delete = false;
            if matches!(code, KeyCode::Char('y') | KeyCode::Char('Y'))
                && let (Some(store), Some(name)) = (
                    tradar_core::storage::snippets(),
                    self.selected_entry().map(|e| e.name.clone()),
                )
            {
                store.delete(&self.driver, &name);
                self.refresh();
            }
            return None;
        }

        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Snippets, Context::List], &mut self.pending, key)
        else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            vim_list::apply(
                mv,
                &mut self.selected,
                self.entries.len(),
                self.visible_height,
            );
            return None;
        }

        self.dispatch_command(command)
    }

    /// Runs whatever `command` means here -- shared by keyboard dispatch
    /// (`handle_key_event`) and a right-click context menu's confirmed
    /// choice (`handle_mouse_event`), so a menu item runs through the exact
    /// same code a keyboard shortcut for it would.
    fn dispatch_command(&mut self, command: Command) -> Option<SnippetOutcome> {
        match command {
            Command::Cancel => Some(SnippetOutcome::Cancelled),
            Command::Confirm => self
                .selected_entry()
                .map(|entry| SnippetOutcome::Insert(entry.text.clone())),
            Command::DeleteSnippet => {
                if self.selected_entry().is_some() {
                    self.confirming_delete = true;
                }
                None
            }
            Command::RenameSnippet => {
                if let Some(entry) = self.selected_entry() {
                    self.renaming = Some(TextInput::new(&entry.name));
                }
                None
            }
            _ => None,
        }
    }

    /// A left click selects the row it landed on, or confirms a menu item
    /// if the right-click menu is open; a second left click on the same row
    /// within the double-click window inserts it, same as `Enter`. Right
    /// click opens the menu (Insert/Rename/Delete) on the row it landed on.
    pub fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<SnippetOutcome> {
        if let Some(menu) = self.context_menu.take() {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(command) = menu.click(self.list_area, event.column, event.row)
            {
                return self.dispatch_command(command);
            }
            return None;
        }
        if self.renaming.is_some() || self.confirming_delete {
            return None;
        }
        let inner = Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        };
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let index = ui::index_at(
                    inner,
                    self.list_state.offset(),
                    event.row,
                    self.entries.len(),
                )?;
                self.selected = index;
                if self.double_click.click(index) {
                    return self.dispatch_command(Command::Confirm);
                }
                None
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let index = ui::index_at(
                    inner,
                    self.list_state.offset(),
                    event.row,
                    self.entries.len(),
                )?;
                self.selected = index;
                let items = vec![
                    ("Insert".to_string(), Command::Confirm),
                    ("Rename".to_string(), Command::RenameSnippet),
                    ("Delete".to_string(), Command::DeleteSnippet),
                ];
                self.context_menu = Some(ui::ContextMenu::new((event.column, event.row), items));
                None
            }
            MouseEventKind::ScrollDown => {
                vim_list::apply(
                    vim_list::VimMove::Down,
                    &mut self.selected,
                    self.entries.len(),
                    self.visible_height,
                );
                None
            }
            MouseEventKind::ScrollUp => {
                vim_list::apply(
                    vim_list::VimMove::Up,
                    &mut self.selected,
                    self.entries.len(),
                    self.visible_height,
                );
                None
            }
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let (list_area, rename_bar_area) = if self.renaming.is_some() {
            let (list_area, bar) = ui::split_bottom_bar(area, 1);
            (list_area, Some(bar))
        } else {
            (area, None)
        };

        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {}", entry.name), Style::default().fg(theme.text)),
                    Span::styled(
                        format!("  {}", entry.text.replace('\n', " ⏎ ")),
                        Style::default().fg(theme.text_dim),
                    ),
                ]))
            })
            .collect();

        self.list_area = list_area;
        if !self.entries.is_empty() {
            self.list_state.select(Some(self.selected));
        }

        self.visible_height = list_area.height.saturating_sub(2) as usize;
        let confirm = keymap()
            .binding_for(Context::Snippets, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Snippets, Command::Cancel)
            .unwrap_or_default();
        let delete = keymap()
            .binding_for(Context::Snippets, Command::DeleteSnippet)
            .unwrap_or_default();
        let rename = keymap()
            .binding_for(Context::Snippets, Command::RenameSnippet)
            .unwrap_or_default();
        let title = if self.confirming_delete {
            format!("Snippets — delete '{}'? y/N", self.selected_name())
        } else {
            format!(
                "Snippets — {confirm} insert, {rename} rename, {delete} delete, {cancel} cancel"
            )
        };
        let list = List::new(items)
            .block(ui::panel(&title, true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        if let (Some(bar_area), Some(input)) = (rename_bar_area, &self.renaming) {
            let mut spans = vec![Span::styled("rename: ", Style::default().fg(theme.accent))];
            spans.extend(input.spans(true));
            frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
        }

        if let Some(menu) = &self.context_menu {
            menu.draw(frame, list_area);
        }
    }

    fn selected_name(&self) -> &str {
        self.selected_entry().map_or("", |e| e.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn click_at(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn entry(name: &str, text: &str) -> SavedSnippet {
        SavedSnippet {
            name: name.to_string(),
            driver: "postgres".to_string(),
            text: text.to_string(),
        }
    }

    fn picker() -> SnippetPickerComponent {
        SnippetPickerComponent::new(
            "postgres",
            vec![
                entry("active-users", "SELECT * FROM users WHERE active;"),
                entry("count-orders", "SELECT count(*) FROM orders;"),
            ],
        )
    }

    #[test]
    fn starts_selecting_the_first_entry() {
        let picker = picker();

        assert_eq!(
            picker.selected_entry().map(|e| e.name.as_str()),
            Some("active-users")
        );
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_entry() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(
            picker.selected_entry().map(|e| e.name.as_str()),
            Some("count-orders")
        );

        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(
            picker.selected_entry().map(|e| e.name.as_str()),
            Some("count-orders")
        );
    }

    #[test]
    fn enter_inserts_the_selected_entry_s_text() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(SnippetOutcome::Insert(
                "SELECT * FROM users WHERE active;".to_string()
            ))
        );
    }

    #[test]
    fn enter_on_an_empty_library_is_a_no_op() {
        let mut picker = SnippetPickerComponent::new("postgres", Vec::new());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None);
    }

    #[test]
    fn esc_cancels() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(outcome, Some(SnippetOutcome::Cancelled));
    }

    #[test]
    fn d_then_a_non_y_key_cancels_the_delete_without_changing_the_list() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(picker.confirming_delete);
        picker.handle_key_event(KeyCode::Char('n'), KeyModifiers::NONE);

        assert!(!picker.confirming_delete);
        assert_eq!(picker.entries.len(), 2, "nothing was deleted");
    }

    #[test]
    fn d_then_y_with_no_global_store_initialized_does_not_panic_or_change_the_list() {
        // `tradar_core::storage::snippets()` is never initialized in tests
        // (same limitation `FilePickerComponent` already has for
        // `query_files()`) -- confirming a delete with nowhere to persist
        // it must be a graceful no-op, not a panic.
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(picker.entries.len(), 2);
    }

    #[test]
    fn r_opens_a_rename_prompt_prefilled_with_the_current_name() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        assert_eq!(
            picker.renaming.as_ref().map(|input| input.text()),
            Some("active-users".to_string())
        );
    }

    #[test]
    fn esc_while_renaming_closes_the_prompt_without_confirming() {
        let mut picker = picker();
        picker.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(picker.renaming.is_none());
        assert_eq!(
            picker.selected_entry().map(|e| e.name.as_str()),
            Some("active-users"),
            "no rename should have been applied"
        );
    }

    #[test]
    fn typing_while_renaming_does_not_leak_into_movement_or_other_commands() {
        let mut picker = picker();
        picker.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        // `j`/`d` would otherwise move the selection or ask to delete --
        // while renaming, every key belongs to the text field.
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        assert_eq!(
            picker.renaming.as_ref().map(|input| input.text()),
            Some("active-usersjd".to_string())
        );
    }

    #[test]
    fn clicking_a_row_selects_it_without_inserting_it() {
        let mut picker = picker();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        // Row 0 is the border, row 2 is "count-orders" (the second entry).
        let outcome =
            picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Left), 2, 2));

        assert_eq!(outcome, None);
        assert_eq!(
            picker.selected_entry().map(|e| e.name.as_str()),
            Some("count-orders")
        );
    }

    #[test]
    fn double_clicking_a_row_inserts_it_same_as_enter() {
        let mut picker = picker();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Left), 2, 2));
        let outcome =
            picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Left), 2, 2));

        assert_eq!(
            outcome,
            Some(SnippetOutcome::Insert(
                "SELECT count(*) FROM orders;".to_string()
            ))
        );
    }

    #[test]
    fn right_clicking_a_row_opens_a_menu_with_insert_rename_delete() {
        let mut picker = picker();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Right), 2, 1));
        assert!(picker.context_menu.is_some());
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Insert"), "buffer was: {text}");
        assert!(text.contains("Rename"), "buffer was: {text}");
        assert!(text.contains("Delete"), "buffer was: {text}");
    }

    #[test]
    fn confirming_delete_from_the_context_menu_asks_for_confirmation_first() {
        let mut picker = picker();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();
        picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Right), 2, 1));

        // "Insert" (0), "Rename" (1), "Delete" (2) -- move down twice.
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None, "delete must not run without a y/N confirm");
        assert!(picker.confirming_delete);
        assert_eq!(picker.entries.len(), 2, "nothing deleted yet");
    }

    #[test]
    fn confirming_rename_from_the_context_menu_opens_the_rename_prompt() {
        let mut picker = picker();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();
        picker.handle_mouse_event(click_at(MouseEventKind::Down(MouseButton::Right), 2, 1));

        // "Rename" is the second item (index 1).
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            picker.renaming.as_ref().map(|input| input.text()),
            Some("active-users".to_string())
        );
        assert!(
            picker.context_menu.is_none(),
            "the menu should close once a choice is confirmed"
        );
    }
}
