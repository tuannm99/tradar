//! Shared widget helpers, so every panel in the app looks the same and is
//! themed from one place: bordered panels, the selection highlight, the
//! bottom hint bar, centered overlay geometry, and the help overlay.
//!
//! Lives in `tradar-core` because both `tradar-app` (picker, tabs) and
//! `tradar-query-workbench` (editor, results, sidebar, overlays) draw
//! these, and neither may depend on the other.

use std::io::Write;

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

/// Which side of a [`SplitPane`] a pane is on -- stacked (top/bottom) or
/// side by side (left/right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOrientation {
    Vertical,
    Horizontal,
}

/// Neither pane may shrink past this share of the split -- it stays
/// visible and usable, just small. "Zoom" resizes, it never fully hides
/// the other pane (a deliberate choice -- see "Query/HTTP screen: resizable
/// split" in docs/architecture.md).
const MIN_SPLIT_PERCENT: i16 = 20;
const MAX_SPLIT_PERCENT: i16 = 80;
const ZOOM_STEP_PERCENT: i16 = 10;

/// A two-pane split (query editor/results, HTTP request/response) that can
/// flip between stacked and side-by-side and can be zoomed -- growing
/// whichever pane currently has focus, shrinking the other. Lives here
/// (not in `tradar-query-workbench` or `tradar-connector-http`) because
/// both want the exact same behavior and neither may depend on the other.
#[derive(Debug, Clone, Copy)]
pub struct SplitPane {
    orientation: SplitOrientation,
    /// Percent of the split given to the "primary" pane (the query editor,
    /// the HTTP request builder) -- the other pane always gets the rest.
    primary_percent: i16,
}

impl Default for SplitPane {
    fn default() -> Self {
        Self {
            orientation: SplitOrientation::Vertical,
            primary_percent: 50,
        }
    }
}

impl SplitPane {
    pub fn toggle_orientation(&mut self) {
        self.orientation = match self.orientation {
            SplitOrientation::Vertical => SplitOrientation::Horizontal,
            SplitOrientation::Horizontal => SplitOrientation::Vertical,
        };
    }

    /// Grows whichever pane currently has focus by one step, shrinking the
    /// other -- `focus_is_primary` says which pane that is. Clamped so
    /// neither pane ever drops below [`MIN_SPLIT_PERCENT`].
    pub fn zoom_in(&mut self, focus_is_primary: bool) {
        self.adjust(if focus_is_primary {
            ZOOM_STEP_PERCENT
        } else {
            -ZOOM_STEP_PERCENT
        });
    }

    /// Undoes one zoom step for whichever pane has focus -- shrinks the
    /// focused pane back toward center.
    pub fn zoom_out(&mut self, focus_is_primary: bool) {
        self.adjust(if focus_is_primary {
            -ZOOM_STEP_PERCENT
        } else {
            ZOOM_STEP_PERCENT
        });
    }

    fn adjust(&mut self, delta: i16) {
        self.primary_percent =
            (self.primary_percent + delta).clamp(MIN_SPLIT_PERCENT, MAX_SPLIT_PERCENT);
    }

    /// Splits `area` into `(primary, secondary)` per the current
    /// orientation/ratio.
    pub fn split(&self, area: Rect) -> (Rect, Rect) {
        let direction = match self.orientation {
            SplitOrientation::Vertical => Direction::Vertical,
            SplitOrientation::Horizontal => Direction::Horizontal,
        };
        let percent = self.primary_percent as u16;
        let chunks = Layout::default()
            .direction(direction)
            .constraints([
                Constraint::Percentage(percent),
                Constraint::Percentage(100 - percent),
            ])
            .split(area);
        (chunks[0], chunks[1])
    }
}

/// A right-click popup: a small list of `(label, Command)` at the click
/// point. Not keymap-integrated -- these are direct actions the caller
/// already decided are relevant to whatever was clicked (a result row, a
/// navigator entry), not remappable bindings, so confirming one just hands
/// back the same `Command` a keyboard shortcut for that action would have
/// produced, for the caller to dispatch through its own existing command
/// handling -- no separate mouse-only code path to keep in sync.
pub struct ContextMenu {
    items: Vec<(String, Command)>,
    selected: usize,
    origin: (u16, u16),
}

/// What happened after a key reached an open [`ContextMenu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuOutcome {
    /// Still open (moved the selection, or an unrecognized key -- any key
    /// not explicitly handled just does nothing rather than falling
    /// through to whatever's behind the menu).
    Open,
    Confirmed(Command),
    Closed,
}

impl ContextMenu {
    /// `origin` is the click point the menu should appear at (clamped to
    /// stay on screen in `draw`/`click`). Empty `items` is a valid, useful
    /// state -- nothing relevant was clicked, so `is_empty` tells the
    /// caller not to bother opening it.
    pub fn new(origin: (u16, u16), items: Vec<(String, Command)>) -> Self {
        Self {
            items,
            selected: 0,
            origin,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn handle_key_event(&mut self, code: crossterm::event::KeyCode) -> ContextMenuOutcome {
        use crossterm::event::KeyCode;
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                ContextMenuOutcome::Open
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                ContextMenuOutcome::Open
            }
            KeyCode::Enter => ContextMenuOutcome::Confirmed(self.items[self.selected].1),
            KeyCode::Esc => ContextMenuOutcome::Closed,
            _ => ContextMenuOutcome::Open,
        }
    }

    /// A click at `(column, row)` within `bounds` (the full screen/panel
    /// area the menu was drawn into) -- `Some(command)` on a hit, `None`
    /// otherwise (the caller should close the menu: clicking away from it
    /// dismisses it, standard popup behavior).
    pub fn click(&self, bounds: Rect, column: u16, row: u16) -> Option<Command> {
        let rect = self.rect(bounds);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        index_at(inner, 0, row, self.items.len())
            .filter(|_| column >= inner.x && column < inner.x.saturating_add(inner.width))
            .map(|i| self.items[i].1)
    }

    fn rect(&self, bounds: Rect) -> Rect {
        let width = (self
            .items
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(0) as u16
            + 4)
        .min(bounds.width);
        let height = (self.items.len() as u16 + 2).min(bounds.height);
        let x = self.origin.0.min(bounds.width.saturating_sub(width));
        let y = self.origin.1.min(bounds.height.saturating_sub(height));
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn draw(&self, frame: &mut Frame, bounds: Rect) {
        let rect = self.rect(bounds);
        frame.render_widget(Clear, rect);
        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|(label, _)| ListItem::new(label.as_str()))
            .collect();
        let mut state = ListState::default();
        state.select(Some(self.selected));
        let list = List::new(items)
            .block(panel("", true))
            .highlight_style(selection_style());
        frame.render_stateful_widget(list, rect, &mut state);
    }
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

/// Copies `text` to the system clipboard via an OSC52 escape sequence,
/// which the terminal emulator itself intercepts -- no clipboard crate
/// needed, and it works through SSH/tmux as long as the terminal supports
/// OSC52 (most modern ones do: iTerm2, kitty, Alacritty, WezTerm, Windows
/// Terminal, ...). Lives here (not `tradar-query-workbench`, its original
/// home) because bespoke non-query screens (HTTP, RabbitMQ...) want to yank
/// too, and connector crates may not depend on `tradar-query-workbench`.
pub fn yank_to_clipboard(text: &str) {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let sequence = format!("\x1b]52;c;{encoded}\x07");
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(sequence.as_bytes());
    let _ = stdout.flush();
}

/// Reads text off the system clipboard, for middle-click paste. `None` on
/// any failure -- no clipboard available (headless/SSH with no display
/// forwarding is common for a terminal app), or its contents aren't text
/// -- middle-click quietly doing nothing is a better failure mode here
/// than an error box for something this incidental. Unlike
/// `yank_to_clipboard` (OSC52, one-way, works over SSH/tmux with no local
/// dependency) this reads the real OS clipboard via `arboard`, since OSC52
/// has no read side a terminal can answer.
pub fn paste_from_clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
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

    /// Inserts `text` at the cursor -- middle-click paste, mainly. Newlines
    /// are dropped: this is a single-line field, and pasted text (a
    /// clipboard URL, say) isn't expected to carry any.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars().filter(|c| *c != '\n' && *c != '\r') {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
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

/// A plain multi-line text box: headers/body fields for the HTTP screen's
/// Postman-style request builder, request/response bodies for gRPC. Not
/// vim-modal (`QueryEditorComponent` is, but that's a SQL-specific editor
/// living in `tradar-query-workbench`, which connector crates may not
/// depend on -- see "Thiết kế UI: HTTP, gRPC, Socket" in
/// docs/architecture.md) -- always-insert like `TextInput`, just with
/// newlines, since a header/JSON-body box has no vim power-user audience
/// to justify modal editing.
#[derive(Debug, Default, Clone)]
pub struct TextArea {
    lines: Vec<Vec<char>>,
    cursor_row: usize,
    cursor_col: usize,
}

impl TextArea {
    pub fn new(initial: &str) -> Self {
        let mut lines: Vec<Vec<char>> = initial.lines().map(|l| l.chars().collect()).collect();
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        let cursor_row = lines.len() - 1;
        let cursor_col = lines[cursor_row].len();
        Self {
            lines,
            cursor_row,
            cursor_col,
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn set_text(&mut self, text: &str) {
        *self = Self::new(text);
    }

    /// Inserts `text` at the cursor -- middle-click paste, mainly. A `\n`
    /// starts a new line, same as pressing Enter while typing it, so a
    /// multi-line clipboard paste (a JSON body, say) lands as real lines
    /// rather than one line with embedded newline characters.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => {
                    let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
                    self.lines.insert(self.cursor_row + 1, rest);
                    self.cursor_row += 1;
                    self.cursor_col = 0;
                }
                '\r' => {}
                c => {
                    self.lines[self.cursor_row].insert(self.cursor_col, c);
                    self.cursor_col += 1;
                }
            }
        }
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    /// Handles one key of text editing. Same "did this field consume the
    /// key" contract as `TextInput::handle_key_event`.
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
                self.lines[self.cursor_row].insert(self.cursor_col, c);
                self.cursor_col += 1;
                true
            }
            KeyCode::Enter => {
                let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
                self.lines.insert(self.cursor_row + 1, rest);
                self.cursor_row += 1;
                self.cursor_col = 0;
                true
            }
            KeyCode::Backspace => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    self.lines[self.cursor_row].remove(self.cursor_col);
                } else if self.cursor_row > 0 {
                    let current = self.lines.remove(self.cursor_row);
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].len();
                    self.lines[self.cursor_row].extend(current);
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor_col < self.lines[self.cursor_row].len() {
                    self.lines[self.cursor_row].remove(self.cursor_col);
                } else if self.cursor_row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.cursor_row + 1);
                    self.lines[self.cursor_row].extend(next);
                }
                true
            }
            KeyCode::Left => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].len();
                }
                true
            }
            KeyCode::Right => {
                if self.cursor_col < self.lines[self.cursor_row].len() {
                    self.cursor_col += 1;
                } else if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.cursor_col = 0;
                }
                true
            }
            KeyCode::Up => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                }
                true
            }
            KeyCode::Down => {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                }
                true
            }
            KeyCode::Home => {
                self.cursor_col = 0;
                true
            }
            KeyCode::End => {
                self.cursor_col = self.lines[self.cursor_row].len();
                true
            }
            _ => false,
        }
    }

    /// Scroll offset that keeps the cursor row on screen for a viewport
    /// `visible_height` rows tall.
    pub fn scroll_offset(&self, visible_height: usize) -> u16 {
        if visible_height == 0 || self.cursor_row < visible_height {
            return 0;
        }
        (self.cursor_row - visible_height + 1) as u16
    }

    /// The field as styled lines. `focused` draws a block cursor, same
    /// reasoning as `TextInput::spans`.
    pub fn styled_lines(&self, focused: bool) -> Vec<Line<'static>> {
        let theme = theme();
        self.lines
            .iter()
            .enumerate()
            .map(|(row, line)| {
                let mut spans: Vec<Span<'static>> = line
                    .iter()
                    .enumerate()
                    .map(|(col, c)| {
                        let style = Style::default().fg(theme.text);
                        if focused && row == self.cursor_row && col == self.cursor_col {
                            Span::styled(c.to_string(), style.add_modifier(Modifier::REVERSED))
                        } else {
                            Span::styled(c.to_string(), style)
                        }
                    })
                    .collect();
                if focused && row == self.cursor_row && self.cursor_col >= line.len() {
                    spans.push(Span::styled(
                        " ",
                        Style::default().add_modifier(Modifier::REVERSED),
                    ));
                }
                Line::from(spans)
            })
            .collect()
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
        Context::Http => "HTTP screen",
        Context::HttpResponse => "HTTP screen — response pane (when focused)",
        Context::HttpRequests => "Saved-request library (while open)",
        Context::List => "Lists (connections, schema, results, history)",
        Context::Prompt => "Prompts and overlays",
        Context::Completion => "Autocomplete (while suggestions show)",
        Context::Snippets => "Snippet library (while open)",
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

    #[test]
    fn text_area_starts_with_the_cursor_after_the_last_character() {
        let area = TextArea::new("GET /foo\nHost: x");

        assert_eq!(area.text(), "GET /foo\nHost: x");
        assert_eq!(area.cursor_row(), 1);
    }

    #[test]
    fn typing_inserts_at_the_cursor_and_enter_splits_the_line() {
        let mut area = TextArea::new("");
        for c in "hello".chars() {
            area.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        area.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        for c in "world".chars() {
            area.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert_eq!(area.text(), "hello\nworld");
    }

    #[test]
    fn text_input_insert_str_drops_embedded_newlines() {
        let mut input = TextInput::new("ab");
        input.cursor = 1;

        input.insert_str("X\nY\r");

        assert_eq!(input.text(), "aXYb");
    }

    #[test]
    fn text_area_insert_str_splits_into_real_lines_on_newline() {
        let mut area = TextArea::new("");

        area.insert_str("line one\nline two");

        assert_eq!(area.text(), "line one\nline two");
        assert_eq!(area.cursor_row(), 1);
    }

    #[test]
    fn text_area_insert_str_pastes_at_the_cursor_not_at_the_end() {
        let mut area = TextArea::new("ac");
        area.cursor_col = 1;

        area.insert_str("b");

        assert_eq!(area.text(), "abc");
    }

    #[test]
    fn backspace_at_the_start_of_a_line_merges_it_into_the_previous_one() {
        let mut area = TextArea::new("ab\ncd");
        area.cursor_row = 1;
        area.cursor_col = 0;

        area.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor_row, 0);
        assert_eq!(area.cursor_col, 2);
    }

    #[test]
    fn delete_at_the_end_of_a_line_merges_the_next_one_up() {
        let mut area = TextArea::new("ab\ncd");
        area.cursor_row = 0;
        area.cursor_col = 2;

        area.handle_key_event(KeyCode::Delete, KeyModifiers::NONE);

        assert_eq!(area.text(), "abcd");
    }

    #[test]
    fn up_and_down_clamp_the_column_to_the_shorter_line() {
        let mut area = TextArea::new("abcdef\nxy");
        area.cursor_row = 0;
        area.cursor_col = 6;

        area.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(area.cursor_row, 1);
        assert_eq!(area.cursor_col, 2, "clamped to the shorter line's length");

        area.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(area.cursor_row, 0);
        assert_eq!(
            area.cursor_col, 2,
            "column position is preserved, not reset"
        );
    }

    #[test]
    fn left_and_right_cross_line_boundaries() {
        let mut area = TextArea::new("ab\ncd");
        area.cursor_row = 1;
        area.cursor_col = 0;

        area.handle_key_event(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!((area.cursor_row, area.cursor_col), (0, 2));

        area.handle_key_event(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!((area.cursor_row, area.cursor_col), (1, 0));
    }

    #[test]
    fn set_text_replaces_content_and_resets_the_cursor() {
        let mut area = TextArea::new("old content");
        area.set_text("new\ntext");

        assert_eq!(area.text(), "new\ntext");
        assert_eq!(area.cursor_row(), 1);
    }

    #[test]
    fn scroll_offset_only_moves_once_the_cursor_passes_the_viewport() {
        let mut area = TextArea::new("");
        for _ in 0..10 {
            area.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        }
        // 11 lines total (rows 0..=10), cursor on row 10.

        assert_eq!(
            area.scroll_offset(20),
            0,
            "fits entirely in a tall viewport"
        );
        assert_eq!(
            area.scroll_offset(5),
            6,
            "scrolls just enough to keep the cursor row visible"
        );
    }

    #[test]
    fn split_pane_starts_vertical_and_even() {
        let split = SplitPane::default();
        let area = Rect::new(0, 0, 100, 100);

        let (primary, secondary) = split.split(area);

        assert_eq!(primary.height, secondary.height, "even 50/50 split");
        assert_eq!(primary.width, area.width, "stacked, not side by side");
        assert!(primary.y < secondary.y, "primary on top");
    }

    #[test]
    fn toggle_orientation_switches_stacked_to_side_by_side() {
        let mut split = SplitPane::default();
        split.toggle_orientation();
        let area = Rect::new(0, 0, 100, 100);

        let (primary, secondary) = split.split(area);

        assert_eq!(primary.width + secondary.width, area.width);
        assert_eq!(primary.height, area.height, "full height, side by side");
        assert!(primary.x < secondary.x, "primary on the left");
    }

    #[test]
    fn zooming_in_on_the_focused_pane_grows_it_and_shrinks_the_other() {
        let mut split = SplitPane::default();
        let area = Rect::new(0, 0, 100, 100);

        split.zoom_in(true);
        let (primary, secondary) = split.split(area);
        assert!(primary.height > secondary.height, "primary grew");

        split.zoom_in(false);
        let (primary, secondary) = split.split(area);
        assert_eq!(
            primary.height, secondary.height,
            "zooming the secondary pane in undid the earlier zoom"
        );
    }

    #[test]
    fn zoom_never_shrinks_a_pane_past_the_minimum_share() {
        let mut split = SplitPane::default();
        for _ in 0..20 {
            split.zoom_in(false);
        }
        let area = Rect::new(0, 0, 100, 100);

        let (primary, _) = split.split(area);

        assert!(primary.height >= 19, "primary never disappears entirely");
    }

    #[test]
    fn zoom_out_reverses_zoom_in_for_the_same_focused_pane() {
        let mut split = SplitPane::default();
        let area = Rect::new(0, 0, 100, 100);
        split.zoom_in(true);

        split.zoom_out(true);
        let (primary, secondary) = split.split(area);

        assert_eq!(primary.height, secondary.height);
    }

    fn sample_menu() -> ContextMenu {
        ContextMenu::new(
            (5, 5),
            vec![
                ("Edit cell".to_string(), Command::EditCell),
                ("Delete row".to_string(), Command::DeleteRow),
                ("Yank".to_string(), Command::Yank),
            ],
        )
    }

    #[test]
    fn context_menu_starts_with_the_first_item_selected() {
        let mut menu = sample_menu();

        let outcome = menu.handle_key_event(KeyCode::Enter);

        assert_eq!(outcome, ContextMenuOutcome::Confirmed(Command::EditCell));
    }

    #[test]
    fn down_then_enter_confirms_the_second_item() {
        let mut menu = sample_menu();

        menu.handle_key_event(KeyCode::Down);
        let outcome = menu.handle_key_event(KeyCode::Enter);

        assert_eq!(outcome, ContextMenuOutcome::Confirmed(Command::DeleteRow));
    }

    #[test]
    fn selection_does_not_move_past_the_last_item() {
        let mut menu = sample_menu();
        for _ in 0..10 {
            menu.handle_key_event(KeyCode::Down);
        }

        let outcome = menu.handle_key_event(KeyCode::Enter);

        assert_eq!(outcome, ContextMenuOutcome::Confirmed(Command::Yank));
    }

    #[test]
    fn esc_closes_the_menu() {
        let mut menu = sample_menu();

        let outcome = menu.handle_key_event(KeyCode::Esc);

        assert_eq!(outcome, ContextMenuOutcome::Closed);
    }

    #[test]
    fn an_unrecognized_key_leaves_the_menu_open() {
        let mut menu = sample_menu();

        let outcome = menu.handle_key_event(KeyCode::Char('z'));

        assert_eq!(outcome, ContextMenuOutcome::Open);
    }

    #[test]
    fn clicking_an_item_returns_its_command() {
        let menu = sample_menu();
        let bounds = Rect::new(0, 0, 80, 24);

        // Origin (5, 5) -> rect (5, 5, 14, 5) -> inner starts at (6, 6), one
        // row per item: row 6 is "Edit cell", row 7 is "Delete row".
        assert_eq!(menu.click(bounds, 6, 6), Some(Command::EditCell));
        assert_eq!(menu.click(bounds, 6, 7), Some(Command::DeleteRow));
    }

    #[test]
    fn clicking_outside_the_menu_returns_none() {
        let menu = sample_menu();
        let bounds = Rect::new(0, 0, 80, 24);

        let hit = menu.click(bounds, 79, 23);

        assert_eq!(hit, None);
    }

    #[test]
    fn the_menu_stays_on_screen_when_the_click_is_near_the_edge() {
        let menu = ContextMenu::new((78, 22), vec![("Edit cell".to_string(), Command::EditCell)]);
        let bounds = Rect::new(0, 0, 80, 24);

        let rect = menu.rect(bounds);

        assert!(rect.x + rect.width <= bounds.width);
        assert!(rect.y + rect.height <= bounds.height);
    }
}
