//! A read-only list overlay for browsing past queries and loading one back
//! into the editor. Not a `Component` -- driven directly by
//! `QueryScreenComponent`, the same way `FilePromptComponent` is, and takes
//! over all key input while open.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker, TextInput};
use tradar_core::vim_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryOutcome {
    Selected(String),
    Cancelled,
}

pub struct HistoryPickerComponent {
    /// Shown in the panel title as "{title} — {confirm} load, {cancel}
    /// cancel" -- "History" for the actual history picker, something else
    /// for the other things this plain list-picker gets reused for (the
    /// ERD overlay's table picker, at least so far).
    title: &'static str,
    entries: Vec<String>,
    selected: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
    list_state: ListState,
    list_area: Rect,
    double_click: DoubleClickTracker,
    /// A case-insensitive substring narrowing `entries` -- same idiom as
    /// `NavigatorComponent::filter`. Kept even while `filter_input` is
    /// closed so the title can still say what's applied and a fresh `/`
    /// prefills it rather than starting over.
    filter: String,
    /// `Some` while the filter bar has the keys -- see `open_filter`.
    filter_input: Option<TextInput>,
}

impl HistoryPickerComponent {
    /// `entries` is most-recent-first -- the order the picker displays and
    /// navigates them in.
    pub fn new(entries: Vec<String>) -> Self {
        Self {
            title: "History",
            entries,
            selected: 0,
            pending: None,
            visible_height: 0,
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            double_click: DoubleClickTracker::new(),
            filter: String::new(),
            filter_input: None,
        }
    }

    /// Overrides the default "History" panel title -- for a caller reusing
    /// this as a plain searchable-by-scroll list picker for something else.
    pub fn with_title(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    /// `entries` narrowed by `filter`, when one's applied -- same idiom as
    /// `NavigatorComponent::visible_rows`.
    fn visible_entries(&self) -> Vec<&String> {
        if self.filter.is_empty() {
            return self.entries.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| entry.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&str> {
        self.visible_entries()
            .get(self.selected)
            .map(|s| s.as_str())
    }

    /// Opens the filter bar, prefilled with whatever's already applied.
    fn open_filter(&mut self) {
        self.filter_input = Some(TextInput::new(&self.filter));
    }

    fn is_filtering(&self) -> bool {
        self.filter_input.is_some()
    }

    /// One key while the filter bar has the keys. `Esc` cancels -- clears
    /// the bar *and* whatever was applied. `Enter` keeps the filter and
    /// closes the bar. Anything else is text editing, applied live.
    fn filter_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let Some(input) = self.filter_input.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                self.filter_input = None;
                self.filter.clear();
                self.selected = 0;
            }
            KeyCode::Enter => self.filter_input = None,
            _ => {
                input.handle_key_event(code, modifiers);
                self.filter = input.text();
                self.selected = 0;
            }
        }
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<HistoryOutcome> {
        if self.is_filtering() {
            self.filter_key_event(code, modifiers);
            return None;
        }

        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) = keymap().resolve_in(
            &[Context::History, Context::Prompt, Context::List],
            &mut self.pending,
            key,
        ) else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            let len = self.visible_entries().len();
            vim_list::apply(mv, &mut self.selected, len, self.visible_height);
            return None;
        }

        match command {
            Command::Cancel => Some(HistoryOutcome::Cancelled),
            Command::Confirm => self
                .selected_entry()
                .map(|entry| HistoryOutcome::Selected(entry.to_string())),
            Command::Search => {
                self.open_filter();
                None
            }
            _ => None,
        }
    }

    /// A left click selects the row it landed on; a second one on that same
    /// row within the double-click window loads it, same as `Enter`.
    pub fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<HistoryOutcome> {
        let MouseEventKind::Down(MouseButton::Left) = event.kind else {
            return None;
        };
        let inner = Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        };
        let visible: Vec<String> = self.visible_entries().into_iter().cloned().collect();
        let index = ui::index_at(inner, self.list_state.offset(), event.row, visible.len())?;
        self.selected = index;
        self.double_click
            .click(index)
            .then(|| HistoryOutcome::Selected(visible[index].clone()))
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let (area, filter_bar_area) = if self.filter_input.is_some() {
            let (list_area, bar) = ui::split_bottom_bar(area, 1);
            (list_area, Some(bar))
        } else {
            (area, None)
        };

        let visible = self.visible_entries();
        let items: Vec<ListItem> = visible
            .iter()
            .map(|entry| {
                ListItem::new(Span::styled(
                    format!(" {}", entry.replace('\n', " ⏎ ")),
                    Style::default().fg(theme.text),
                ))
            })
            .collect();
        let visible_len = items.len();

        self.list_area = area;
        if visible_len > 0 {
            self.list_state.select(Some(self.selected));
        }

        self.visible_height = area.height.saturating_sub(2) as usize;
        let confirm = keymap()
            .binding_for(Context::Prompt, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Prompt, Command::Cancel)
            .unwrap_or_default();
        let title = if self.filter.is_empty() {
            format!("{} — {confirm} load, {cancel} cancel", self.title)
        } else {
            format!(
                "{} — filter: {} — {confirm} load, {cancel} cancel",
                self.title, self.filter
            )
        };
        let list = List::new(items)
            .block(ui::panel(&title, true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut self.list_state);

        if let (Some(bar_area), Some(input)) = (filter_bar_area, &self.filter_input) {
            let mut spans = vec![Span::styled("/", Style::default().fg(theme.accent))];
            spans.extend(input.spans(true));
            frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn picker() -> HistoryPickerComponent {
        HistoryPickerComponent::new(vec!["select 2".to_string(), "select 1".to_string()])
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

    #[test]
    fn slash_opens_the_filter_and_narrows_entries_case_insensitively() {
        let mut picker = HistoryPickerComponent::new(vec![
            "SELECT orders".to_string(),
            "select payments".to_string(),
            "delete users".to_string(),
        ]);

        picker.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(picker.is_filtering());
        for c in "select".chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert_eq!(picker.selected_entry(), Some("SELECT orders"));
        let visible: Vec<&str> = picker
            .visible_entries()
            .into_iter()
            .map(String::as_str)
            .collect();
        assert_eq!(visible, vec!["SELECT orders", "select payments"]);
    }

    #[test]
    fn esc_in_the_filter_clears_it_and_resets_the_cursor() {
        let mut picker =
            HistoryPickerComponent::new(vec!["select 1".to_string(), "select 2".to_string()]);
        picker.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "select".chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(!picker.is_filtering());
        assert!(picker.filter.is_empty());
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.visible_entries().len(), 2);
    }

    #[test]
    fn enter_in_the_filter_keeps_it_applied_and_closes_the_bar() {
        let mut picker =
            HistoryPickerComponent::new(vec!["select 1".to_string(), "delete users".to_string()]);
        picker.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "select".chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(!picker.is_filtering());
        assert_eq!(picker.filter, "select");
        assert_eq!(
            picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE),
            Some(HistoryOutcome::Selected("select 1".to_string())),
            "the real Enter (not the filter's) must still load the filtered entry"
        );
    }

    #[test]
    fn loading_an_entry_while_filtered_loads_the_matching_one_not_the_raw_index() {
        let mut picker = HistoryPickerComponent::new(vec![
            "select 1".to_string(),
            "delete users".to_string(),
            "select 2".to_string(),
        ]);
        picker.handle_key_event(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "select".chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE); // close the bar
        picker.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE); // "select 2"

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(HistoryOutcome::Selected("select 2".to_string()))
        );
    }

    #[test]
    fn clicking_a_row_selects_it_without_loading_it() {
        let mut picker = picker();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        // Row 0 is the border, row 2 is "select 1" (the second entry).
        let outcome = picker.handle_mouse_event(click_at(2, 2));

        assert_eq!(outcome, None);
        assert_eq!(picker.selected_entry(), Some("select 1"));
    }

    #[test]
    fn double_clicking_a_row_loads_it_same_as_enter() {
        let mut picker = picker();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        picker.handle_mouse_event(click_at(2, 2));
        let outcome = picker.handle_mouse_event(click_at(2, 2));

        assert_eq!(
            outcome,
            Some(HistoryOutcome::Selected("select 1".to_string()))
        );
    }
}
