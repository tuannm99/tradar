//! A small vim-modal text editor, purpose-built for the query buffer.
//! Previously backed by `edtui`; rewritten from scratch (2026-08-12) so a
//! later pass can render tree-sitter-highlighted spans directly instead of
//! going through a dependency whose syntax highlighting is hard-wired to
//! `syntect` with no pluggable backend -- see "Đánh bóng UI tổng thể" in
//! docs/backlog.md.
//!
//! Deliberately minimal compared to real vim: `Normal`/`Insert` modes only
//! (no Visual mode, no registers/macros, no counts, no operator+motion
//! combos beyond `dd`) -- enough for editing a query, not a general-purpose
//! editor. Row movement (`j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u`) reuses
//! `tradar_core::vim_list`, the same module every list-rendering component
//! in the app shares.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list;

use crate::sql_highlight;

/// What counts as part of a word for completion: identifier characters,
/// plus `$` and `_` so Mongo's `$match` and `snake_case` names complete as
/// one unit.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
}

/// Which tree-sitter grammar (if any) to highlight the buffer with. Set
/// once by `QueryScreenComponent` based on the active connection's driver
/// id -- `QueryEditorComponent` itself has no notion of connectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    PlainText,
    Sql,
}

/// A buffer + cursor position, captured before an edit so `undo` has
/// something to restore. Cheap enough to clone freely: query buffers are
/// small, and a checkpoint is taken once per discrete edit (an entire
/// Insert session, not per keystroke), not per character.
#[derive(Clone)]
struct Snapshot {
    lines: Vec<Vec<char>>,
    cursor_row: usize,
    cursor_col: usize,
}

/// Caps how many edits back `u` can reach, so an extremely long editing
/// session can't grow the undo stack without bound -- see "Support large
/// result sets efficiently" in CLAUDE.md, same low-memory principle applied
/// to editor history rather than query results.
const MAX_UNDO_HISTORY: usize = 500;

pub struct QueryEditorComponent {
    pub mode: EditorMode,
    dialect: Dialect,
    lines: Vec<Vec<char>>,
    cursor_row: usize,
    cursor_col: usize,
    pending_g: bool,
    pending_d: bool,
    scroll: usize,
    visible_height: usize,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
}

impl Default for QueryEditorComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryEditorComponent {
    pub fn new() -> Self {
        Self {
            mode: EditorMode::Normal,
            dialect: Dialect::PlainText,
            lines: vec![Vec::new()],
            cursor_row: 0,
            cursor_col: 0,
            pending_g: false,
            pending_d: false,
            scroll: 0,
            visible_height: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn set_dialect(&mut self, dialect: Dialect) {
        self.dialect = dialect;
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Inserts `text` at the cursor and switches to Insert mode -- used to
    /// splice a schema name in from the navigator or replay a history
    /// entry. One undo step, same as any other single edit.
    pub fn insert_at_cursor(&mut self, text: &str) {
        self.checkpoint();
        for c in text.chars() {
            self.insert_char(c);
        }
        self.mode = EditorMode::Insert;
    }

    /// Replaces the whole buffer, e.g. after loading a file from disk or
    /// picking a query from history.
    pub fn set_text(&mut self, text: &str) {
        self.lines = if text.is_empty() {
            vec![Vec::new()]
        } else {
            text.lines().map(|line| line.chars().collect()).collect()
        };
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll = 0;
        self.mode = EditorMode::Normal;
        self.pending_g = false;
        self.pending_d = false;
        // A freshly loaded buffer (a file, a history entry, a restored
        // session) has no relationship to whatever was undoable before --
        // undoing into it would silently resurrect the previous query.
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Scrolls the viewport by a wheel notch, moving the cursor with it so
    /// the cursor stays on screen (this editor has no concept of a cursor
    /// that's scrolled out of view).
    pub fn scroll(&mut self, mv: tradar_core::vim_list::VimMove) {
        let mut row = self.cursor_row;
        vim_list::apply(mv, &mut row, self.lines.len(), self.visible_height);
        self.move_row_to(row);
    }

    /// The word being typed, immediately before the cursor -- what a
    /// completion would replace. Word characters only, so `users.` or
    /// `(id` starts a fresh word rather than dragging punctuation in.
    pub fn word_before_cursor(&self) -> String {
        let line = &self.lines[self.cursor_row];
        let end = self.cursor_col.min(line.len());
        let start = line[..end]
            .iter()
            .rposition(|c| !is_word_char(*c))
            .map_or(0, |i| i + 1);
        line[start..end].iter().collect()
    }

    /// Swaps the word before the cursor for `text` and leaves the cursor
    /// after it -- accepting a completion.
    pub fn replace_word_before_cursor(&mut self, text: &str) {
        self.checkpoint();
        let line = &self.lines[self.cursor_row];
        let end = self.cursor_col.min(line.len());
        let start = line[..end]
            .iter()
            .rposition(|c| !is_word_char(*c))
            .map_or(0, |i| i + 1);
        let replacement: Vec<char> = text.chars().collect();
        let inserted = replacement.len();
        self.lines[self.cursor_row].splice(start..end, replacement);
        self.cursor_col = start + inserted;
        self.clamp_col();
    }

    /// The cursor's position as a byte offset into `text()` -- what
    /// "which statement am I in?" needs, since statements are reported as
    /// offsets into the whole buffer.
    pub fn cursor_offset(&self) -> usize {
        let mut offset = 0;
        for row in &self.lines[..self.cursor_row] {
            offset += row.iter().map(|c| c.len_utf8()).sum::<usize>() + 1; // + newline
        }
        offset
            + self.lines[self.cursor_row][..self.cursor_col.min(self.lines[self.cursor_row].len())]
                .iter()
                .map(|c| c.len_utf8())
                .sum::<usize>()
    }

    /// Where the cursor sits on screen, given the area the editor was last
    /// drawn into -- so a popup can be placed against it.
    pub fn cursor_screen_position(&self, area: Rect) -> (u16, u16) {
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(self.cursor_col as u16);
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(self.cursor_row.saturating_sub(self.scroll) as u16);
        (x, y)
    }

    pub fn forward_key(&mut self, key: KeyEvent) {
        match self.mode {
            EditorMode::Normal => self.handle_normal_key(key.code, key.modifiers),
            EditorMode::Insert => self.handle_insert_key(key.code, key.modifiers),
        }
    }

    fn current_line_len(&self) -> usize {
        self.lines[self.cursor_row].len()
    }

    /// Clamps the column into `[0, len)` in Normal mode -- the cursor rests
    /// on the last character, never past it (except an empty line, where
    /// it sits at 0) -- or `[0, len]` in Insert mode, where it may sit one
    /// past the last character to append there.
    fn clamp_col(&mut self) {
        let len = self.current_line_len();
        let max = match self.mode {
            EditorMode::Normal => len.saturating_sub(1),
            EditorMode::Insert => len,
        };
        self.cursor_col = self.cursor_col.min(max);
    }

    /// Records the buffer as it stands right now, so `undo` can get back to
    /// it. Called once per discrete edit -- on entering Insert mode rather
    /// than per keystroke, so an entire typing session collapses into a
    /// single undo step, the same as real vim's `i...Esc`.
    fn checkpoint(&mut self) {
        self.undo_stack.push(Snapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        });
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        // A new edit starting from here makes whatever was undone no longer
        // reachable by redo -- the same branch-is-discarded rule every
        // undo/redo stack follows.
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        let Some(previous) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(Snapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        });
        self.restore(previous);
    }

    fn redo(&mut self) {
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(Snapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        });
        self.restore(next);
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.lines = snapshot.lines;
        self.cursor_row = snapshot.cursor_row.min(self.lines.len().saturating_sub(1));
        self.cursor_col = snapshot.cursor_col;
        self.clamp_col();
    }

    fn move_row_to(&mut self, row: usize) {
        self.cursor_row = row.min(self.lines.len().saturating_sub(1));
        self.clamp_col();
    }

    fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_row].insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    fn delete_current_line(&mut self) {
        if self.lines.len() > 1 {
            self.lines.remove(self.cursor_row);
            if self.cursor_row >= self.lines.len() {
                self.cursor_row = self.lines.len() - 1;
            }
        } else {
            self.lines[0].clear();
            self.cursor_col = 0;
        }
        self.clamp_col();
    }

    fn handle_normal_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // `dd` deletes the current line -- the one operator+motion combo
        // supported. Any key other than a second `d` cancels it (already
        // consumed via `take` above, same "any other key cancels" rule as
        // a pending `g`).
        if std::mem::take(&mut self.pending_d) {
            if code == KeyCode::Char('d') {
                self.checkpoint();
                self.delete_current_line();
            }
            return;
        }

        if let Some(mv) = vim_list::recognize(code, modifiers, &mut self.pending_g) {
            let mut row = self.cursor_row;
            vim_list::apply(mv, &mut row, self.lines.len(), self.visible_height);
            self.move_row_to(row);
            return;
        }

        match code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.cursor_col = self.cursor_col.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let max = self.current_line_len().saturating_sub(1);
                self.cursor_col = (self.cursor_col + 1).min(max);
            }
            KeyCode::Char('0') => self.cursor_col = 0,
            KeyCode::Char('$') => {
                self.cursor_col = self.current_line_len().saturating_sub(1);
            }
            KeyCode::Char('i') => {
                self.checkpoint();
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('a') => {
                self.checkpoint();
                if self.current_line_len() > 0 {
                    self.cursor_col += 1;
                }
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('I') => {
                self.checkpoint();
                self.cursor_col = 0;
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('A') => {
                self.checkpoint();
                self.cursor_col = self.current_line_len();
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('o') => {
                self.checkpoint();
                self.lines.insert(self.cursor_row + 1, Vec::new());
                self.cursor_row += 1;
                self.cursor_col = 0;
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('O') => {
                self.checkpoint();
                self.lines.insert(self.cursor_row, Vec::new());
                self.cursor_col = 0;
                self.mode = EditorMode::Insert;
            }
            KeyCode::Char('x') => {
                if self.cursor_col < self.current_line_len() {
                    self.checkpoint();
                    self.lines[self.cursor_row].remove(self.cursor_col);
                    self.clamp_col();
                }
            }
            KeyCode::Char('d') => self.pending_d = true,
            // Real vim's redo key, `ctrl-r`, is already query-screen's
            // "open history" and is intercepted before it ever reaches
            // this editor (see `Context::QueryScreen` in
            // `tradar_core::keymap`) -- `U` is the substitute. `u` itself
            // is free: `vim_list::recognize` only claims `ctrl-u`
            // (half-page-up), not the bare key.
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('U') => self.redo(),
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.mode = EditorMode::Normal;
                self.clamp_col();
            }
            KeyCode::Enter => {
                let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
                self.lines.insert(self.cursor_row + 1, rest);
                self.cursor_row += 1;
                self.cursor_col = 0;
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
            }
            KeyCode::Left => self.cursor_col = self.cursor_col.saturating_sub(1),
            KeyCode::Right => {
                let max = self.current_line_len();
                self.cursor_col = (self.cursor_col + 1).min(max);
            }
            KeyCode::Up => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.clamp_col();
                }
            }
            KeyCode::Down => {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.clamp_col();
                }
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_char(c);
            }
            _ => {}
        }
    }

    fn scroll_into_view(&mut self) {
        if self.visible_height == 0 {
            return;
        }
        if self.cursor_row < self.scroll {
            self.scroll = self.cursor_row;
        } else if self.cursor_row >= self.scroll + self.visible_height {
            self.scroll = self.cursor_row + 1 - self.visible_height;
        }
    }

    /// One `Color` per line, indexed the same as `self.lines` -- `None` in
    /// `Dialect::PlainText`, or if highlighting the current buffer failed
    /// (falls back to unstyled text either way).
    fn line_colors(&self) -> Option<Vec<Vec<Color>>> {
        if self.dialect != Dialect::Sql {
            return None;
        }
        let text = self.text();
        let mut colors = sql_highlight::char_colors(&text)?.into_iter();
        let mut result = Vec::with_capacity(self.lines.len());
        for (i, line) in self.lines.iter().enumerate() {
            result.push(colors.by_ref().take(line.len()).collect());
            if i + 1 < self.lines.len() {
                colors.next(); // the '\n' joining this line to the next
            }
        }
        Some(result)
    }

    fn render_line(&self, line: &[char], row: usize, colors: Option<&[Color]>) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = line
            .iter()
            .enumerate()
            .map(|(col, &c)| {
                let mut style = Style::default();
                if let Some(color) = colors.and_then(|c| c.get(col)) {
                    style = style.fg(*color);
                }
                if row == self.cursor_row && col == self.cursor_col {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                Span::styled(c.to_string(), style)
            })
            .collect();
        if row == self.cursor_row && self.cursor_col >= line.len() {
            // Insert-mode cursor sitting just past the last character (or
            // on an empty line) -- still needs a visible cell.
            spans.push(Span::styled(
                " ",
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }
        Line::from(spans)
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str, focused: bool) {
        let block = ui::panel(&format!("Query — {connection_name}"), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Reserve the last inner row for a mode indicator.
        let text_height = inner.height.saturating_sub(1);
        self.visible_height = text_height as usize;
        self.scroll_into_view();

        let line_colors = self.line_colors();
        let lines: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(text_height as usize)
            .enumerate()
            .map(|(i, line)| {
                let row = self.scroll + i;
                let colors = line_colors.as_ref().map(|lc| lc[row].as_slice());
                self.render_line(line, row, colors)
            })
            .collect();
        let text_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: text_height,
        };
        frame.render_widget(Paragraph::new(Text::from(lines)), text_area);

        if inner.height > text_height {
            let theme = theme();
            let (mode_label, mode_color) = match self.mode {
                EditorMode::Normal => (" NORMAL ", theme.accent),
                EditorMode::Insert => (" INSERT ", theme.warning),
            };
            let mode_area = Rect {
                x: inner.x,
                y: inner.y + text_height,
                width: inner.width,
                height: 1,
            };
            let position = format!(" {}:{} ", self.cursor_row + 1, self.cursor_col + 1);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        mode_label,
                        Style::default()
                            .bg(mode_color)
                            .fg(theme.status_bar_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(position, Style::default().fg(theme.text_dim)),
                ])),
                mode_area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(editor: &mut QueryEditorComponent, s: &str) {
        for c in s.chars() {
            editor.forward_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_in_insert_mode_updates_the_text() {
        let mut editor = QueryEditorComponent::new();

        editor.forward_key(key(KeyCode::Char('i')));
        editor.forward_key(key(KeyCode::Char('a')));
        editor.forward_key(key(KeyCode::Char('b')));

        assert_eq!(editor.text(), "ab");
    }

    #[test]
    fn backspace_in_insert_mode_removes_the_last_character() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Backspace));

        assert_eq!(editor.text(), "");
    }

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_with_the_previous_one() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        type_str(&mut editor, "ab");
        editor.forward_key(key(KeyCode::Enter));
        type_str(&mut editor, "cd");

        editor.forward_key(key(KeyCode::Backspace));
        editor.forward_key(key(KeyCode::Backspace));
        editor.forward_key(key(KeyCode::Backspace));

        assert_eq!(editor.text(), "ab");
    }

    #[test]
    fn enter_in_insert_mode_splits_the_line_at_the_cursor() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        type_str(&mut editor, "abcd");
        editor.forward_key(key(KeyCode::Left));
        editor.forward_key(key(KeyCode::Left));

        editor.forward_key(key(KeyCode::Enter));

        assert_eq!(editor.text(), "ab\ncd");
    }

    #[test]
    fn insert_at_cursor_inserts_text_and_switches_to_insert_mode() {
        let mut editor = QueryEditorComponent::new();

        editor.insert_at_cursor("users");

        assert_eq!(editor.text(), "users");
        assert_eq!(editor.mode, EditorMode::Insert);
    }

    #[test]
    fn set_text_replaces_the_whole_buffer() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("old");

        editor.set_text("select 1\nfrom users");

        assert_eq!(editor.text(), "select 1\nfrom users");
        assert_eq!(editor.mode, EditorMode::Normal);
    }

    #[test]
    fn esc_leaves_the_cursor_on_the_last_typed_character() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        type_str(&mut editor, "ab");

        editor.forward_key(key(KeyCode::Esc));
        editor.insert_at_cursor("X");

        // Cursor sat on 'b' (index 1) after Esc, so "X" lands before it.
        assert_eq!(editor.text(), "aXb");
    }

    #[test]
    fn hjkl_moves_the_cursor_in_normal_mode() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("ab\ncd");

        editor.forward_key(key(KeyCode::Char('l')));
        editor.insert_at_cursor("X");
        assert_eq!(editor.text(), "aXb\ncd");

        editor.set_text("ab\ncd");
        editor.forward_key(key(KeyCode::Char('j')));
        editor.insert_at_cursor("X");
        assert_eq!(
            editor.text(),
            "ab\nXcd",
            "column is preserved across the row move (still 0), landing before 'c'"
        );
    }

    #[test]
    fn gg_and_shift_g_jump_between_the_first_and_last_line() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb\nc");

        editor.forward_key(key(KeyCode::Char('G')));
        editor.insert_at_cursor("X");
        assert_eq!(
            editor.text(),
            "a\nb\nXc",
            "column is preserved across the row move (still 0)"
        );

        editor.set_text("a\nb\nc");
        editor.forward_key(key(KeyCode::Char('g')));
        editor.forward_key(key(KeyCode::Char('g')));
        editor.insert_at_cursor("X");
        assert_eq!(editor.text(), "Xa\nb\nc");
    }

    #[test]
    fn x_deletes_the_character_under_the_cursor() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('x')));

        assert_eq!(editor.text(), "bc");
    }

    #[test]
    fn dd_deletes_the_current_line() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb\nc");
        editor.forward_key(key(KeyCode::Char('j')));

        editor.forward_key(key(KeyCode::Char('d')));
        editor.forward_key(key(KeyCode::Char('d')));

        assert_eq!(editor.text(), "a\nc");
    }

    #[test]
    fn dd_on_the_only_line_clears_it_instead_of_removing_it() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('d')));
        editor.forward_key(key(KeyCode::Char('d')));

        assert_eq!(editor.text(), "");
    }

    #[test]
    fn a_non_d_key_cancels_a_pending_d() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb\nc");

        editor.forward_key(key(KeyCode::Char('d')));
        editor.forward_key(key(KeyCode::Char('j')));

        assert_eq!(editor.text(), "a\nb\nc", "the line must not be deleted");
    }

    #[test]
    fn o_and_shift_o_open_a_line_below_and_above() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("b");

        editor.forward_key(key(KeyCode::Char('o')));
        type_str(&mut editor, "c");
        editor.forward_key(key(KeyCode::Esc));
        editor.forward_key(key(KeyCode::Char('g')));
        editor.forward_key(key(KeyCode::Char('g')));
        editor.forward_key(key(KeyCode::Char('O')));
        type_str(&mut editor, "a");

        assert_eq!(editor.text(), "a\nb\nc");
    }

    #[test]
    fn u_undoes_a_whole_insert_session_as_one_step() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        type_str(&mut editor, "abc");
        editor.forward_key(key(KeyCode::Esc));

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(
            editor.text(),
            "",
            "the entire typing session undoes at once, not one character at a time"
        );
    }

    #[test]
    fn shift_u_redoes_what_was_just_undone() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("abc");
        editor.forward_key(key(KeyCode::Esc));
        editor.forward_key(key(KeyCode::Char('u')));
        assert_eq!(editor.text(), "");

        editor.forward_key(key(KeyCode::Char('U')));

        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn u_undoes_x() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('x')));
        assert_eq!(editor.text(), "bc");

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn u_undoes_dd() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb\nc");
        editor.forward_key(key(KeyCode::Char('j')));

        editor.forward_key(key(KeyCode::Char('d')));
        editor.forward_key(key(KeyCode::Char('d')));
        assert_eq!(editor.text(), "a\nc");

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(editor.text(), "a\nb\nc");
    }

    #[test]
    fn undo_with_nothing_to_undo_is_a_no_op() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn redo_with_nothing_to_redo_is_a_no_op() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('U')));

        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn multiple_undos_walk_back_through_several_edits() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("a");
        editor.forward_key(key(KeyCode::Esc));
        editor.forward_key(key(KeyCode::Char('A')));
        type_str(&mut editor, "b");
        editor.forward_key(key(KeyCode::Esc));
        editor.forward_key(key(KeyCode::Char('A')));
        type_str(&mut editor, "c");
        editor.forward_key(key(KeyCode::Esc));
        assert_eq!(editor.text(), "abc");

        editor.forward_key(key(KeyCode::Char('u')));
        assert_eq!(editor.text(), "ab");
        editor.forward_key(key(KeyCode::Char('u')));
        assert_eq!(editor.text(), "a");
        editor.forward_key(key(KeyCode::Char('u')));
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn a_new_edit_after_undoing_discards_the_redo_branch() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("abc");
        editor.forward_key(key(KeyCode::Esc));
        editor.forward_key(key(KeyCode::Char('u')));
        assert_eq!(editor.text(), "");

        editor.insert_at_cursor("xyz");
        editor.forward_key(key(KeyCode::Esc));

        editor.forward_key(key(KeyCode::Char('U')));
        assert_eq!(
            editor.text(),
            "xyz",
            "redo must not resurrect the abandoned 'abc' branch"
        );
    }

    #[test]
    fn set_text_clears_undo_history_so_a_loaded_file_cannot_be_undone_away() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("old query");
        editor.forward_key(key(KeyCode::Esc));

        editor.set_text("select 1");
        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(
            editor.text(),
            "select 1",
            "undo must not reach back into the buffer that was replaced"
        );
    }

    #[test]
    fn undo_restores_the_cursor_position_along_with_the_text() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("ab\ncd");
        editor.forward_key(key(KeyCode::Char('j'))); // row 1
        editor.forward_key(key(KeyCode::Char('l'))); // col 1
        editor.forward_key(key(KeyCode::Char('x'))); // deletes 'd' on row 1

        editor.forward_key(key(KeyCode::Char('u')));
        editor.insert_at_cursor("X");

        // Cursor must be back on row 1 col 1 (before 'd'), not wherever `x`
        // left it after clamping onto a shorter line.
        assert_eq!(editor.text(), "ab\ncXd");
    }

    #[test]
    fn draw_shows_the_connection_name_and_input() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("x");
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "local-sqlite", true))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_the_current_mode() {
        let mut editor = QueryEditorComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("NORMAL"), "buffer was: {text}");

        editor.forward_key(key(KeyCode::Char('i')));
        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("INSERT"), "buffer was: {text}");
    }

    #[test]
    fn scrolls_so_the_cursor_row_stays_visible() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text(
            &(0..20)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        editor.forward_key(key(KeyCode::Char('G')));

        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 20, 8), "x", true))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains('9'),
            "last line (19) should be visible: {text}"
        );
    }
}
