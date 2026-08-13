//! Shared widget helpers, so every panel in the app looks the same and is
//! themed from one place: bordered panels, the selection highlight, the
//! bottom hint bar, centered overlay geometry, and the help overlay.
//!
//! Lives in `tradar-core` because both `tradar-app` (picker, tabs) and
//! `tradar-query-workbench` (editor, results, sidebar, overlays) draw
//! these, and neither may depend on the other.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::keymap::{Command, Context, keymap};
use crate::theme::theme;
use crate::vim_list;

/// A bordered panel with a themed title. `focused` brightens the border and
/// title so the keyboard focus is obvious at a glance -- the only cue a TUI
/// has for "keys go here".
pub fn panel(title: &str, focused: bool) -> Block<'static> {
    let theme = theme();
    let (border, title_color) = if focused {
        (theme.border_focused, theme.title_focused)
    } else {
        (theme.border, theme.title)
    };
    let title = if focused {
        Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {title} "), Style::default().fg(title_color))
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(title)
}

/// The highlight for the selected row in any list.
pub fn selection_style() -> Style {
    let theme = theme();
    Style::default()
        .bg(theme.selection_bg)
        .fg(theme.selection_fg)
        .add_modifier(Modifier::BOLD)
}

/// A centered rect covering `percent_x` × `percent_y` of `area` -- the
/// geometry every overlay (file prompt, history, help) is drawn into.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// One `key: label` pair in the status bar.
pub struct Hint {
    pub key: String,
    pub label: &'static str,
}

/// Looks up `command`'s current binding in `context` -- so the hint bar
/// shows the user's own keys, not the defaults, after a remap. Returns
/// `None` when the command has been unbound, in which case there's nothing
/// worth hinting.
pub fn hint(context: Context, command: Command, label: &'static str) -> Option<Hint> {
    keymap()
        .binding_for(context, command)
        .map(|key| Hint { key, label })
}

/// The one-line bar along the bottom of the screen: `key label   key label`,
/// with an optional right-aligned status (connection name, row count, ...).
pub fn draw_status_bar(frame: &mut Frame, area: Rect, hints: &[Hint], right: Option<&str>) {
    let theme = theme();
    let mut spans = Vec::new();
    for hint in hints {
        spans.push(Span::styled(
            format!(" {} ", hint.key),
            Style::default()
                .fg(theme.status_key)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{}  ", hint.label),
            Style::default().fg(theme.status_bar_fg),
        ));
    }

    // `Paragraph::style` applies to the whole widget area, so this paints
    // the bar's background across the full width, not just behind the text.
    let background = Style::default().bg(theme.status_bar_bg);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(background), area);

    if let Some(right) = right {
        let width = right.chars().count() as u16 + 1;
        if area.width > width {
            let right_area = Rect {
                x: area.x + area.width - width,
                y: area.y,
                width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    right.to_string(),
                    Style::default().fg(theme.accent),
                ))
                .style(background),
                right_area,
            );
        }
    }
}

/// The `?` overlay: every binding currently in effect, grouped by context.
/// Built from the live `Keymap`, so a remapped key shows up here without
/// anyone maintaining a second copy of the list.
#[derive(Default)]
pub struct HelpOverlay {
    scroll: usize,
    pending: Option<crate::keymap::KeyPress>,
    visible_height: usize,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles a key while the overlay is up. Returns `true` when the
    /// overlay should close -- which anything that isn't list navigation
    /// does, including `?` itself, so the key that opened it dismisses it.
    pub fn handle_key_event(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        let key = crate::keymap::KeyPress::new(code, modifiers);
        match keymap().resolve(Context::List, &mut self.pending, key) {
            crate::keymap::Resolution::Command(command) => match command.as_vim_move() {
                Some(mv) => {
                    let lines = self.lines().len();
                    vim_list::apply(mv, &mut self.scroll, lines, self.visible_height);
                    false
                }
                None => true,
            },
            // Mid-way through a two-key sequence (the first `g` of `gg`).
            crate::keymap::Resolution::Pending => false,
            crate::keymap::Resolution::None => true,
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let theme = theme();
        let keymap = keymap();
        let mut lines = Vec::new();
        for context in Context::all() {
            let bindings = keymap.bindings(context);
            if bindings.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                context_title(context).to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for (binding, command) in bindings {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<12}", binding.display()),
                        Style::default()
                            .fg(theme.status_key)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        command.description().to_string(),
                        Style::default().fg(theme.text),
                    ),
                ]));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Editor keys (fixed): i a I A o O  h j k l  gg G  0 $  x  dd  ctrl-d ctrl-u",
            Style::default().fg(theme.text_dim),
        )));
        lines.push(Line::from(Span::styled(
            "  Rebind anything above in ~/.config/tradar/config.toml",
            Style::default().fg(theme.text_dim),
        )));
        lines
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(70, 80, area);
        frame.render_widget(Clear, popup);

        let block = panel("Keys — any key to close, j/k to scroll", true);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        self.visible_height = inner.height as usize;
        let lines = self.lines();
        self.scroll = self.scroll.min(lines.len().saturating_sub(1));

        let items: Vec<ListItem> = lines
            .into_iter()
            .skip(self.scroll)
            .take(inner.height as usize)
            .map(ListItem::new)
            .collect();
        // A plain `List` with no selection: the overlay scrolls, but there's
        // nothing here to act on, so highlighting a "current" line would
        // imply an Enter that doesn't exist.
        frame.render_stateful_widget(List::new(items), inner, &mut ListState::default());
    }
}

fn context_title(context: Context) -> &'static str {
    match context {
        Context::Global => "Anywhere",
        Context::Picker => "Connection picker",
        Context::QueryScreen => "Query screen",
        Context::List => "Lists (connections, schema, results, history)",
        Context::Prompt => "Prompts and overlays",
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn centered_rect_is_centered_and_smaller_than_its_area() {
        let area = Rect::new(0, 0, 100, 100);

        let centered = centered_rect(50, 50, area);

        assert_eq!(centered.width, 50);
        assert_eq!(centered.height, 50);
        assert_eq!(centered.x, 25);
        assert_eq!(centered.y, 25);
    }

    #[test]
    fn hint_reports_the_binding_currently_in_effect() {
        let hint = hint(Context::Global, Command::Quit, "quit").unwrap();

        assert_eq!(hint.key, "ctrl-q");
        assert_eq!(hint.label, "quit");
    }

    #[test]
    fn the_status_bar_shows_each_hint_and_the_right_hand_status() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let hints = vec![
            hint(Context::Global, Command::Quit, "quit").unwrap(),
            hint(Context::List, Command::MoveDown, "move").unwrap(),
        ];

        terminal
            .draw(|frame| draw_status_bar(frame, frame.area(), &hints, Some("local-sqlite")))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text = buffer_text(buffer);
        assert!(text.contains("ctrl-q quit"), "buffer was: {text}");
        assert!(text.contains("j move"), "buffer was: {text}");
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert_eq!(
            buffer.cell((0, 0)).unwrap().bg,
            theme().status_bar_bg,
            "the bar's background should span the full width"
        );
    }

    #[test]
    fn a_status_bar_hint_is_skipped_when_its_command_is_unbound() {
        // `binding_for` returning `None` is how an unbound command shows up;
        // there's nothing useful to hint in that case.
        let mut keymap = crate::keymap::Keymap::default();
        let mut overrides = std::collections::HashMap::new();
        let mut commands = std::collections::HashMap::new();
        commands.insert("quit".to_string(), Vec::new());
        overrides.insert("global".to_string(), commands);
        keymap.apply_overrides(&overrides).unwrap();

        assert_eq!(keymap.binding_for(Context::Global, Command::Quit), None);
    }

    #[test]
    fn the_help_overlay_lists_every_context_and_its_bindings() {
        let mut help = HelpOverlay::new();
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| help.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Anywhere"), "buffer was: {text}");
        assert!(text.contains("ctrl-q"), "buffer was: {text}");
        assert!(text.contains("Quit tradar"), "buffer was: {text}");
    }

    #[test]
    fn navigation_keys_scroll_the_help_overlay_and_anything_else_closes_it() {
        let mut help = HelpOverlay::new();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| help.draw(frame, frame.area()))
            .unwrap();

        let closed_on_j = help.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!closed_on_j, "j should scroll, not close");
        assert_eq!(help.scroll, 1);

        let closed_on_esc = help.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert!(closed_on_esc);
    }
}
