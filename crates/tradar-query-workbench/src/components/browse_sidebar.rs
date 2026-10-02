//! The Redis key-browser sidebar: lists every entry from `list_schema()`
//! alongside its `kind` (Redis: `TYPE`). A focused pane like `results.rs`,
//! not an overlay -- `QueryScreenComponent` resolves keys centrally and
//! calls these plain methods, the same way it drives `ResultsComponent`.
//! See "Redis: key browser" in `docs/backlog/mockup-ui-2026-08-15.md`.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use tradar_core::keymap::{Command, Context, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker, TextInput};
use tradar_core::vim_list::{self, VimMove};

use crate::query_driver::SchemaInfo;

/// What a left click on the sidebar means -- `QueryScreenComponent` acts on
/// this the same way it does `ResultsComponent::click`'s bool (focus the
/// pane), plus a second case for the row-activating click `Results` doesn't
/// have an equivalent of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseClick {
    /// The click missed the panel entirely.
    Missed,
    /// Landed on a row (or empty space below the list) -- worth focusing,
    /// nothing more.
    Selected,
    /// A second click on the same row within the double-click window --
    /// same as `Command::BrowseOpen` on it.
    Activated,
}

pub struct BrowseSidebarComponent {
    entries: Vec<SchemaInfo>,
    /// Set instead of `entries` when the initial `list_schema()` failed --
    /// same split as `ResultsComponent::last_error`, shown in place of the
    /// list rather than as an empty one indistinguishable from "no keys".
    error: Option<String>,
    selected: usize,
    visible_height: usize,
    list_state: ListState,
    list_area: Rect,
    double_click: DoubleClickTracker,
    /// A case-insensitive substring narrowing the key list -- same idiom
    /// as `NavigatorComponent::filter`/`KafkaScreen::filter`. Kept even
    /// while `filter_input` is closed so the title can still say what's
    /// applied and a fresh `/` prefills it rather than starting over.
    filter: String,
    /// `Some` while the filter bar has the keys -- see `open_filter`.
    filter_input: Option<TextInput>,
}

impl BrowseSidebarComponent {
    pub fn new(schema: &Result<Vec<SchemaInfo>, String>) -> Self {
        let (entries, error) = match schema {
            Ok(entries) => (entries.clone(), None),
            Err(error) => (Vec::new(), Some(error.clone())),
        };
        Self {
            entries,
            error,
            selected: 0,
            visible_height: 0,
            list_state: ListState::default(),
            list_area: Rect::ZERO,
            double_click: DoubleClickTracker::new(),
            filter: String::new(),
            filter_input: None,
        }
    }

    /// `entries` narrowed by `filter`, when one's applied -- same idiom as
    /// `NavigatorComponent::visible_rows`.
    fn visible_entries(&self) -> Vec<&SchemaInfo> {
        if self.filter.is_empty() {
            return self.entries.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&SchemaInfo> {
        self.visible_entries().get(self.selected).copied()
    }

    pub fn apply_move(&mut self, mv: VimMove) {
        let len = self.visible_entries().len();
        vim_list::apply(mv, &mut self.selected, len, self.visible_height);
    }

    /// Opens the filter bar, prefilled with whatever's already applied --
    /// same idiom as `KafkaScreen::open_filter`.
    pub fn open_filter(&mut self) {
        self.filter_input = Some(TextInput::new(&self.filter));
    }

    pub fn is_filtering(&self) -> bool {
        self.filter_input.is_some()
    }

    /// One key while the filter bar has the keys. `Esc` cancels -- clears
    /// the bar *and* whatever was applied. `Enter` keeps the filter and
    /// closes the bar. Anything else is text editing, applied live. Same
    /// shape as `KafkaScreen::filter_key_event`.
    pub fn filter_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) {
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

    /// See `BrowseClick`. A no-op (`Missed`) while `error` is showing --
    /// there's no list to click into then.
    pub fn click(&mut self, column: u16, row: u16) -> BrowseClick {
        if self.error.is_some() || !ui::contains(self.list_area, column, row) {
            return BrowseClick::Missed;
        }
        let inner = Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        };
        let len = self.visible_entries().len();
        let Some(index) = ui::index_at(inner, self.list_state.offset(), row, len) else {
            return BrowseClick::Selected;
        };
        self.selected = index;
        if self.double_click.click(index) {
            BrowseClick::Activated
        } else {
            BrowseClick::Selected
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = theme();
        let (area, filter_bar_area) = if self.filter_input.is_some() {
            let (list_area, bar) = ui::split_bottom_bar(area, 1);
            (list_area, Some(bar))
        } else {
            (area, None)
        };
        self.visible_height = area.height.saturating_sub(2) as usize;
        self.list_area = area;

        if let Some(error) = &self.error {
            let paragraph = Paragraph::new(error.as_str())
                .style(Style::default().fg(theme.error))
                .block(ui::panel("Keys", focused))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        let visible = self.visible_entries();
        let items: Vec<ListItem> = visible
            .iter()
            .map(|entry| {
                let kind = entry.kind.as_deref().unwrap_or("?");
                let mut spans = vec![
                    Span::styled(format!(" {}", entry.name), Style::default().fg(theme.text)),
                    Span::styled(format!("  {kind}"), Style::default().fg(theme.text_dim)),
                ];
                if let Some(ttl) = entry.ttl {
                    spans.push(Span::styled(
                        format!("  ttl:{ttl}s"),
                        Style::default().fg(theme.text_dim),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let visible_len = visible.len();
        drop(visible);
        if visible_len > 0 {
            self.list_state.select(Some(self.selected));
        }

        let title = if self.filter.is_empty() {
            let open = keymap()
                .binding_for(Context::Browse, Command::BrowseOpen)
                .unwrap_or_default();
            format!("Keys ({visible_len}) — {open} open")
        } else {
            format!("Keys ({visible_len}) — filter: {}", self.filter)
        };
        let list = List::new(items)
            .block(ui::panel(&title, focused))
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
    use super::*;

    fn entry(name: &str, kind: &str) -> SchemaInfo {
        SchemaInfo {
            name: name.to_string(),
            columns: Vec::new(),
            kind: Some(kind.to_string()),
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }

    fn sidebar() -> BrowseSidebarComponent {
        BrowseSidebarComponent::new(&Ok(vec![
            entry("user:1", "hash"),
            entry("greeting", "string"),
        ]))
    }

    #[test]
    fn starts_selecting_the_first_entry() {
        let sidebar = sidebar();
        assert_eq!(sidebar.selected_entry(), Some(&entry("user:1", "hash")));
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_entry() {
        let mut sidebar = sidebar();

        sidebar.apply_move(VimMove::Down);
        assert_eq!(sidebar.selected_entry(), Some(&entry("greeting", "string")));

        sidebar.apply_move(VimMove::Down);
        assert_eq!(sidebar.selected_entry(), Some(&entry("greeting", "string")));
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut sidebar = sidebar();

        sidebar.apply_move(VimMove::Up);
        assert_eq!(sidebar.selected_entry(), Some(&entry("user:1", "hash")));
    }

    #[test]
    fn selected_entry_on_an_empty_list_is_none() {
        let sidebar = BrowseSidebarComponent::new(&Ok(Vec::new()));

        assert_eq!(sidebar.selected_entry(), None);
    }

    #[test]
    fn a_failed_schema_load_carries_no_entries() {
        let sidebar = BrowseSidebarComponent::new(&Err("scan failed".to_string()));

        assert_eq!(sidebar.selected_entry(), None);
        assert_eq!(sidebar.error.as_deref(), Some("scan failed"));
    }

    #[test]
    fn filter_narrows_visible_entries_case_insensitively() {
        let mut sidebar = BrowseSidebarComponent::new(&Ok(vec![
            entry("user:1", "hash"),
            entry("payment:1", "string"),
            entry("USER:2", "hash"),
        ]));

        sidebar.filter = "user".to_string();
        let names: Vec<&str> = sidebar
            .visible_entries()
            .into_iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["user:1", "USER:2"]);
    }

    #[test]
    fn esc_clears_the_filter_and_resets_the_cursor() {
        let mut sidebar = sidebar();

        sidebar.open_filter();
        assert!(sidebar.is_filtering());
        for c in "greet".chars() {
            sidebar.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(sidebar.filter, "greet");
        assert_eq!(sidebar.visible_entries().len(), 1);
        sidebar.selected = 5;

        sidebar.filter_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(!sidebar.is_filtering());
        assert!(sidebar.filter.is_empty());
        assert_eq!(sidebar.visible_entries().len(), 2);
        assert_eq!(sidebar.selected, 0);
    }

    #[test]
    fn enter_keeps_the_filter_applied_and_closes_the_bar() {
        let mut sidebar = sidebar();

        sidebar.open_filter();
        for c in "greet".chars() {
            sidebar.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        sidebar.filter_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(!sidebar.is_filtering());
        assert_eq!(sidebar.filter, "greet");
        assert_eq!(sidebar.selected_entry(), Some(&entry("greeting", "string")));
    }

    #[test]
    fn open_filter_prefills_whatever_was_already_applied() {
        let mut sidebar = sidebar();
        sidebar.filter = "greet".to_string();

        sidebar.open_filter();

        assert_eq!(
            sidebar.filter_input.as_ref().map(|input| input.text()),
            Some("greet".to_string())
        );
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut sidebar = sidebar();
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        // Row 0 is the border, row 2 is "greeting" (the second entry).
        let click = sidebar.click(2, 2);

        assert_eq!(click, BrowseClick::Selected);
        assert_eq!(sidebar.selected_entry(), Some(&entry("greeting", "string")));
    }

    #[test]
    fn double_clicking_a_row_activates_it() {
        let mut sidebar = sidebar();
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        sidebar.click(2, 2);
        let click = sidebar.click(2, 2);

        assert_eq!(click, BrowseClick::Activated);
    }

    #[test]
    fn a_click_outside_the_panel_is_reported_as_missed() {
        let mut sidebar = sidebar();
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        let click = sidebar.click(100, 100);

        assert_eq!(click, BrowseClick::Missed);
    }

    #[test]
    fn a_click_while_showing_an_error_is_always_missed() {
        let mut sidebar = BrowseSidebarComponent::new(&Err("scan failed".to_string()));
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        let click = sidebar.click(2, 2);

        assert_eq!(click, BrowseClick::Missed);
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn a_key_with_a_ttl_shows_it_next_to_its_type() {
        let mut sidebar = BrowseSidebarComponent::new(&Ok(vec![SchemaInfo {
            name: "session:abc".to_string(),
            columns: Vec::new(),
            kind: Some("string".to_string()),
            ttl: Some(3421),
            schema: None,
            object_kind: None,
        }]));

        let backend = ratatui::backend::TestBackend::new(40, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("ttl:3421s"), "buffer was: {text}");
    }

    #[test]
    fn a_key_with_no_ttl_shows_nothing_extra() {
        let mut sidebar = sidebar();

        let backend = ratatui::backend::TestBackend::new(40, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| sidebar.draw(frame, frame.area(), true))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(!text.contains("ttl:"), "buffer was: {text}");
    }
}
