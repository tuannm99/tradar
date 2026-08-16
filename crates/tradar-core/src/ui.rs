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

/// The cell cursor in a grid, on top of the selected row's own highlight.
/// Reversed rather than another color of its own: it's the same trick
/// `TextInput` uses to draw a cursor a terminal won't, and it stays visible
/// whatever `selection_bg` a user's theme picks.
pub fn cell_cursor_style() -> Style {
    selection_style().add_modifier(Modifier::REVERSED)
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

/// Carves a `height`-row bar off the bottom of `area` for a one-line
/// input/echo strip (a filter bar, a status line) below whatever fills the
/// rest -- returns `(content, bar)`. Shared so this exact split doesn't get
/// hand-rolled again per panel that wants one.
pub fn split_bottom_bar(area: Rect, height: u16) -> (Rect, Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(height)])
        .split(area);
    (rows[0], rows[1])
}

/// Whether `area` covers the given cell -- the hit test every mouse
/// handler starts with.
pub fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

/// Which list index sits at screen row `row`, for a list drawn into
/// `inner` scrolled by `offset`. `None` when the click is outside the list
/// or past its last item -- clicking empty space below the rows shouldn't
/// jump the selection to the end.
pub fn index_at(inner: Rect, offset: usize, row: u16, len: usize) -> Option<usize> {
    if row < inner.y || row >= inner.y.saturating_add(inner.height) {
        return None;
    }
    let index = offset + (row - inner.y) as usize;
    (index < len).then_some(index)
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

/// A single-line text field: the editing half of any prompt or form
/// (`Ctrl+S`'s file path, the connection form's fields). Owns only the
/// text and cursor -- confirming, cancelling and drawing a frame around it
/// belong to whoever hosts it, since those differ per prompt.
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    chars: Vec<char>,
    cursor: usize,
}

impl TextInput {
    pub fn new(initial: &str) -> Self {
        let chars: Vec<char> = initial.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Handles one key of text editing. Returns whether the key was used,
    /// so the host can tell "the field consumed this" from "this is mine to
    /// interpret". Deliberately fixed (not remappable): these are the
    /// editing keys every terminal field has, and a user who rebinds
    /// `backspace` has bigger problems.
    pub fn handle_key_event(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        match code {
            KeyCode::Char(c)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
                true
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.chars.remove(self.cursor);
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
                true
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.chars.len());
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.chars.len();
                true
            }
            _ => false,
        }
    }

    /// The field as styled spans. `focused` draws a block cursor -- a
    /// terminal hides the real one, so without this you can't see where
    /// you're typing.
    pub fn spans(&self, focused: bool) -> Vec<Span<'static>> {
        let theme = theme();
        let mut spans: Vec<Span<'static>> = self
            .chars
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let style = Style::default().fg(theme.text);
                if focused && i == self.cursor {
                    Span::styled(c.to_string(), style.add_modifier(Modifier::REVERSED))
                } else {
                    Span::styled(c.to_string(), style)
                }
            })
            .collect();
        if focused && self.cursor >= self.chars.len() {
            spans.push(Span::styled(
                " ",
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }
        spans
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
        Context::Navigator => "Database navigator (when focused)",
        Context::Results => "Results pane (when focused)",
        Context::Editor => "Query editor (when focused)",
        Context::Browse => "Redis key browser (when focused)",
        Context::Rabbit => "RabbitMQ screen",
        Context::Kafka => "Kafka screen",
        Context::List => "Lists (connections, schema, results, history)",
        Context::Prompt => "Prompts and overlays",
        Context::Completion => "Autocomplete (while suggestions show)",
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
    fn index_at_maps_a_click_row_to_a_list_index() {
        let inner = Rect::new(0, 5, 20, 4);

        assert_eq!(index_at(inner, 0, 5, 10), Some(0), "first visible row");
        assert_eq!(index_at(inner, 0, 7, 10), Some(2));
        assert_eq!(
            index_at(inner, 3, 5, 10),
            Some(3),
            "a scrolled list offsets the answer"
        );
    }

    #[test]
    fn index_at_ignores_clicks_outside_the_rows() {
        let inner = Rect::new(0, 5, 20, 4);

        assert_eq!(index_at(inner, 0, 4, 10), None, "above the list");
        assert_eq!(index_at(inner, 0, 9, 10), None, "below the list");
        assert_eq!(
            index_at(inner, 0, 8, 2),
            None,
            "empty space past the last item must not select the last item"
        );
    }

    #[test]
    fn contains_is_inclusive_of_the_top_left_and_exclusive_of_the_far_edge() {
        let area = Rect::new(2, 3, 4, 5);

        assert!(contains(area, 2, 3));
        assert!(contains(area, 5, 7));
        assert!(!contains(area, 6, 7), "one past the right edge");
        assert!(!contains(area, 5, 8), "one past the bottom edge");
        assert!(!contains(area, 1, 3));
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
