//! A read-only list overlay for browsing past queries and loading one back
//! into the editor. Not a `Component` -- driven directly by
//! `QueryScreenComponent`, the same way `FilePromptComponent` is, and takes
//! over all key input while open.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryOutcome {
    Selected(String),
    Cancelled,
}

pub struct HistoryPickerComponent {
    entries: Vec<String>,
    selected: usize,
    pending_g: bool,
}

impl HistoryPickerComponent {
    /// `entries` is most-recent-first -- the order the picker displays and
    /// navigates them in.
    pub fn new(entries: Vec<String>) -> Self {
        Self {
            entries,
            selected: 0,
            pending_g: false,
        }
    }

    pub fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.entries.len() - 1);
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_to_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_to_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
    }

    pub fn selected_entry(&self) -> Option<&str> {
        self.entries.get(self.selected).map(String::as_str)
    }

    pub fn handle_key_event(&mut self, code: KeyCode) -> Option<HistoryOutcome> {
        let had_pending_g = std::mem::take(&mut self.pending_g);
        match code {
            KeyCode::Esc => Some(HistoryOutcome::Cancelled),
            KeyCode::Enter => self
                .selected_entry()
                .map(|entry| HistoryOutcome::Selected(entry.to_string())),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                None
            }
            KeyCode::Char('g') if had_pending_g => {
                self.move_to_top();
                None
            }
            KeyCode::Char('g') => {
                self.pending_g = true;
                None
            }
            KeyCode::Char('G') => {
                self.move_to_bottom();
                None
            }
            _ => None,
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| ListItem::new(entry.replace('\n', " ⏎ ")))
            .collect();

        let mut state = ListState::default();
        if !self.entries.is_empty() {
            state.select(Some(self.selected));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("History (Enter=load, Esc=cancel)"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
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

        picker.move_down();
        assert_eq!(picker.selected_entry(), Some("select 1"));

        picker.move_down();
        assert_eq!(picker.selected_entry(), Some("select 1"));
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut picker = picker();

        picker.move_up();
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[test]
    fn gg_and_shift_g_jump_to_top_and_bottom() {
        let mut picker = picker();

        picker.handle_key_event(KeyCode::Char('G'));
        assert_eq!(picker.selected_entry(), Some("select 1"));

        let first = picker.handle_key_event(KeyCode::Char('g'));
        assert!(first.is_none(), "a lone 'g' should not act yet");
        picker.handle_key_event(KeyCode::Char('g'));
        assert_eq!(picker.selected_entry(), Some("select 2"));
    }

    #[test]
    fn enter_selects_the_current_entry() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Enter);

        assert_eq!(
            outcome,
            Some(HistoryOutcome::Selected("select 2".to_string()))
        );
    }

    #[test]
    fn enter_on_an_empty_history_is_a_no_op() {
        let mut picker = HistoryPickerComponent::new(Vec::new());

        let outcome = picker.handle_key_event(KeyCode::Enter);

        assert_eq!(outcome, None);
    }

    #[test]
    fn esc_cancels() {
        let mut picker = picker();

        let outcome = picker.handle_key_event(KeyCode::Esc);

        assert_eq!(outcome, Some(HistoryOutcome::Cancelled));
    }
}
