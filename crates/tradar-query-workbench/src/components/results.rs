//! The results/error pane on the query screen. Owns no keys of its own —
//! driven entirely by `QueryScreenComponent` calling `set_result`/`set_error`
//! and its movement/yank methods directly from key handling.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};

use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list::{self, VimMove};

use crate::query_driver::QueryResult;

/// Width for each column: whatever its widest value needs, capped at
/// `MAX_COLUMN_WIDTH`, and never narrower than its own header.
fn column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|name| name.chars().count().min(MAX_COLUMN_WIDTH))
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(cell.chars().count()).min(MAX_COLUMN_WIDTH);
            }
        }
    }
    widths
}

/// Cuts `text` to `width` columns, marking the cut with `…` so a truncated
/// value can't be mistaken for a complete one.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    text.chars()
        .take(keep)
        .chain(std::iter::once('…'))
        .collect()
}

/// `1 row` / `2 rows` -- the results title reads as a sentence, so the
/// plural has to agree.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Widest a single column may get before its cells are truncated. Without
/// a cap one long text column pushes every other column off screen, which
/// is worse than losing the tail of that one value.
const MAX_COLUMN_WIDTH: usize = 40;

pub struct ResultsComponent {
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub selected: usize,
    /// First visible column, for tables too wide to fit -- see
    /// `scroll_left`/`scroll_right`.
    col_offset: usize,
    /// Whether a query is in flight, so the pane can say so: without it a
    /// slow query is indistinguishable from a key that didn't register.
    running: bool,
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
            col_offset: 0,
            running: false,
            visible_height: 0,
        }
    }

    /// Told by the screen each frame, since the engine owns that state.
    pub fn draw_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.last_result = Some(result);
        self.last_error = None;
        self.selected = 0;
        self.col_offset = 0;
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
            // Nothing to select: it's a one-line report, not a list.
            Some(QueryResult::Affected { .. }) | None => 0,
        }
    }

    /// For `Documents` `visible_height`-based half-page scrolling is an
    /// approximation (items can span multiple rows), same tradeoff as the
    /// row-count-based scrolling in `SchemaSidebarComponent`.
    /// `pub` because `QueryScreenComponent` resolves the key (it owns
    /// focus) and hands the movement down.
    pub fn apply_move(&mut self, mv: VimMove) {
        let count = self.item_count();
        vim_list::apply(mv, &mut self.selected, count, self.visible_height);
    }

    pub fn move_down(&mut self) {
        self.apply_move(VimMove::Down);
    }

    pub fn move_up(&mut self) {
        self.apply_move(VimMove::Up);
    }

    pub fn move_to_top(&mut self) {
        self.apply_move(VimMove::Top);
    }

    pub fn move_to_bottom(&mut self) {
        self.apply_move(VimMove::Bottom);
    }

    pub fn move_half_page_down(&mut self) {
        self.apply_move(VimMove::HalfPageDown);
    }

    pub fn move_half_page_up(&mut self) {
        self.apply_move(VimMove::HalfPageUp);
    }

    /// Column count of the current table result -- `0` for documents or no
    /// result, since neither scrolls horizontally.
    fn column_count(&self) -> usize {
        match &self.last_result {
            Some(QueryResult::Table { columns, .. }) => columns.len(),
            _ => 0,
        }
    }

    pub fn scroll_left(&mut self) {
        self.col_offset = self.col_offset.saturating_sub(1);
    }

    /// Scrolls one column right, always leaving at least the last column
    /// visible -- scrolling into empty space would just look broken.
    pub fn scroll_right(&mut self) {
        let last = self.column_count().saturating_sub(1);
        self.col_offset = (self.col_offset + 1).min(last);
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
            QueryResult::Affected { .. } => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = theme();
        // The row count belongs in the title: it's the first thing anyone
        // wants to know about a result, and it costs no extra space.
        if self.running {
            let block = ui::panel("Results — running…", focused);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            self.visible_height = 0;
            let cancel = tradar_core::keymap::keymap()
                .binding_for(
                    tradar_core::keymap::Context::QueryScreen,
                    tradar_core::keymap::Command::CancelQuery,
                )
                .unwrap_or_default();
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("running… {cancel} to cancel"),
                    Style::default().fg(theme.text_dim),
                )),
                inner,
            );
            return;
        }

        let title = match (&self.last_error, &self.last_result) {
            (Some(_), _) => "Results — error".to_string(),
            (
                None,
                Some(QueryResult::Table {
                    rows, truncated, ..
                }),
            ) => {
                if *truncated {
                    // Say so loudly: a silently clipped result set is a
                    // wrong answer you can't see is wrong.
                    format!("Results (first {} rows — truncated)", rows.len())
                } else {
                    format!("Results ({})", count(rows.len(), "row"))
                }
            }
            (None, Some(QueryResult::Documents(docs))) => {
                format!("Results ({})", count(docs.len(), "document"))
            }
            (None, Some(QueryResult::Affected { .. })) => "Results".to_string(),
            (None, None) => "Results".to_string(),
        };
        let block = ui::panel(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(error) = &self.last_error {
            self.visible_height = 0;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    error.as_str(),
                    Style::default().fg(theme.error),
                ))
                .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }

        let Some(result) = &self.last_result else {
            self.visible_height = 0;
            return;
        };

        match result {
            QueryResult::Table { columns, rows, .. } => {
                // A real table widget, not rows joined by spaces: columns
                // have to line up or the values can't be read down a column,
                // which is most of the point of tabular output.
                self.col_offset = self.col_offset.min(columns.len().saturating_sub(1));
                let widths = column_widths(columns, rows);

                // The header row is drawn by the widget, so it costs one row
                // of the body -- account for it or half-page scrolling
                // overshoots by one.
                self.visible_height = (inner.height as usize).saturating_sub(1);

                let header = Row::new(
                    columns
                        .iter()
                        .skip(self.col_offset)
                        .zip(widths.iter().skip(self.col_offset))
                        .map(|(name, width)| Cell::from(truncate(name, *width)))
                        .collect::<Vec<_>>(),
                )
                .style(
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                );

                let body: Vec<Row> = rows
                    .iter()
                    .map(|row| {
                        Row::new(
                            row.iter()
                                .skip(self.col_offset)
                                .zip(widths.iter().skip(self.col_offset))
                                .map(|(cell, width)| Cell::from(truncate(cell, *width)))
                                .collect::<Vec<_>>(),
                        )
                        .style(Style::default().fg(theme.text))
                    })
                    .collect();

                let constraints: Vec<Constraint> = widths
                    .iter()
                    .skip(self.col_offset)
                    .map(|w| Constraint::Length(*w as u16))
                    .collect();

                let mut state = TableState::default();
                if !rows.is_empty() {
                    state.select(Some(self.selected));
                }
                let table = Table::new(body, constraints)
                    .header(header)
                    .column_spacing(2)
                    .row_highlight_style(ui::selection_style());
                frame.render_stateful_widget(table, inner, &mut state);
            }
            QueryResult::Affected { rows } => {
                self.visible_height = 0;
                // A write reports what it did, in the pane where results
                // would otherwise appear -- the whole point is that this
                // can't be mistaken for an empty SELECT.
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!("OK — {} affected", count(*rows as usize, "row")),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )),
                    inner,
                );
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
                let list = List::new(items).highlight_style(ui::selection_style());
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
    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::query_driver::QueryResult;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    /// One rendered row as a string, for asserting on column positions.
    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer.cell((x, y)).unwrap().symbol())
            .collect()
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
            truncated: false,
        }
    }

    /// Two columns whose values are much wider than their headers -- the
    /// case that used to render ragged.
    fn wide_table() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string(), "n".to_string()],
            rows: vec![
                vec!["1".to_string(), "alice".to_string(), "10".to_string()],
                vec!["1000".to_string(), "bo".to_string(), "2000".to_string()],
            ],
            truncated: false,
        }
    }

    #[test]
    fn columns_line_up_under_their_headers() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 40, 8), false))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Column starts, read off the header row, must be where the data
        // rows start too -- that's what "aligned" means.
        let header = row_text(&buffer, 1);
        let first = row_text(&buffer, 2);
        let second = row_text(&buffer, 3);
        let name_column = header.find("name").expect("header row was: {header}");
        assert_eq!(
            first.find("alice"),
            Some(name_column),
            "row 1 was: {first:?}, header: {header:?}"
        );
        assert_eq!(
            second.find("bo"),
            Some(name_column),
            "row 2 was: {second:?}, header: {header:?}"
        );
    }

    #[test]
    fn a_value_wider_than_the_column_cap_is_truncated_with_an_ellipsis() {
        assert_eq!(truncate("alice", 5), "alice");
        assert_eq!(truncate("alexandra", 5), "alex…");
        assert_eq!(
            column_widths(
                &["v".to_string()],
                &[vec!["x".repeat(MAX_COLUMN_WIDTH + 10)]]
            ),
            vec![MAX_COLUMN_WIDTH],
            "one huge value must not push the other columns off screen"
        );
    }

    #[test]
    fn column_width_is_the_widest_of_the_header_and_its_values() {
        let widths = column_widths(
            &["id".to_string(), "name".to_string()],
            &[
                vec!["1".to_string(), "alice".to_string()],
                vec!["1000".to_string(), "bo".to_string()],
            ],
        );

        assert_eq!(widths, vec![4, 5], "id -> '1000', name -> 'alice'");
    }

    #[test]
    fn scrolling_right_moves_the_first_visible_column_and_stops_at_the_last() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());

        results.scroll_right();
        assert_eq!(results.col_offset, 1);

        results.scroll_right();
        results.scroll_right();
        assert_eq!(results.col_offset, 2, "must keep the last column visible");

        results.scroll_left();
        assert_eq!(results.col_offset, 1);
        results.scroll_left();
        results.scroll_left();
        assert_eq!(
            results.col_offset, 0,
            "must not scroll past the first column"
        );
    }

    #[test]
    fn scrolling_right_hides_the_leading_columns() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());

        results.scroll_right();

        // The `id` column (values 1 / 1000) scrolls off; the rest stays.
        let text = draw_component(&mut results, 40, 8);
        assert!(!text.contains("1000"), "buffer was: {text}");
        assert!(text.contains("alice"), "buffer was: {text}");
        assert!(text.contains("2000"), "buffer was: {text}");
    }

    #[test]
    fn a_new_result_resets_the_horizontal_scroll() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        results.scroll_right();

        results.set_result(wide_table());

        assert_eq!(results.col_offset, 0);
    }

    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());

        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });

        assert!(results.last_error.is_none());
        assert_eq!(
            results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            })
        );
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
            truncated: false,
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
            truncated: false,
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
            truncated: false,
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

    /// The border color of the panel's top-left corner, which is what
    /// tells the user where the keyboard focus is now that the title no
    /// longer spells it out.
    fn border_color(results: &mut ResultsComponent, focused: bool) -> Option<Color> {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, frame.area(), focused))
            .unwrap();
        terminal.backend().buffer().cell((0, 0)).unwrap().fg.into()
    }

    #[test]
    fn draw_marks_the_panel_as_focused_with_the_border_color() {
        let mut results = ResultsComponent::new();

        let focused = border_color(&mut results, true);
        let unfocused = border_color(&mut results, false);

        assert_eq!(focused, Some(theme().border_focused));
        assert_eq!(unfocused, Some(theme().border));
        assert_ne!(focused, unfocused);
    }

    #[test]
    fn draw_shows_the_row_count_in_the_title() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("3 rows"), "buffer was: {text}");
    }

    #[test]
    fn a_write_reports_what_it_affected_instead_of_looking_empty() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Affected { rows: 3 });

        let text = draw_component(&mut results, 40, 8);

        assert!(text.contains("OK"), "buffer was: {text}");
        assert!(text.contains("3 rows affected"), "buffer was: {text}");
        assert!(
            !text.contains("0 rows)"),
            "must not read like an empty result: {text}"
        );
    }

    #[test]
    fn a_write_has_nothing_to_select_or_yank() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Affected { rows: 3 });

        results.move_down();

        assert_eq!(results.selected, 0);
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn a_single_row_is_not_pluralised() {
        let mut results = ResultsComponent::new();
        results.set_result(table(1));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("(1 row)"), "buffer was: {text}");
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
        assert_eq!(selected_cell.bg, theme().selection_bg);
        assert!(selected_cell.modifier.contains(Modifier::BOLD));
        assert_ne!(unselected_cell.bg, theme().selection_bg);
    }
}
