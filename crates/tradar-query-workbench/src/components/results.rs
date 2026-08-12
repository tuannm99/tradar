//! The results/error pane on the query screen. Owns no keys of its own —
//! driven entirely by `QueryScreenComponent` calling `set_result`/`set_error`
//! and its movement/yank methods directly from key handling.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::query_driver::QueryResult;

pub struct ResultsComponent {
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub selected: usize,
    visible_height: usize,
}

impl Default for ResultsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultsComponent {
    pub fn new() -> Self {
        Self {
            last_result: None,
            last_error: None,
            selected: 0,
            visible_height: 0,
        }
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.last_result = Some(result);
        self.last_error = None;
        self.selected = 0;
    }

    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.last_result = None;
        self.selected = 0;
    }

    fn item_count(&self) -> usize {
        match &self.last_result {
            Some(QueryResult::Table { rows, .. }) => rows.len(),
            Some(QueryResult::Documents(docs)) => docs.len(),
            None => 0,
        }
    }

    fn move_down_by(&mut self, delta: usize) {
        let count = self.item_count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + delta).min(count - 1);
    }

    fn move_up_by(&mut self, delta: usize) {
        self.selected = self.selected.saturating_sub(delta);
    }

    pub fn move_down(&mut self) {
        self.move_down_by(1);
    }

    pub fn move_up(&mut self) {
        self.move_up_by(1);
    }

    pub fn move_to_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_to_bottom(&mut self) {
        self.selected = self.item_count().saturating_sub(1);
    }

    /// Half the last-rendered visible row count, minimum 1 -- matches vim's
    /// `Ctrl+d`/`Ctrl+u`. For `Documents` this is an approximation (items
    /// can span multiple rows), same tradeoff as the row-count-based
    /// scrolling in `SchemaSidebarComponent`.
    fn half_page(&self) -> usize {
        (self.visible_height / 2).max(1)
    }

    pub fn move_half_page_down(&mut self) {
        let step = self.half_page();
        self.move_down_by(step);
    }

    pub fn move_half_page_up(&mut self) {
        let step = self.half_page();
        self.move_up_by(step);
    }

    /// Plain-text form of the currently selected row/document, ready to
    /// yank to the clipboard. `None` when there's nothing to select (no
    /// result yet, or the last response was an error). Table rows are
    /// tab-separated, matching what spreadsheets expect when pasted.
    pub fn selected_text(&self) -> Option<String> {
        match self.last_result.as_ref()? {
            QueryResult::Table { rows, .. } => rows.get(self.selected).map(|row| row.join("\t")),
            QueryResult::Documents(docs) => docs
                .get(self.selected)
                .map(|doc| serde_json::to_string_pretty(doc).unwrap_or_default()),
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = if focused {
            "Results [focused]"
        } else {
            "Results"
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(error) = &self.last_error {
            self.visible_height = 0;
            frame.render_widget(Paragraph::new(error.as_str()), inner);
            return;
        }

        let Some(result) = &self.last_result else {
            self.visible_height = 0;
            return;
        };

        match result {
            QueryResult::Table { columns, rows } => {
                let list_area = if columns.is_empty() {
                    inner
                } else {
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(1), Constraint::Min(0)])
                        .split(inner);
                    frame.render_widget(
                        Paragraph::new(columns.join(" | "))
                            .style(Style::default().add_modifier(Modifier::BOLD)),
                        chunks[0],
                    );
                    chunks[1]
                };
                self.visible_height = list_area.height as usize;

                let items: Vec<ListItem> = rows
                    .iter()
                    .map(|row| ListItem::new(row.join(" | ")))
                    .collect();
                let mut state = ListState::default();
                if !rows.is_empty() {
                    state.select(Some(self.selected));
                }
                let list = List::new(items)
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                frame.render_stateful_widget(list, list_area, &mut state);
            }
            QueryResult::Documents(docs) => {
                self.visible_height = inner.height as usize;

                let items: Vec<ListItem> = docs
                    .iter()
                    .map(|doc| {
                        let pretty = serde_json::to_string_pretty(doc).unwrap_or_default();
                        ListItem::new(Text::from(
                            pretty
                                .lines()
                                .map(|line| Line::from(line.to_string()))
                                .collect::<Vec<_>>(),
                        ))
                    })
                    .collect();
                let mut state = ListState::default();
                if !docs.is_empty() {
                    state.select(Some(self.selected));
                }
                let list = List::new(items)
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                frame.render_stateful_widget(list, inner, &mut state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::*;
    use crate::query_driver::QueryResult;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn draw_component(component: &mut ResultsComponent, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, Rect::new(0, 0, width, height), false))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn table(rows: usize) -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: (0..rows).map(|i| vec![i.to_string()]).collect(),
        }
    }

    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());

        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
        });

        assert!(results.last_error.is_none());
        assert_eq!(
            results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
        });

        results.set_error("boom".to_string());

        assert!(results.last_result.is_none());
        assert_eq!(results.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn set_result_resets_the_selection() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));
        results.move_down();
        assert_eq!(results.selected, 1);

        results.set_result(table(5));

        assert_eq!(results.selected, 0);
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_row() {
        let mut results = ResultsComponent::new();
        results.set_result(table(2));

        results.move_down();
        assert_eq!(results.selected, 1);

        results.move_down();
        assert_eq!(results.selected, 1, "should stop at the last row, not wrap");
    }

    #[test]
    fn move_up_retreats_and_stops_at_zero() {
        let mut results = ResultsComponent::new();
        results.set_result(table(2));
        results.move_down();

        results.move_up();
        assert_eq!(results.selected, 0);

        results.move_up();
        assert_eq!(results.selected, 0, "should stop at zero, not go negative");
    }

    #[test]
    fn move_to_top_and_bottom_jump_straight_there() {
        let mut results = ResultsComponent::new();
        results.set_result(table(5));
        results.move_down();

        results.move_to_bottom();
        assert_eq!(results.selected, 4);

        results.move_to_top();
        assert_eq!(results.selected, 0);
    }

    #[test]
    fn movement_is_a_no_op_when_there_is_no_result() {
        let mut results = ResultsComponent::new();

        results.move_down();
        results.move_to_bottom();

        assert_eq!(results.selected, 0);
    }

    #[test]
    fn half_page_scroll_moves_by_half_the_visible_rows() {
        let mut results = ResultsComponent::new();
        results.set_result(table(20));
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 30, 12), false))
            .unwrap();
        // 12-row area minus 2 border rows minus 1 header row = 9 visible
        // rows -> half page = 4.

        results.move_half_page_down();
        assert_eq!(results.selected, 4);

        results.move_half_page_up();
        assert_eq!(results.selected, 0);
    }

    #[test]
    fn selected_text_is_none_without_a_result() {
        let results = ResultsComponent::new();
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn selected_text_is_none_for_an_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn selected_text_tab_separates_the_selected_table_row() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec!["1".to_string(), "Ada".to_string()],
                vec!["2".to_string(), "Lin".to_string()],
            ],
        });
        results.move_down();

        assert_eq!(results.selected_text().as_deref(), Some("2\tLin"));
    }

    #[test]
    fn selected_text_pretty_prints_the_selected_document() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"a": 1}),
            serde_json::json!({"b": 2}),
        ]));
        results.move_down();

        assert_eq!(
            results.selected_text().as_deref(),
            Some(
                serde_json::to_string_pretty(&serde_json::json!({"b": 2}))
                    .unwrap()
                    .as_str()
            )
        );
    }

    #[test]
    fn draw_shows_the_last_table_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["42".to_string()]],
        });

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("42"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_documents_pretty_printed() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada"}),
        ]));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("Ada"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_the_last_error() {
        let mut results = ResultsComponent::new();
        results.set_error("syntax error".to_string());

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("syntax error"), "buffer was: {text}");
    }

    #[test]
    fn draw_marks_the_title_as_focused() {
        let mut results = ResultsComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| results.draw(frame, frame.area(), true))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Results [focused]"), "buffer was: {text}");
    }

    #[test]
    fn draw_selection_highlight_tracks_selected() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));
        results.move_down();

        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 20, 10), false))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Row 0 is the border+title, row 1 is the column header, row 2 is
        // the first data row ("0"), row 3 is the selected row ("1").
        let unselected_cell = buffer.cell((1, 2)).unwrap();
        let selected_cell = buffer.cell((1, 3)).unwrap();
        assert!(selected_cell.modifier.contains(Modifier::REVERSED));
        assert!(!unselected_cell.modifier.contains(Modifier::REVERSED));
    }
}
