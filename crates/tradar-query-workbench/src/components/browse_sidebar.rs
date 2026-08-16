//! The Redis key-browser sidebar: lists every entry from `list_schema()`
//! alongside its `kind` (Redis: `TYPE`). A focused pane like `results.rs`,
//! not an overlay -- `QueryScreenComponent` resolves keys centrally and
//! calls these plain methods, the same way it drives `ResultsComponent`.
//! See "Redis: key browser" in `docs/backlog.md`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use tradar_core::keymap::{Command, Context, keymap};
use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list::{self, VimMove};

use crate::query_driver::SchemaInfo;

pub struct BrowseSidebarComponent {
    entries: Vec<SchemaInfo>,
    /// Set instead of `entries` when the initial `list_schema()` failed --
    /// same split as `ResultsComponent::last_error`, shown in place of the
    /// list rather than as an empty one indistinguishable from "no keys".
    error: Option<String>,
    selected: usize,
    visible_height: usize,
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
        }
    }

    pub fn selected_entry(&self) -> Option<&SchemaInfo> {
        self.entries.get(self.selected)
    }

    pub fn apply_move(&mut self, mv: VimMove) {
        vim_list::apply(
            mv,
            &mut self.selected,
            self.entries.len(),
            self.visible_height,
        );
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = theme();
        self.visible_height = area.height.saturating_sub(2) as usize;

        if let Some(error) = &self.error {
            let paragraph = Paragraph::new(error.as_str())
                .style(Style::default().fg(theme.error))
                .block(ui::panel("Keys", focused))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        let items: Vec<ListItem> = self
            .entries
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

        let mut state = ListState::default();
        if !self.entries.is_empty() {
            state.select(Some(self.selected));
        }

        let open = keymap()
            .binding_for(Context::Browse, Command::BrowseOpen)
            .unwrap_or_default();
        let title = format!("Keys ({}) — {open} open", self.entries.len());
        let list = List::new(items)
            .block(ui::panel(&title, focused))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut state);
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
