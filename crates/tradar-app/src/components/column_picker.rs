//! The navigator's column picker: a checkbox multi-select overlay opened by
//! `c`/`r`/`u`/`d` on a table/collection/index row, letting the user narrow
//! the generated CRUD snippet to specific columns before it's inserted --
//! see `Component::crud_snippet`. Not a `Component`: like the snippet
//! library overlay, it's driven directly by its host (`NavigatorComponent`),
//! which owns focus and steals every key while this is open.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};

use tradar_core::action::CrudOp;
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list;

/// One column offered by the picker: its name, and whether it's part of
/// the table's primary key -- dimmed differently, and what decides each
/// op's default checked set (see `ColumnPickerComponent::new`).
#[derive(Debug, Clone)]
pub struct PickerColumn {
    pub name: String,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnPickerOutcome {
    /// Confirmed with this column selection, in table order. Empty is a
    /// real, meaningful value here for Read (not "nothing happened") --
    /// see `can_confirm`.
    Confirmed(Vec<String>),
    Cancelled,
}

pub struct ColumnPickerComponent {
    op: CrudOp,
    columns: Vec<PickerColumn>,
    checked: HashSet<usize>,
    selected: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
    list_area: Rect,
    list_state: ListState,
}

impl ColumnPickerComponent {
    /// Starts with each op's own natural default checked -- matching
    /// exactly what `build_crud_snippet`/each connector's bespoke
    /// `crud_snippet` produces for an empty selection, so confirming
    /// immediately without touching anything reproduces the pre-picker,
    /// one-keystroke behavior: every column for Create, non-key columns
    /// for Update's `SET` (falling back to every column when the table
    /// has no primary key, same fallback `build_crud_snippet` already
    /// had), the primary key for Delete's `WHERE`, and nothing for Read
    /// (whose own empty-selection meaning is "SELECT *", not "no
    /// default" -- see `Component::crud_snippet`'s doc comment).
    pub fn new(op: CrudOp, columns: Vec<PickerColumn>) -> Self {
        let checked = match op {
            CrudOp::Read => HashSet::new(),
            CrudOp::Create => (0..columns.len()).collect(),
            CrudOp::Update => {
                let non_key: HashSet<usize> = columns
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !c.primary_key)
                    .map(|(i, _)| i)
                    .collect();
                if non_key.is_empty() {
                    (0..columns.len()).collect()
                } else {
                    non_key
                }
            }
            CrudOp::Delete => columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.primary_key)
                .map(|(i, _)| i)
                .collect(),
        };
        Self {
            op,
            columns,
            checked,
            selected: 0,
            pending: None,
            visible_height: 0,
            list_area: Rect::ZERO,
            list_state: ListState::default(),
        }
    }

    /// Whether the current selection is legal to confirm. Read allows
    /// empty (falls back to `SELECT *`); Create/Update/Delete all require
    /// at least one checked column -- Delete because a WHERE-less DELETE
    /// must never be reachable through this UI (see
    /// `Component::crud_snippet`'s doc comment), Create/Update because an
    /// empty column list means nothing for either (there's no "everything"
    /// shorthand the way `SELECT *` is for Read).
    fn can_confirm(&self) -> bool {
        match self.op {
            CrudOp::Read => true,
            CrudOp::Create | CrudOp::Update | CrudOp::Delete => !self.checked.is_empty(),
        }
    }

    fn selection(&self) -> Vec<String> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(i, _)| self.checked.contains(i))
            .map(|(_, c)| c.name.clone())
            .collect()
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<ColumnPickerOutcome> {
        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) = keymap().resolve_in(
            &[Context::ColumnPicker, Context::List],
            &mut self.pending,
            key,
        ) else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            vim_list::apply(
                mv,
                &mut self.selected,
                self.columns.len(),
                self.visible_height,
            );
            return None;
        }

        match command {
            Command::ToggleColumn => {
                if !self.columns.is_empty() && !self.checked.insert(self.selected) {
                    self.checked.remove(&self.selected);
                }
                None
            }
            Command::ToggleAllColumns => {
                if self.checked.len() == self.columns.len() {
                    self.checked.clear();
                } else {
                    self.checked = (0..self.columns.len()).collect();
                }
                None
            }
            Command::Confirm if self.can_confirm() => {
                Some(ColumnPickerOutcome::Confirmed(self.selection()))
            }
            Command::Cancel => Some(ColumnPickerOutcome::Cancelled),
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let popup = ui::centered_rect(50, 60, area);
        frame.render_widget(Clear, popup);

        let items: Vec<ListItem> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                let checkbox = if self.checked.contains(&i) {
                    "[x] "
                } else {
                    "[ ] "
                };
                let mut spans = vec![
                    Span::styled(checkbox, Style::default().fg(theme.text_dim)),
                    Span::styled(column.name.clone(), Style::default().fg(theme.text)),
                ];
                if column.primary_key {
                    spans.push(Span::styled("  pk", Style::default().fg(theme.text_dim)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        self.list_area = popup;
        if !self.columns.is_empty() {
            self.list_state.select(Some(self.selected));
        }
        self.visible_height = popup.height.saturating_sub(2) as usize;

        let op_label = match self.op {
            CrudOp::Create => "Create",
            CrudOp::Read => "Read",
            CrudOp::Update => "Update",
            CrudOp::Delete => "Delete",
        };
        let toggle = keymap()
            .binding_for(Context::ColumnPicker, Command::ToggleColumn)
            .unwrap_or_default();
        let toggle_all = keymap()
            .binding_for(Context::ColumnPicker, Command::ToggleAllColumns)
            .unwrap_or_default();
        let confirm = keymap()
            .binding_for(Context::ColumnPicker, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::ColumnPicker, Command::Cancel)
            .unwrap_or_default();
        let title = if self.can_confirm() {
            format!(
                "{op_label} columns — {toggle} toggle, {toggle_all} all, {confirm} confirm, {cancel} cancel"
            )
        } else {
            format!("{op_label} columns — select at least one to confirm ({cancel} cancel)")
        };

        let list = List::new(items)
            .block(ui::panel(&title, true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, popup, &mut self.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, primary_key: bool) -> PickerColumn {
        PickerColumn {
            name: name.to_string(),
            primary_key,
        }
    }

    fn users_columns() -> Vec<PickerColumn> {
        vec![column("id", true), column("email", false)]
    }

    #[test]
    fn read_defaults_to_nothing_checked_and_confirms_empty() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Read, users_columns());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, Some(ColumnPickerOutcome::Confirmed(Vec::new())));
    }

    #[test]
    fn create_defaults_to_every_column_checked() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Create, users_columns());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec![
                "id".to_string(),
                "email".to_string()
            ]))
        );
    }

    #[test]
    fn update_defaults_to_non_key_columns_checked() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Update, users_columns());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec!["email".to_string()]))
        );
    }

    #[test]
    fn update_falls_back_to_every_column_when_none_is_a_key() {
        let columns = vec![column("message", false)];
        let mut picker = ColumnPickerComponent::new(CrudOp::Update, columns);

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec!["message".to_string()]))
        );
    }

    #[test]
    fn delete_defaults_to_the_primary_key_checked() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Delete, users_columns());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec!["id".to_string()]))
        );
    }

    #[test]
    fn delete_with_everything_unchecked_cannot_be_confirmed() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Delete, users_columns());
        // Unchecks the default-checked "id" (the only checked row, at
        // cursor position 0).
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome, None,
            "a WHERE-less DELETE must never be reachable through this picker"
        );
    }

    #[test]
    fn create_with_everything_unchecked_cannot_be_confirmed() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Create, users_columns());
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None);
    }

    #[test]
    fn toggling_a_column_flips_its_membership_in_the_confirmed_selection() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Read, users_columns());

        // Cursor starts on "id" -- check it.
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);
        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec!["id".to_string()]))
        );
    }

    #[test]
    fn toggle_all_checks_everything_then_unchecks_everything() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Read, users_columns());

        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(picker.checked.len(), 2);

        picker.handle_key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(picker.checked.len(), 0);
    }

    #[test]
    fn esc_cancels_regardless_of_the_current_selection() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Create, users_columns());

        let outcome = picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(outcome, Some(ColumnPickerOutcome::Cancelled));
    }

    #[test]
    fn selection_reproduces_table_order_even_after_toggling_out_of_order() {
        let mut picker = ColumnPickerComponent::new(CrudOp::Read, users_columns());

        // Check "email" (move down first), then "id" -- table order is
        // still id, email.
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char('k'), KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Char(' '), KeyModifiers::NONE);

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(ColumnPickerOutcome::Confirmed(vec![
                "id".to_string(),
                "email".to_string()
            ]))
        );
    }
}
