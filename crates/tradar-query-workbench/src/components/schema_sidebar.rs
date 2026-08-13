//! The schema (table/collection/index/key) sidebar on the query screen.
//! Not a `Component` — driven entirely by `QueryScreenComponent`, which
//! owns whether it currently has keyboard focus.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list::{self, VimMove};

use crate::query_driver::SchemaInfo;

/// The drawable area inside a bordered panel.
fn inner_of(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// One visible line of the sidebar. Tables are always listed; a table's
/// columns appear underneath it only while it's expanded, so the selection
/// index has to address both kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Table(usize),
    Column { table: usize, column: usize },
}

pub struct SchemaSidebarComponent {
    pub schema: Vec<SchemaInfo>,
    /// Index into the *visible rows* (tables plus the columns of expanded
    /// tables), not into `schema`.
    pub schema_selected: usize,
    pub schema_error: Option<String>,
    /// Indices into `schema` whose columns are shown.
    expanded: std::collections::HashSet<usize>,
    /// Kept between frames so ratatui's scroll offset survives -- which is
    /// also what makes a click land on the row the user actually pointed
    /// at once the list has scrolled.
    list_state: ListState,
    /// Where the list was last drawn, for hit-testing clicks.
    list_area: Rect,
    visible_height: usize,
}

impl Default for SchemaSidebarComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaSidebarComponent {
    pub fn new() -> Self {
        Self {
            schema: Vec::new(),
            schema_selected: 0,
            schema_error: None,
            expanded: std::collections::HashSet::new(),
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            visible_height: 0,
        }
    }

    pub fn set_schema(&mut self, schema: Vec<SchemaInfo>) {
        self.schema = schema;
        self.schema_selected = 0;
        self.schema_error = None;
        self.expanded.clear();
    }

    /// The visible lines, in order.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.schema.len());
        for (table, entry) in self.schema.iter().enumerate() {
            rows.push(Row::Table(table));
            if self.expanded.contains(&table) {
                rows.extend((0..entry.columns.len()).map(|column| Row::Column { table, column }));
            }
        }
        rows
    }

    fn selected_row(&self) -> Option<Row> {
        self.rows().get(self.schema_selected).copied()
    }

    /// Shows the selected table's columns. A table with no column detail
    /// (Mongo, Redis, or a driver that doesn't report them) has nothing to
    /// show, so this leaves it alone rather than drawing an empty branch.
    pub fn expand(&mut self) {
        if let Some(Row::Table(table)) = self.selected_row()
            && !self.schema[table].columns.is_empty()
        {
            self.expanded.insert(table);
        }
    }

    /// Hides the selected table's columns. On a column, collapses its
    /// parent table and moves the selection there -- otherwise the row you
    /// were on would vanish from under you.
    pub fn collapse(&mut self) {
        match self.selected_row() {
            Some(Row::Table(table)) => {
                self.expanded.remove(&table);
            }
            Some(Row::Column { table, .. }) => {
                self.expanded.remove(&table);
                if let Some(index) = self.rows().iter().position(|r| *r == Row::Table(table)) {
                    self.schema_selected = index;
                }
            }
            None => {}
        }
    }

    pub fn set_schema_error(&mut self, error: String) {
        self.schema_error = Some(error);
    }

    /// Moves the selection. `pub` because `QueryScreenComponent` resolves
    /// the key (it owns focus) and hands the movement down.
    pub fn apply_move(&mut self, mv: VimMove) {
        let len = self.rows().len();
        vim_list::apply(mv, &mut self.schema_selected, len, self.visible_height);
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

    /// Selects whatever row was clicked. Returns whether the click landed
    /// in this pane at all, so the caller can tell "handled" from "not
    /// mine".
    pub fn click(&mut self, column: u16, row: u16) -> bool {
        if !ui::contains(self.list_area, column, row) {
            return false;
        }
        let inner = inner_of(self.list_area);
        if let Some(index) = ui::index_at(inner, self.list_state.offset(), row, self.rows().len()) {
            self.schema_selected = index;
        }
        true
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        ui::contains(self.list_area, column, row)
    }

    /// The name under the cursor -- a table's, or a column's when one is
    /// selected, which is what you want inserted into the query either way.
    pub fn selected_name(&self) -> Option<&str> {
        match self.selected_row()? {
            Row::Table(table) => Some(self.schema[table].name.as_str()),
            Row::Column { table, column } => Some(self.schema[table].columns[column].name.as_str()),
        }
    }

    pub fn reset(&mut self) {
        self.schema = Vec::new();
        self.schema_selected = 0;
        self.schema_error = None;
        self.expanded.clear();
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = theme();
        let rows = self.rows();
        let items: Vec<ListItem> = rows
            .iter()
            .map(|row| match row {
                Row::Table(table) => {
                    let entry = &self.schema[*table];
                    // A marker only where there's something to open, so an
                    // un-expandable entry doesn't look broken when `l`
                    // does nothing.
                    let marker = match (entry.columns.is_empty(), self.expanded.contains(table)) {
                        (true, _) => "  ",
                        (false, true) => "▾ ",
                        (false, false) => "▸ ",
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker.to_string(), Style::default().fg(theme.text_dim)),
                        Span::styled(entry.name.clone(), Style::default().fg(theme.text)),
                    ]))
                }
                Row::Column { table, column } => {
                    let column = &self.schema[*table].columns[*column];
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("    {}", column.name),
                            Style::default().fg(theme.text),
                        ),
                        Span::styled(
                            format!("  {}", column.type_name),
                            Style::default().fg(theme.text_dim),
                        ),
                    ]))
                }
            })
            .collect();

        if rows.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(self.schema_selected));
        }

        let title = format!("Schema ({})", self.schema.len());

        let Some(error) = &self.schema_error else {
            self.visible_height = area.height.saturating_sub(2) as usize;
            self.list_area = area;
            let list = List::new(items)
                .block(ui::panel(&title, focused))
                .highlight_style(ui::selection_style());
            frame.render_stateful_widget(list, area, &mut self.list_state);
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(7)])
            .split(area);
        self.visible_height = chunks[0].height.saturating_sub(2) as usize;
        self.list_area = chunks[0];

        let list = List::new(items)
            .block(ui::panel(&title, focused))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, chunks[0], &mut self.list_state);

        let error_box = Paragraph::new(Span::styled(
            error.as_str(),
            Style::default().fg(theme.error),
        ))
        .block(ui::panel("Error", false))
        .wrap(Wrap { trim: true });
        frame.render_widget(error_box, chunks[1]);
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
    use crate::query_driver::{ColumnInfo, SchemaInfo};

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo::new("users".to_string()),
            SchemaInfo::new("orders".to_string()),
        ]
    }

    fn draw_component(component: &mut SchemaSidebarComponent, focused: bool) -> (String, Buffer) {
        let backend = TestBackend::new(26, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, Rect::new(0, 0, 26, 10), focused))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (buffer_text(&buffer), buffer)
    }

    #[test]
    fn set_schema_replaces_the_schema_and_resets_selection_and_error() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema_error("boom".to_string());
        sidebar.schema_selected = 1;

        sidebar.set_schema(schema());

        assert_eq!(sidebar.schema, schema());
        assert_eq!(sidebar.schema_selected, 0);
        assert!(sidebar.schema_error.is_none());
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_item() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());

        sidebar.move_down();
        assert_eq!(sidebar.schema_selected, 1);

        sidebar.move_down();
        assert_eq!(
            sidebar.schema_selected, 1,
            "should stop at the last item, not wrap"
        );
    }

    #[test]
    fn move_up_retreats_and_stops_at_zero() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());
        sidebar.move_down();

        sidebar.move_up();
        assert_eq!(sidebar.schema_selected, 0);

        sidebar.move_up();
        assert_eq!(
            sidebar.schema_selected, 0,
            "should stop at zero, not go negative"
        );
    }

    #[test]
    fn move_to_top_jumps_straight_to_the_first_item() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());
        sidebar.move_down();

        sidebar.move_to_top();

        assert_eq!(sidebar.schema_selected, 0);
    }

    #[test]
    fn move_to_bottom_jumps_straight_to_the_last_item() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());

        sidebar.move_to_bottom();

        assert_eq!(sidebar.schema_selected, 1);
    }

    #[test]
    fn move_to_bottom_on_an_empty_schema_stays_at_zero() {
        let mut sidebar = SchemaSidebarComponent::new();

        sidebar.move_to_bottom();

        assert_eq!(sidebar.schema_selected, 0);
    }

    #[test]
    fn half_page_scroll_moves_by_half_the_visible_rows() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(vec![
            SchemaInfo::new("a".to_string()),
            SchemaInfo::new("b".to_string()),
            SchemaInfo::new("c".to_string()),
            SchemaInfo::new("d".to_string()),
            SchemaInfo::new("e".to_string()),
            SchemaInfo::new("f".to_string()),
        ]);
        let backend = TestBackend::new(26, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, Rect::new(0, 0, 26, 12), false))
            .unwrap();
        // 12-row area minus 2 border rows = 10 visible rows -> half page = 5.

        sidebar.move_half_page_down();
        assert_eq!(sidebar.schema_selected, 5, "should clamp to the last item");

        sidebar.move_half_page_up();
        assert_eq!(sidebar.schema_selected, 0);
    }

    fn schema_with_columns() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo {
                name: "users".to_string(),
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        type_name: "INTEGER".to_string(),
                    },
                    ColumnInfo {
                        name: "email".to_string(),
                        type_name: "TEXT".to_string(),
                    },
                ],
            },
            SchemaInfo::new("orders"),
        ]
    }

    #[test]
    fn expanding_a_table_inserts_its_columns_as_rows_below_it() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema_with_columns());
        assert_eq!(sidebar.rows().len(), 2, "collapsed: just the two tables");

        sidebar.expand();

        assert_eq!(sidebar.rows().len(), 4, "users + its 2 columns + orders");
        sidebar.move_down();
        assert_eq!(sidebar.selected_name(), Some("id"));
        sidebar.move_down();
        assert_eq!(sidebar.selected_name(), Some("email"));
        sidebar.move_down();
        assert_eq!(sidebar.selected_name(), Some("orders"));
    }

    #[test]
    fn collapsing_from_a_column_returns_to_its_table() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema_with_columns());
        sidebar.expand();
        sidebar.move_down();
        assert_eq!(sidebar.selected_name(), Some("id"));

        sidebar.collapse();

        assert_eq!(sidebar.rows().len(), 2);
        assert_eq!(
            sidebar.selected_name(),
            Some("users"),
            "the selection must not be left pointing at a row that vanished"
        );
    }

    #[test]
    fn a_table_with_no_column_detail_cannot_be_expanded() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema_with_columns());
        sidebar.move_to_bottom();
        assert_eq!(sidebar.selected_name(), Some("orders"));

        sidebar.expand();

        assert_eq!(sidebar.rows().len(), 2, "nothing to expand, nothing added");
    }

    #[test]
    fn a_new_schema_collapses_everything() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema_with_columns());
        sidebar.expand();
        assert_eq!(sidebar.rows().len(), 4);

        sidebar.set_schema(schema_with_columns());

        assert_eq!(sidebar.rows().len(), 2);
        assert_eq!(sidebar.schema_selected, 0);
    }

    #[test]
    fn draw_shows_columns_with_their_types_when_expanded() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema_with_columns());
        sidebar.expand();

        let (text, _) = draw_component(&mut sidebar, false);

        assert!(text.contains("users"), "buffer was: {text}");
        assert!(text.contains("id"), "buffer was: {text}");
        assert!(text.contains("INTEGER"), "buffer was: {text}");
    }

    #[test]
    fn selected_name_returns_none_when_schema_is_empty() {
        let sidebar = SchemaSidebarComponent::new();
        assert_eq!(sidebar.selected_name(), None);
    }

    #[test]
    fn selected_name_returns_the_item_at_schema_selected() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());
        sidebar.move_down();

        assert_eq!(sidebar.selected_name(), Some("orders"));
    }

    #[test]
    fn reset_clears_schema_selection_and_error() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());
        sidebar.set_schema_error("boom".to_string());

        sidebar.reset();

        assert_eq!(sidebar.schema, Vec::new());
        assert_eq!(sidebar.schema_selected, 0);
        assert!(sidebar.schema_error.is_none());
    }

    #[test]
    fn draw_shows_schema_items() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(vec![SchemaInfo::new("users".to_string())]);

        let (text, _) = draw_component(&mut sidebar, false);

        assert!(text.contains("users"), "buffer was: {text}");
    }

    #[test]
    fn draw_marks_the_panel_as_focused_with_the_border_color() {
        let mut sidebar = SchemaSidebarComponent::new();

        // The top-left corner of the border is what signals focus now that
        // the title no longer spells it out.
        let (_, focused) = draw_component(&mut sidebar, true);
        let (_, unfocused) = draw_component(&mut sidebar, false);

        assert_eq!(focused.cell((0, 0)).unwrap().fg, theme().border_focused);
        assert_eq!(unfocused.cell((0, 0)).unwrap().fg, theme().border);
    }

    #[test]
    fn draw_shows_the_entry_count_in_the_title() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());

        let (text, _) = draw_component(&mut sidebar, false);

        assert!(text.contains("Schema (2)"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_a_wrapped_schema_error() {
        let mut sidebar = SchemaSidebarComponent::new();
        let message =
            "failed to run SCAN against redis at 10.0.0.5:6379: connection timed out after 5s"
                .to_string();
        sidebar.set_schema_error(message.clone());

        let (_, buffer) = draw_component(&mut sidebar, false);

        // Error box is a Length(7) region at the bottom; inner text area is
        // 22 columns wide, 5 rows tall (24-wide sidebar minus borders).
        let region = Rect::new(1, 10 - 7 + 1, 22, 5);
        let wrapped = sidebar_text_in(&buffer, region);
        assert_eq!(wrapped, message, "buffer region was: {wrapped:?}");
    }

    fn sidebar_text_in(buffer: &Buffer, region: Rect) -> String {
        let mut rows = Vec::new();
        for y in region.y..region.y + region.height {
            let mut row = String::new();
            for x in region.x..region.x + region.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    row.push_str(cell.symbol());
                }
            }
            let trimmed = row.trim().to_string();
            if !trimmed.is_empty() {
                rows.push(trimmed);
            }
        }
        rows.join(" ")
    }

    #[test]
    fn draw_selection_highlight_tracks_schema_selected() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(vec![
            SchemaInfo::new("aaa".to_string()),
            SchemaInfo::new("bbb".to_string()),
            SchemaInfo::new("ccc".to_string()),
        ]);
        sidebar.move_down();
        assert_eq!(sidebar.schema_selected, 1);

        let (_, buffer) = draw_component(&mut sidebar, false);

        let unselected_cell = buffer.cell((1, 1)).unwrap();
        let selected_cell = buffer.cell((1, 2)).unwrap();
        assert_eq!(selected_cell.bg, theme().selection_bg);
        assert!(selected_cell.modifier.contains(Modifier::BOLD));
        assert_ne!(unselected_cell.bg, theme().selection_bg);
    }
}
