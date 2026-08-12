//! A read-only list overlay for browsing past queries and loading one back
//! into the editor. Not a `Component` -- driven directly by
//! `QueryScreenComponent`, the same way `FilePromptComponent` is, and takes
//! over all key input while open.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, ListState};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryOutcome {
    Selected(String),
    Cancelled,
}

pub struct HistoryPickerComponent {
    entries: Vec<String>,
    selected: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
}

impl HistoryPickerComponent {
    /// `entries` is most-recent-first -- the order the picker displays and
    /// navigates them in.
    pub fn new(entries: Vec<String>) -> Self {
        Self {
            entries,
            selected: 0,
            pending: None,
            visible_height: 0,
        }
    }

    pub fn selected_entry(&self) -> Option<&str> {
        self.entries.get(self.selected).map(String::as_str)
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<HistoryOutcome> {
        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Prompt, Context::List], &mut self.pending, key)
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

        match command {
            Command::Cancel => Some(HistoryOutcome::Cancelled),
            Command::Confirm => self
                .selected_entry()
                .map(|entry| HistoryOutcome::Selected(entry.to_string())),
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(Span::styled(
                    format!(" {}", entry.replace('\n', " ⏎ ")),
                    Style::default().fg(theme.text),
                ))
            })
            .collect();

        let mut state = ListState::default();
        if !self.entries.is_empty() {
            state.select(Some(self.selected));
        }

        self.visible_height = area.height.saturating_sub(2) as usize;
        let confirm = keymap()
            .binding_for(Context::Prompt, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Prompt, Command::Cancel)
            .unwrap_or_default();
        let list = List::new(items)
            .block(ui::panel(
                &format!("History — {confirm} load, {cancel} cancel"),
                true,
            ))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> HistoryPickerComponent {
        HistoryPickerComponent::new(vec!["select 2".to_string(), "select 1".to_string()])
    }

    #[test]
    fn starts_selecting_the_most_recent_entry() {
        let picker = picker();
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_entry() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(picker.selected_entry(), Some("select 1"));

        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(picker.selected_entry(), Some("select 1"));
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[test]
    fn gg_and_shift_g_jump_to_top_and_bottom() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(picker.selected_entry(), Some("select 1"));

        let first = picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(first.is_none(), "a lone 'g' should not act yet");
        picker.handle_key_event(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[test]
    fn ctrl_d_and_ctrl_u_scroll_by_half_the_visible_height() {
        let mut picker = HistoryPickerComponent::new(vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string(),
            "5".to_string(),
            "6".to_string(),
        ]);
        picker.visible_height = 10;

        picker.handle_key_event(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 5, "should clamp to the last entry");

        picker.handle_key_event(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn enter_selects_the_current_entry() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(HistoryOutcome::Selected("select 2".to_string()))
        );
    }

    #[test]
    fn enter_on_an_empty_history_is_a_no_op() {
        let mut picker = HistoryPickerComponent::new(Vec::new());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None);
    }

    #[test]
    fn esc_cancels() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(outcome, Some(HistoryOutcome::Cancelled));
    }
}
