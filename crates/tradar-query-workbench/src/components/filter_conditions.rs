//! The filter-conditions panel (`F3` in `Context::Results`): lists every
//! condition the current results-grid filter parses into (see
//! `crate::filter::ParsedFilter`) with its `AND`/`OR` relation to its
//! neighbors, and lets `d` drop one. Not a `Component` -- driven directly
//! by `QueryScreenComponent`, the same way `HistoryPickerComponent` is, and
//! takes over all key input while open.
//!
//! Deleting doesn't reach into `ResultsComponent` directly: this owns its
//! own `ParsedFilter`, rebuilds it (`ParsedFilter::without`, then
//! `render()`) on every delete, and hands the new filter text back via
//! `FilterConditionsOutcome::Changed` -- the caller re-applies it through
//! `ResultsComponent::set_filter`, the same single entry point typing into
//! the filter box already goes through, so the two can't drift apart on
//! how a filter string is interpreted.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, ListState};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker};
use tradar_core::vim_list;

use crate::filter::ParsedFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterConditionsOutcome {
    Cancelled,
    /// A condition was deleted -- the new filter text to apply. Empty when
    /// that was the last condition; the caller closes the panel then, the
    /// same as it would for `Cancelled` (nothing left here to manage).
    Changed(String),
}

pub struct FilterConditionsComponent {
    filter: ParsedFilter,
    /// One row per condition, flattened out of `filter`'s groups with its
    /// `AND`/`OR` prefix already applied -- `(group index, condition index
    /// within that group, display text)`. Recomputed whenever `filter`
    /// changes (`new`, after a delete) so the two can never drift apart --
    /// `self.selected` indexes into this, and `delete_selected` reads the
    /// `(group, condition)` pair back out of it to know what to remove.
    rows: Vec<(usize, usize, String)>,
    selected: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
    list_state: ListState,
    list_area: Rect,
    double_click: DoubleClickTracker,
}

impl FilterConditionsComponent {
    /// `text`/`columns` are the results grid's current filter text and
    /// column list -- the same two `ParsedFilter::parse` always takes (see
    /// `ResultsComponent::filter`/`columns`).
    pub fn new(text: &str, columns: &[String]) -> Self {
        let filter = ParsedFilter::parse(text, columns);
        let rows = flatten(&filter);
        Self {
            filter,
            rows,
            selected: 0,
            pending: None,
            visible_height: 0,
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            double_click: DoubleClickTracker::new(),
        }
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<FilterConditionsOutcome> {
        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) = keymap().resolve_in(
            &[Context::FilterConditions, Context::List],
            &mut self.pending,
            key,
        ) else {
            return None;
        };

        if let Some(mv) = command.as_vim_move() {
            vim_list::apply(mv, &mut self.selected, self.rows.len(), self.visible_height);
            return None;
        }

        match command {
            Command::Cancel | Command::Confirm => Some(FilterConditionsOutcome::Cancelled),
            Command::DeleteFilterCondition => self.delete_selected(),
            _ => None,
        }
    }

    /// A left click selects the row it landed on; a second one on that same
    /// row within the double-click window deletes it, same as `d`.
    pub fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<FilterConditionsOutcome> {
        let MouseEventKind::Down(MouseButton::Left) = event.kind else {
            return None;
        };
        let inner = Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        };
        let index = ui::index_at(inner, self.list_state.offset(), event.row, self.rows.len())?;
        self.selected = index;
        if self.double_click.click(index) {
            self.delete_selected()
        } else {
            None
        }
    }

    fn delete_selected(&mut self) -> Option<FilterConditionsOutcome> {
        let &(group, condition, _) = self.rows.get(self.selected)?;
        self.filter = self.filter.without(group, condition);
        self.rows = flatten(&self.filter);
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        Some(FilterConditionsOutcome::Changed(self.filter.render()))
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|(_, _, text)| {
                ListItem::new(Span::styled(
                    format!(" {text}"),
                    Style::default().fg(theme.text),
                ))
            })
            .collect();

        self.list_area = area;
        if !self.rows.is_empty() {
            self.list_state.select(Some(self.selected));
        }

        self.visible_height = area.height.saturating_sub(2) as usize;
        let cancel = keymap()
            .binding_for(Context::FilterConditions, Command::Cancel)
            .unwrap_or_default();
        let delete = keymap()
            .binding_for(Context::FilterConditions, Command::DeleteFilterCondition)
            .unwrap_or_default();
        let list = List::new(items)
            .block(ui::panel(
                &format!("Filter conditions — {delete} delete, {cancel} close"),
                true,
            ))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}

/// One row per condition, with its `AND`/`OR` relation to what came before
/// it baked into the display text: the first condition overall has no
/// prefix, the first condition of every later group is prefixed `OR `
/// (that's the operator joining it to the group before it), every other
/// condition within a group is prefixed `AND `.
fn flatten(filter: &ParsedFilter) -> Vec<(usize, usize, String)> {
    let mut rows = Vec::new();
    for (group_index, group) in filter.groups().iter().enumerate() {
        for (condition_index, condition) in group.iter().enumerate() {
            let prefix = if group_index == 0 && condition_index == 0 {
                ""
            } else if condition_index == 0 {
                "OR "
            } else {
                "AND "
            };
            let text = match &condition.column {
                Some((name, _)) => format!("{prefix}{name}: {}", condition.value),
                None => format!("{prefix}{}", condition.value),
            };
            rows.push((group_index, condition_index, text));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn columns() -> Vec<String> {
        vec!["status".to_string(), "role".to_string()]
    }

    fn panel() -> FilterConditionsComponent {
        FilterConditionsComponent::new("status:active AND role:admin OR status:pending", &columns())
    }

    #[test]
    fn lists_every_condition_with_its_and_or_prefix() {
        let panel = panel();
        let texts: Vec<&str> = panel.rows.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(
            texts,
            vec!["status: active", "AND role: admin", "OR status: pending"]
        );
    }

    #[test]
    fn esc_cancels() {
        let mut panel = panel();
        let outcome = panel.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(outcome, Some(FilterConditionsOutcome::Cancelled));
    }

    #[test]
    fn enter_also_cancels_a_read_only_panel() {
        let mut panel = panel();
        let outcome = panel.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(outcome, Some(FilterConditionsOutcome::Cancelled));
    }

    #[test]
    fn d_deletes_the_selected_condition_and_reports_the_rebuilt_filter() {
        let mut panel = panel();

        let outcome = panel.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(FilterConditionsOutcome::Changed(
                "role:admin OR status:pending".to_string()
            ))
        );
    }

    #[test]
    fn deleting_the_last_condition_of_a_group_drops_the_whole_group() {
        let mut panel = panel();
        panel.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE); // AND role: admin
        panel.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE); // OR status: pending

        let outcome = panel.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(FilterConditionsOutcome::Changed(
                "status:active AND role:admin".to_string()
            ))
        );
    }

    #[test]
    fn deleting_the_only_condition_reports_an_empty_filter() {
        let mut panel = FilterConditionsComponent::new("status:active", &columns());

        let outcome = panel.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(FilterConditionsOutcome::Changed(String::new()))
        );
    }

    #[test]
    fn deleting_clamps_selection_to_the_new_last_row() {
        let mut panel = panel();
        panel.selected = 2; // "OR status: pending", the last row

        panel.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        assert_eq!(
            panel.selected, 1,
            "selection should clamp back onto a real row"
        );
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_row() {
        let mut panel = panel();

        panel.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        panel.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        panel.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        assert_eq!(panel.selected, 2);
    }

    #[test]
    fn double_clicking_a_row_deletes_it_same_as_d() {
        let mut panel = panel();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| panel.draw(frame, frame.area()))
            .unwrap();

        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        panel.handle_mouse_event(click);
        let outcome = panel.handle_mouse_event(click);

        assert_eq!(
            outcome,
            Some(FilterConditionsOutcome::Changed(
                "role:admin OR status:pending".to_string()
            ))
        );
    }
}
