//! The schema (table/collection/index/key) sidebar on the query screen.
//! Not a `Component` — driven entirely by `QueryScreenComponent`, which
//! owns whether it currently has keyboard focus.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::query_driver::SchemaInfo;

pub struct SchemaSidebarComponent {
    pub schema: Vec<SchemaInfo>,
    pub schema_selected: usize,
    pub schema_error: Option<String>,
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
            visible_height: 0,
        }
    }

    pub fn set_schema(&mut self, schema: Vec<SchemaInfo>) {
        self.schema = schema;
        self.schema_selected = 0;
        self.schema_error = None;
    }

    pub fn set_schema_error(&mut self, error: String) {
        self.schema_error = Some(error);
    }

    fn move_down_by(&mut self, delta: usize) {
        if self.schema.is_empty() {
            return;
        }
        self.schema_selected = (self.schema_selected + delta).min(self.schema.len() - 1);
    }

    fn move_up_by(&mut self, delta: usize) {
        self.schema_selected = self.schema_selected.saturating_sub(delta);
    }

    pub fn move_down(&mut self) {
        self.move_down_by(1);
    }

    pub fn move_up(&mut self) {
        self.move_up_by(1);
    }

    pub fn move_to_top(&mut self) {
        self.schema_selected = 0;
    }

    pub fn move_to_bottom(&mut self) {
        self.schema_selected = self.schema.len().saturating_sub(1);
    }

    /// Half the last-rendered visible row count, minimum 1 -- matches vim's
    /// `Ctrl+d`/`Ctrl+u`. Falls back to 1 before the first `draw()`.
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

    pub fn selected_name(&self) -> Option<&str> {
        self.schema
            .get(self.schema_selected)
            .map(|s| s.name.as_str())
    }

    pub fn reset(&mut self) {
        self.schema = Vec::new();
        self.schema_selected = 0;
        self.schema_error = None;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let items: Vec<ListItem> = self
            .schema
            .iter()
            .map(|entry| ListItem::new(entry.name.clone()))
            .collect();

        let mut state = ListState::default();
        if !self.schema.is_empty() {
            state.select(Some(self.schema_selected));
        }

        let title = if focused {
            "Schema [focused]"
        } else {
            "Schema"
        };

        let Some(error) = &self.schema_error else {
            self.visible_height = area.height.saturating_sub(2) as usize;
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(title))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            frame.render_stateful_widget(list, area, &mut state);
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(7)])
            .split(area);
        self.visible_height = chunks[0].height.saturating_sub(2) as usize;

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, chunks[0], &mut state);

        let error_box = Paragraph::new(error.as_str())
            .block(Block::default().borders(Borders::ALL).title("Error"))
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
    use crate::query_driver::SchemaInfo;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo {
                name: "users".to_string(),
            },
            SchemaInfo {
                name: "orders".to_string(),
            },
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
            SchemaInfo {
                name: "a".to_string(),
            },
            SchemaInfo {
                name: "b".to_string(),
            },
            SchemaInfo {
                name: "c".to_string(),
            },
            SchemaInfo {
                name: "d".to_string(),
            },
            SchemaInfo {
                name: "e".to_string(),
            },
            SchemaInfo {
                name: "f".to_string(),
            },
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
        sidebar.set_schema(vec![SchemaInfo {
            name: "users".to_string(),
        }]);

        let (text, _) = draw_component(&mut sidebar, false);

        assert!(text.contains("users"), "buffer was: {text}");
    }

    #[test]
    fn draw_marks_the_title_as_focused() {
        let mut sidebar = SchemaSidebarComponent::new();

        let (text, _) = draw_component(&mut sidebar, true);

        assert!(text.contains("Schema [focused]"), "buffer was: {text}");
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
            SchemaInfo {
                name: "aaa".to_string(),
            },
            SchemaInfo {
                name: "bbb".to_string(),
            },
            SchemaInfo {
                name: "ccc".to_string(),
            },
        ]);
        sidebar.move_down();
        assert_eq!(sidebar.schema_selected, 1);

        let (_, buffer) = draw_component(&mut sidebar, false);

        let unselected_cell = buffer.cell((1, 1)).unwrap();
        let selected_cell = buffer.cell((1, 2)).unwrap();
        assert!(selected_cell.modifier.contains(Modifier::REVERSED));
        assert!(!unselected_cell.modifier.contains(Modifier::REVERSED));
    }
}
