//! A small vim-modal text editor, purpose-built for the query buffer.
//! Previously backed by `edtui`; rewritten from scratch (2026-08-12) so a
//! later pass can render tree-sitter-highlighted spans directly instead of
//! going through a dependency whose syntax highlighting is hard-wired to
//! `syntect` with no pluggable backend -- see "Đánh bóng UI tổng thể" in
//! docs/backlog.md.
//!
//! Deliberately minimal compared to real vim: no macros, no counts, no
//! operator+motion combos beyond `dd`/`yy` -- enough for editing a query,
//! not a general-purpose editor. `Normal`/`Insert`/`Visual`/`VisualLine`
//! modes; one unnamed yank/delete register (no `"a`-style named ones). Row
//! movement (`j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u`) reuses
//! `tradar_core::vim_list`, the same module every list-rendering component
//! in the app shares.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list;

use crate::query_driver;
use crate::sql_highlight;

/// Width, in columns, of the fold-marker gutter (`▸ `/`▾ `) prepended to
/// each line when the buffer has foldable statements (`Dialect::Sql` only
/// -- Mongo/Redis/ES statements are one line each, nothing to fold).
const FOLD_GUTTER_WIDTH: u16 = 2;

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
    /// Charwise selection (`v`) -- `visual_anchor` holds where it started.
    Visual,
    /// Linewise selection (`V`) -- same anchor, but the selection always
    /// covers whole lines regardless of either endpoint's column.
    VisualLine,
}

/// What `y`/`d`/`x`/`c` last cut or copied -- a single unnamed register,
/// the same one every one of those commands shares (real vim's default
/// `"` register), not per-command or named (`"a`, `"b`, ...) ones.
/// `Charwise` may itself contain `\n`: a multi-line charwise selection
/// (`v` spanning lines, not `V`) yanks as one blob with embedded line
/// breaks, same as real vim, so pasting it back reconstructs the lines
/// instead of splicing a literal `\n` character into one line.
#[derive(Debug, Clone, PartialEq)]
enum Register {
    Charwise(Vec<char>),
    Linewise(Vec<Vec<char>>),
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
    pending_y: bool,
    pending_z: bool,
    /// Where `v`/`V` was pressed -- `Some` only while `mode` is `Visual`/
    /// `VisualLine`, cleared the moment either exits (`Esc`, or `y`/`d`/
    /// `x`/`c` finishing the selection).
    visual_anchor: Option<(usize, usize)>,
    register: Option<Register>,
    scroll: usize,
    visible_height: usize,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// Start lines (in `self.lines`) of statements currently folded --
    /// see `foldable_ranges`. Keyed by line index rather than by statement
    /// identity, so an edit that shifts lines above a fold can make the
    /// entry silently stop matching a real statement start instead of
    /// folding the wrong thing; re-folding after such an edit is a manual
    /// `za` away. `checkpoint()` removes the entry for whatever statement
    /// the cursor is in before any edit, so typing into a folded statement
    /// always unfolds it first rather than hiding what was just typed.
    folded: HashSet<usize>,
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
            pending_y: false,
            pending_z: false,
            visual_anchor: None,
            register: None,
            scroll: 0,
            visible_height: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            folded: HashSet::new(),
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
        self.insert_multiline(text);
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
        self.pending_y = false;
        self.pending_z = false;
        self.visual_anchor = None;
        // A freshly loaded buffer (a file, a history entry, a restored
        // session) has no relationship to whatever was undoable before --
        // undoing into it would silently resurrect the previous query.
        self.undo_stack.clear();
        self.redo_stack.clear();
        // Same reasoning: line indices in `folded` mean nothing against a
        // buffer they weren't computed from.
        self.folded.clear();
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

    /// Line ranges (`start..=end`, both inclusive) of top-level SQL
    /// statements that span more than one line -- the only fold
    /// candidates, since a one-line statement has nothing to hide. Empty
    /// outside `Dialect::Sql`: folding needs a notion of "one statement",
    /// which only SQL's `;`-delimited grammar gives this editor (Mongo's
    /// `db.<coll>.<method>(...)`, Redis's one-command-per-line, and
    /// Elasticsearch's `METHOD /path` + body are already one statement per
    /// line each, so there'd be nothing to fold even if this ran for them).
    /// Recomputed from the live buffer on every call, same tradeoff
    /// `line_colors` already makes for tree-sitter highlighting -- cheap
    /// for a query-sized buffer, and simpler than keeping a parallel
    /// structure in sync with every edit.
    fn foldable_ranges(&self) -> Vec<(usize, usize)> {
        if self.dialect != Dialect::Sql {
            return Vec::new();
        }
        let text = self.text();
        query_driver::split_sql_statements(&text)
            .into_iter()
            .filter_map(|stmt| {
                let start_line = Self::line_at_byte_offset(&text, stmt.start);
                let last_byte = stmt.end.saturating_sub(1).max(stmt.start);
                let end_line = Self::line_at_byte_offset(&text, last_byte);
                (end_line > start_line).then_some((start_line, end_line))
            })
            .collect()
    }

    fn line_at_byte_offset(text: &str, offset: usize) -> usize {
        text.as_bytes()[..offset.min(text.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    /// The folded range `row` falls inside, if `row` is one of its hidden
    /// body lines (the header line itself doesn't count -- it's always
    /// visible, so landing there is never a problem).
    fn fold_containing(&self, row: usize) -> Option<(usize, usize)> {
        self.foldable_ranges()
            .into_iter()
            .find(|&(start, end)| self.folded.contains(&start) && row > start && row <= end)
    }

    /// Every line index currently hidden by a fold -- a folded range's
    /// header line stays visible (it's what shows the "N lines folded"
    /// placeholder), only the lines after it disappear.
    fn hidden_rows(&self) -> HashSet<usize> {
        let mut hidden = HashSet::new();
        for (start, end) in self.foldable_ranges() {
            if self.folded.contains(&start) {
                hidden.extend((start + 1)..=end);
            }
        }
        hidden
    }

    /// `za`: toggle the fold for the statement the cursor is currently in.
    /// A no-op if the cursor isn't inside a foldable (multi-line) statement.
    fn toggle_fold(&mut self) {
        let Some((start, _)) = self
            .foldable_ranges()
            .into_iter()
            .find(|&(start, end)| (start..=end).contains(&self.cursor_row))
        else {
            return;
        };
        if self.folded.remove(&start) {
            return;
        }
        self.folded.insert(start);
        self.cursor_row = start;
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

    /// The cursor's current `(row, col)` -- what incremental search needs
    /// to remember as "where to jump back to" if the search is cancelled.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    /// Moves the cursor straight to `(row, col)`, clamped into bounds --
    /// the other half of `cursor()`, for restoring a remembered position.
    /// Bypasses fold-awareness (`move_row_to`) deliberately: this is meant
    /// for jumping back to a position the cursor was already actually on,
    /// not for arbitrary navigation.
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col;
        self.clamp_col();
    }

    /// Moves the cursor to the next (or, if `backwards`, previous)
    /// occurrence of `pattern` -- case-insensitive (ASCII only: this is a
    /// query buffer, not general text, so full Unicode case-folding isn't
    /// worth the index-mapping complexity it'd add), starting just past
    /// (or before) the cursor and wrapping around the whole buffer the
    /// same way real vim's default `wrapscan` does. Returns whether
    /// anything was found; the cursor is left exactly where it was for an
    /// empty `pattern` or no match anywhere.
    pub fn find(&mut self, pattern: &str, backwards: bool) -> bool {
        let needle: Vec<char> = pattern.chars().collect();
        if needle.is_empty() || self.lines.is_empty() {
            return false;
        }
        let n = self.lines.len();
        for step in 0..=n {
            let row = if backwards {
                (self.cursor_row + n - (step % n)) % n
            } else {
                (self.cursor_row + step) % n
            };
            let line = &self.lines[row];
            let found = if backwards {
                let before = if step == 0 {
                    self.cursor_col
                } else {
                    line.len()
                };
                Self::rfind_in_line(line, &needle, before)
            } else {
                let after = if step == 0 { self.cursor_col + 1 } else { 0 };
                Self::find_in_line(line, &needle, after)
            };
            if let Some(col) = found {
                self.cursor_row = row;
                self.cursor_col = col;
                self.clamp_col();
                return true;
            }
        }
        false
    }

    /// The first index at or after `from_col` where `needle` matches,
    /// case-insensitively (ASCII).
    fn find_in_line(haystack: &[char], needle: &[char], from_col: usize) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (from_col.min(haystack.len())..=haystack.len() - needle.len())
            .find(|&i| Self::matches_at(haystack, needle, i))
    }

    /// The last index strictly before `before_col` where `needle` matches
    /// -- the backward counterpart of `find_in_line`.
    fn rfind_in_line(haystack: &[char], needle: &[char], before_col: usize) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        (0..=haystack.len() - needle.len())
            .filter(|&i| i < before_col)
            .filter(|&i| Self::matches_at(haystack, needle, i))
            .max()
    }

    fn matches_at(haystack: &[char], needle: &[char], at: usize) -> bool {
        haystack[at..at + needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Where the cursor sits on screen, given the area the editor was last
    /// drawn into -- so a popup can be placed against it.
    pub fn cursor_screen_position(&self, area: Rect) -> (u16, u16) {
        let hidden = self.hidden_rows();
        let visible: Vec<usize> = (0..self.lines.len())
            .filter(|r| !hidden.contains(r))
            .collect();
        let cursor_idx = visible
            .iter()
            .position(|&r| r == self.cursor_row)
            .unwrap_or(0);
        let scroll_idx = visible.iter().position(|&r| r >= self.scroll).unwrap_or(0);
        let gutter = if self.dialect == Dialect::Sql {
            FOLD_GUTTER_WIDTH
        } else {
            0
        };
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(gutter)
            .saturating_add(self.cursor_col as u16);
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(cursor_idx.saturating_sub(scroll_idx) as u16);
        (x, y)
    }

    pub fn forward_key(&mut self, key: KeyEvent) {
        match self.mode {
            EditorMode::Normal => self.handle_normal_key(key.code, key.modifiers),
            EditorMode::Insert => self.handle_insert_key(key.code, key.modifiers),
            EditorMode::Visual | EditorMode::VisualLine => {
                self.handle_visual_key(key.code, key.modifiers)
            }
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
            EditorMode::Normal | EditorMode::Visual | EditorMode::VisualLine => {
                len.saturating_sub(1)
            }
            EditorMode::Insert => len,
        };
        self.cursor_col = self.cursor_col.min(max);
    }

    /// Records the buffer as it stands right now, so `undo` can get back to
    /// it. Called once per discrete edit -- on entering Insert mode rather
    /// than per keystroke, so an entire typing session collapses into a
    /// single undo step, the same as real vim's `i...Esc`.
    fn checkpoint(&mut self) {
        // Editing a folded statement must not silently bury the new text
        // under its still-collapsed body -- open it first so whatever gets
        // typed next is visible.
        if let Some((start, _)) = self
            .foldable_ranges()
            .into_iter()
            .find(|&(start, end)| (start..=end).contains(&self.cursor_row))
        {
            self.folded.remove(&start);
        }
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

    /// Moves the cursor to `row`, but a folded statement's hidden body
    /// lines aren't valid landing spots -- moving down into one jumps past
    /// the whole fold to the line after it, moving up into one lands back
    /// on the fold's header, so a fold behaves like a single line for
    /// `j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u` (all of which funnel through
    /// here) without the cursor ever sitting somewhere invisible.
    fn move_row_to(&mut self, row: usize) {
        let mut target = row.min(self.lines.len().saturating_sub(1));
        let going_down = target >= self.cursor_row;
        if let Some((start, end)) = self.fold_containing(target) {
            target = if going_down {
                (end + 1).min(self.lines.len().saturating_sub(1))
            } else {
                start
            };
        }
        self.cursor_row = target;
        self.clamp_col();
    }

    fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_row].insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    /// Deletes the current line into `register` (linewise) -- `dd`.
    fn delete_current_line(&mut self) {
        let removed = if self.lines.len() > 1 {
            let removed = self.lines.remove(self.cursor_row);
            if self.cursor_row >= self.lines.len() {
                self.cursor_row = self.lines.len() - 1;
            }
            removed
        } else {
            self.cursor_col = 0;
            std::mem::take(&mut self.lines[0])
        };
        self.register = Some(Register::Linewise(vec![removed]));
        self.clamp_col();
    }

    /// Copies the current line into `register` without deleting it -- `yy`.
    fn yank_current_line(&mut self) {
        self.register = Some(Register::Linewise(vec![
            self.lines[self.cursor_row].clone(),
        ]));
    }

    /// Inserts `text` at the cursor, splitting on any embedded `\n` into
    /// real new lines rather than a literal newline character -- what a
    /// charwise register that spans lines needs on paste. A single-line
    /// `text` behaves exactly like repeated `insert_char`.
    fn insert_multiline(&mut self, text: &str) {
        let mut pieces = text.split('\n');
        for c in pieces.next().unwrap_or("").chars() {
            self.insert_char(c);
        }
        for piece in pieces {
            let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
            self.lines.insert(self.cursor_row + 1, rest);
            self.cursor_row += 1;
            self.cursor_col = 0;
            for c in piece.chars() {
                self.insert_char(c);
            }
        }
    }

    /// `p`/`P`: pastes `register` after (`p`) or before (`P`) the cursor --
    /// linewise as a new line, charwise inline (splitting embedded `\n`
    /// via `insert_multiline`). A no-op with nothing to checkpoint if
    /// nothing's ever been yanked or deleted.
    fn paste(&mut self, before: bool) {
        let Some(register) = self.register.clone() else {
            return;
        };
        self.checkpoint();
        match register {
            Register::Linewise(lines) => {
                let at = if before {
                    self.cursor_row
                } else {
                    self.cursor_row + 1
                };
                for (i, line) in lines.into_iter().enumerate() {
                    self.lines.insert(at + i, line);
                }
                self.cursor_row = at;
                self.cursor_col = 0;
            }
            Register::Charwise(chars) => {
                self.cursor_col = if before {
                    self.cursor_col
                } else {
                    (self.cursor_col + 1).min(self.current_line_len())
                };
                let text: String = chars.into_iter().collect();
                self.insert_multiline(&text);
            }
        }
        self.clamp_col();
    }

    /// Movement-only keys, shared by Normal and Visual/VisualLine mode:
    /// `h`/`l`/`0`/`$` here, plus `j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u` via
    /// `vim_list`. Returns whether `code` was one of them, so a caller can
    /// fall through to its own mode-specific keys otherwise.
    fn handle_motion_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if let Some(mv) = vim_list::recognize(code, modifiers, &mut self.pending_g) {
            let mut row = self.cursor_row;
            vim_list::apply(mv, &mut row, self.lines.len(), self.visible_height);
            self.move_row_to(row);
            return true;
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
            _ => return false,
        }
        true
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

        // `yy` copies the current line -- no checkpoint, nothing mutates.
        if std::mem::take(&mut self.pending_y) {
            if code == KeyCode::Char('y') {
                self.yank_current_line();
            }
            return;
        }

        // `za` toggles the fold under the cursor -- real vim's binding for
        // it, kept even though this editor has no other `z`-prefixed folds
        // command (`zo`/`zc`/`zR`/`zM`...) so a vim user's first guess works.
        if std::mem::take(&mut self.pending_z) {
            if code == KeyCode::Char('a') {
                self.toggle_fold();
            }
            return;
        }

        if self.handle_motion_key(code, modifiers) {
            return;
        }

        match code {
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
            KeyCode::Char('v') => {
                self.visual_anchor = Some((self.cursor_row, self.cursor_col));
                self.mode = EditorMode::Visual;
            }
            KeyCode::Char('V') => {
                self.visual_anchor = Some((self.cursor_row, self.cursor_col));
                self.mode = EditorMode::VisualLine;
            }
            KeyCode::Char('x') => {
                if self.cursor_col < self.current_line_len() {
                    self.checkpoint();
                    let removed = self.lines[self.cursor_row].remove(self.cursor_col);
                    self.register = Some(Register::Charwise(vec![removed]));
                    self.clamp_col();
                }
            }
            KeyCode::Char('p') => self.paste(false),
            KeyCode::Char('P') => self.paste(true),
            KeyCode::Char('d') => self.pending_d = true,
            KeyCode::Char('y') => self.pending_y = true,
            KeyCode::Char('z') => self.pending_z = true,
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

    /// The current Visual/VisualLine selection's endpoints as `(start,
    /// end)`, ordered so `start <= end` regardless of which direction the
    /// cursor moved from the anchor. Falls back to the cursor alone (an
    /// empty/degenerate selection) if called with no anchor set -- callers
    /// only reach this from Visual/VisualLine, where `visual_anchor` is
    /// always `Some`, but the type doesn't guarantee it.
    fn selection_bounds(&self) -> ((usize, usize), (usize, usize)) {
        let anchor = self
            .visual_anchor
            .unwrap_or((self.cursor_row, self.cursor_col));
        let cursor = (self.cursor_row, self.cursor_col);
        if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        }
    }

    /// The exact characters a charwise selection covers, `sr`/`sc` through
    /// `er`/`ec` **inclusive of both ends** (real vim's charwise selection
    /// includes the character the cursor is on, unlike most editors'
    /// exclusive ranges). A selection spanning more than one line joins
    /// them with `\n` -- see `Register::Charwise`'s own doc comment.
    fn charwise_slice(&self, sr: usize, sc: usize, er: usize, ec: usize) -> Vec<char> {
        if sr == er {
            let line = &self.lines[sr];
            let end = (ec + 1).min(line.len());
            return line[sc.min(end)..end].to_vec();
        }
        let mut result = self.lines[sr][sc.min(self.lines[sr].len())..].to_vec();
        result.push('\n');
        for row in (sr + 1)..er {
            result.extend(self.lines[row].iter());
            result.push('\n');
        }
        let last = &self.lines[er];
        let end = (ec + 1).min(last.len());
        result.extend(last[..end].iter());
        result
    }

    fn exit_visual(&mut self) {
        self.mode = EditorMode::Normal;
        self.visual_anchor = None;
        self.clamp_col();
    }

    /// `y` in Visual/VisualLine: copies the selection into `register` and
    /// leaves the cursor at its start -- doesn't mutate the buffer, so no
    /// checkpoint.
    fn yank_selection(&mut self) {
        let linewise = self.mode == EditorMode::VisualLine;
        let ((sr, sc), (er, ec)) = self.selection_bounds();
        self.register = Some(if linewise {
            Register::Linewise(self.lines[sr..=er].to_vec())
        } else {
            Register::Charwise(self.charwise_slice(sr, sc, er, ec))
        });
        self.cursor_row = sr;
        self.cursor_col = sc;
        self.exit_visual();
    }

    /// `d`/`x` (`enter_insert = false`) or `c` (`enter_insert = true`) on
    /// the current Visual/VisualLine selection: checkpoints, cuts the
    /// range into `register`, then either drops back to Normal or lands in
    /// Insert right at the cut -- real vim's `c` behavior, so the
    /// replacement can be typed immediately.
    fn delete_selection(&mut self, enter_insert: bool) {
        self.checkpoint();
        let linewise = self.mode == EditorMode::VisualLine;
        let ((sr, sc), (er, ec)) = self.selection_bounds();
        if linewise {
            self.register = Some(Register::Linewise(self.lines[sr..=er].to_vec()));
            self.lines.drain(sr..=er);
            if self.lines.is_empty() {
                self.lines.push(Vec::new());
            }
            self.cursor_row = sr.min(self.lines.len() - 1);
            self.cursor_col = 0;
        } else {
            self.register = Some(Register::Charwise(self.charwise_slice(sr, sc, er, ec)));
            let tail: Vec<char> = self.lines[er][(ec + 1).min(self.lines[er].len())..].to_vec();
            self.lines[sr].truncate(sc);
            self.lines[sr].extend(tail);
            if er > sr {
                self.lines.drain(sr + 1..=er);
            }
            self.cursor_row = sr;
            self.cursor_col = sc;
        }
        self.visual_anchor = None;
        self.mode = if enter_insert {
            EditorMode::Insert
        } else {
            EditorMode::Normal
        };
        self.clamp_col();
    }

    fn handle_visual_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => self.exit_visual(),
            KeyCode::Char('y') => self.yank_selection(),
            KeyCode::Char('d') | KeyCode::Char('x') => self.delete_selection(false),
            KeyCode::Char('c') => self.delete_selection(true),
            _ => {
                self.handle_motion_key(code, modifiers);
            }
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

    /// Adjusts `self.scroll` so the cursor's row stays on screen, counting
    /// in *visible* rows rather than raw line indices -- a folded
    /// statement's hidden body must not count toward "how far down the
    /// cursor is", or the viewport would scroll as if those lines were
    /// still taking up space. `visible` is every line index not currently
    /// hidden by a fold, ascending; the caller (`draw`) already has it.
    fn scroll_into_view(&mut self, visible: &[usize]) {
        if self.visible_height == 0 {
            return;
        }
        let Some(cursor_idx) = visible.iter().position(|&r| r == self.cursor_row) else {
            return;
        };
        let scroll_idx = visible.iter().position(|&r| r >= self.scroll).unwrap_or(0);
        if cursor_idx < scroll_idx {
            self.scroll = self.cursor_row;
        } else if cursor_idx >= scroll_idx + self.visible_height {
            self.scroll = visible[cursor_idx + 1 - self.visible_height];
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

    /// Whether `(row, col)` falls inside the current Visual/VisualLine
    /// selection -- always `false` outside those modes. Charwise checks
    /// the exact inclusive range (see `charwise_slice`); linewise counts
    /// the whole row regardless of column.
    fn is_selected(&self, row: usize, col: usize) -> bool {
        if !matches!(self.mode, EditorMode::Visual | EditorMode::VisualLine) {
            return false;
        }
        let ((sr, sc), (er, ec)) = self.selection_bounds();
        if row < sr || row > er {
            return false;
        }
        if self.mode == EditorMode::VisualLine {
            return true;
        }
        match row {
            r if r == sr && r == er => col >= sc && col <= ec,
            r if r == sr => col >= sc,
            r if r == er => col <= ec,
            _ => true,
        }
    }

    /// `fold`, when `row` is the header line of a foldable statement, is
    /// `(currently folded, hidden line count)` -- drives the gutter marker
    /// (`▸` folded / `▾` open) and, when folded, the trailing "N lines
    /// folded" summary. `None` for every other line, including inside
    /// non-SQL buffers where there's nothing to fold.
    fn render_line(
        &self,
        line: &[char],
        row: usize,
        colors: Option<&[Color]>,
        fold: Option<(bool, usize)>,
    ) -> Line<'static> {
        let theme = theme();
        let mut spans: Vec<Span<'static>> = Vec::new();
        if self.dialect == Dialect::Sql {
            let marker = match fold {
                Some((true, _)) => "▸ ",
                Some((false, _)) => "▾ ",
                None => "  ",
            };
            spans.push(Span::styled(marker, Style::default().fg(theme.text_dim)));
        }
        spans.extend(line.iter().enumerate().map(|(col, &c)| {
            let mut style = Style::default();
            if let Some(color) = colors.and_then(|c| c.get(col)) {
                style = style.fg(*color);
            }
            // Selection background first, cursor `REVERSED` on top -- the
            // cursor cell still needs to read clearly as "here" even
            // inside a highlighted selection.
            if self.is_selected(row, col) {
                style = style.bg(theme.selection_bg);
            }
            if row == self.cursor_row && col == self.cursor_col {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Span::styled(c.to_string(), style)
        }));
        if row == self.cursor_row && self.cursor_col >= line.len() {
            // Insert-mode cursor sitting just past the last character (or
            // on an empty line) -- still needs a visible cell.
            spans.push(Span::styled(
                " ",
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        } else if line.is_empty() && self.is_selected(row, 0) {
            // An empty line inside a selection (typically `VisualLine`)
            // still needs *something* highlighted, or it'd look skipped.
            spans.push(Span::styled(" ", Style::default().bg(theme.selection_bg)));
        }
        if let Some((true, count)) = fold {
            let plural = if count == 1 { "" } else { "s" };
            spans.push(Span::styled(
                format!(" ⋯ {count} line{plural} folded"),
                Style::default().fg(theme.text_dim),
            ));
        }
        Line::from(spans)
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        connection_name: &str,
        focused: bool,
        alive: bool,
        in_transaction: bool,
    ) {
        let theme = theme();
        let mut block = ui::panel(&format!("Query — {connection_name}"), focused);
        if !alive {
            // A dropped connection outranks focus for attention, so it
            // overrides the usual focused/unfocused border color rather
            // than competing with it.
            block = block.border_style(Style::default().fg(theme.error));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Reserve the last inner row for a mode indicator.
        let text_height = inner.height.saturating_sub(1);
        self.visible_height = text_height as usize;

        let fold_ranges = self.foldable_ranges();
        let hidden = self.hidden_rows();
        let visible: Vec<usize> = (0..self.lines.len())
            .filter(|r| !hidden.contains(r))
            .collect();
        self.scroll_into_view(&visible);
        let scroll_idx = visible.iter().position(|&r| r >= self.scroll).unwrap_or(0);

        let line_colors = self.line_colors();
        let lines: Vec<Line> = visible
            .iter()
            .skip(scroll_idx)
            .take(text_height as usize)
            .map(|&row| {
                let colors = line_colors.as_ref().map(|lc| lc[row].as_slice());
                let fold = fold_ranges
                    .iter()
                    .find(|&&(start, _)| start == row)
                    .map(|&(start, end)| (self.folded.contains(&start), end - start));
                self.render_line(&self.lines[row], row, colors, fold)
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
            let (mode_label, mode_color) = match self.mode {
                EditorMode::Normal => (" NORMAL ", theme.accent),
                EditorMode::Insert => (" INSERT ", theme.warning),
                EditorMode::Visual => (" VISUAL ", theme.selection_bg),
                EditorMode::VisualLine => (" V-LINE ", theme.selection_bg),
            };
            let mode_area = Rect {
                x: inner.x,
                y: inner.y + text_height,
                width: inner.width,
                height: 1,
            };
            let position = format!(" {}:{} ", self.cursor_row + 1, self.cursor_col + 1);
            let mut spans = vec![
                Span::styled(
                    mode_label,
                    Style::default()
                        .bg(mode_color)
                        .fg(theme.status_bar_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(position, Style::default().fg(theme.text_dim)),
            ];
            if !alive {
                spans.push(Span::styled(
                    " ● disconnected ",
                    Style::default()
                        .bg(theme.error)
                        .fg(theme.status_bar_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if in_transaction {
                // Silent when there's nothing open, same "only speak up for
                // the state that needs attention" rule as the disconnected
                // badge above -- auto-commit ON is the default, quiet state.
                spans.push(Span::styled(
                    " transaction open (F8 commit / F9 rollback) ",
                    Style::default()
                        .bg(theme.warning)
                        .fg(theme.status_bar_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), mode_area);
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
    fn insert_at_cursor_splits_embedded_newlines_into_real_lines() {
        let mut editor = QueryEditorComponent::new();

        editor.insert_at_cursor("INSERT INTO t (\n  a,\n  b\n);");

        assert_eq!(editor.lines.len(), 4, "lines were: {:?}", editor.lines);
        assert_eq!(editor.text(), "INSERT INTO t (\n  a,\n  b\n);");
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
            .draw(|frame| {
                editor.draw(
                    frame,
                    Rect::new(0, 0, 40, 10),
                    "local-sqlite",
                    true,
                    true,
                    false,
                )
            })
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
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true, true, false))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("NORMAL"), "buffer was: {text}");

        editor.forward_key(key(KeyCode::Char('i')));
        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true, true, false))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("INSERT"), "buffer was: {text}");
    }

    #[test]
    fn a_dead_connection_is_called_out_and_a_live_one_is_quiet() {
        let mut editor = QueryEditorComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true, true, false))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains("disconnected"),
            "a live connection should not clutter the status line: {text}"
        );

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "x", true, false, false))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("disconnected"), "buffer was: {text}");
    }

    #[test]
    fn an_open_transaction_is_called_out_and_auto_commit_is_quiet() {
        let mut editor = QueryEditorComponent::new();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 60, 10), "x", true, true, false))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains("transaction"),
            "auto-commit is the default, quiet state: {text}"
        );

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 60, 10), "x", true, true, true))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("transaction open"), "buffer was: {text}");
        assert!(text.contains("F8"), "buffer was: {text}");
        assert!(text.contains("F9"), "buffer was: {text}");
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
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 20, 8), "x", true, true, false))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains('9'),
            "last line (19) should be visible: {text}"
        );
    }

    fn draw_text(editor: &mut QueryEditorComponent) -> String {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 60, 10), "x", true, true, false))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn za_folds_a_multi_line_statement_hiding_its_body_and_showing_a_count() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;\nSELECT 2;");

        let text = draw_text(&mut editor);
        assert!(text.contains("FROM users"), "starts unfolded: {text}");

        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));
        let text = draw_text(&mut editor);

        assert!(
            !text.contains("FROM users") && !text.contains("WHERE x"),
            "folded body must not render: {text}"
        );
        assert!(text.contains("SELECT 1"), "header stays visible: {text}");
        assert!(
            text.contains("2 lines folded"),
            "placeholder reports the hidden count: {text}"
        );
        assert!(
            text.contains("SELECT 2"),
            "the second, unfolded statement is unaffected: {text}"
        );
    }

    #[test]
    fn za_again_unfolds_it() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;\nSELECT 2;");
        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        let text = draw_text(&mut editor);
        assert!(text.contains("FROM users"), "unfolded again: {text}");
        assert!(!text.contains("folded"), "no leftover placeholder: {text}");
    }

    #[test]
    fn folding_a_single_line_statement_is_a_no_op() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1;");

        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        let text = draw_text(&mut editor);
        assert!(text.contains("SELECT 1"), "buffer: {text}");
        assert!(!text.contains("folded"), "nothing to fold: {text}");
    }

    #[test]
    fn folding_does_nothing_outside_the_sql_dialect() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("db.users.find({\nstatus: 'active'\n});\ndb.users.count();");

        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        let text = draw_text(&mut editor);
        assert!(
            text.contains("status: 'active'"),
            "plain-text buffers have no fold concept: {text}"
        );
    }

    #[test]
    fn moving_down_past_a_folded_statement_lands_after_it() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;\nSELECT 2;");
        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Char('j')));
        editor.insert_at_cursor("X");

        assert_eq!(
            editor.text(),
            "SELECT 1\nFROM users\nWHERE x = 1;\nXSELECT 2;",
            "j from the fold header must skip its hidden body entirely"
        );
    }

    #[test]
    fn starting_to_edit_a_folded_statement_unfolds_it_first() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;\nSELECT 2;");
        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Char('i')));

        let text = draw_text(&mut editor);
        assert!(
            text.contains("FROM users"),
            "typing into a folded statement must reveal it, not bury the new text: {text}"
        );
    }

    #[test]
    fn set_text_clears_folds_from_the_previous_buffer() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;");
        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;");

        let text = draw_text(&mut editor);
        assert!(
            text.contains("FROM users"),
            "a freshly loaded buffer must not inherit stale fold state: {text}"
        );
    }

    // --- registers: dd/x/yy/p/P ---

    #[test]
    fn dd_stages_the_deleted_line_for_paste() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb\nc");
        editor.forward_key(key(KeyCode::Char('j')));
        editor.forward_key(key(KeyCode::Char('d')));
        editor.forward_key(key(KeyCode::Char('d')));
        assert_eq!(editor.text(), "a\nc");

        // The cursor now sits on "c" (what used to be line 3); `P` pastes
        // *before* it, restoring the original order. (`p` would paste
        // after and give "a\nc\nb" instead -- also correct, just not a
        // round trip.)
        editor.forward_key(key(KeyCode::Char('P')));

        assert_eq!(editor.text(), "a\nb\nc");
    }

    #[test]
    fn x_stages_the_deleted_character_for_paste() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");
        editor.forward_key(key(KeyCode::Char('x')));
        assert_eq!(editor.text(), "bc");

        editor.forward_key(key(KeyCode::Char('p')));

        assert_eq!(editor.text(), "bac", "x then p is vim's classic swap");
    }

    #[test]
    fn yy_copies_the_current_line_without_deleting_it() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb");

        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('y')));

        assert_eq!(editor.text(), "a\nb", "yy must not mutate the buffer");

        editor.forward_key(key(KeyCode::Char('p')));

        assert_eq!(editor.text(), "a\na\nb");
    }

    #[test]
    fn shift_p_pastes_a_linewise_register_above_the_cursor() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb");
        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('j')));

        editor.forward_key(key(KeyCode::Char('P')));

        assert_eq!(editor.text(), "a\na\nb");
    }

    #[test]
    fn paste_with_an_empty_register_is_a_no_op() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('p')));

        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn u_undoes_a_paste() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nb");
        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('p')));
        assert_eq!(editor.text(), "a\na\nb");

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(editor.text(), "a\nb");
    }

    // --- Visual/VisualLine mode ---

    #[test]
    fn v_then_esc_leaves_the_buffer_untouched_and_returns_to_normal() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        editor.forward_key(key(KeyCode::Char('v')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Esc));

        assert_eq!(editor.mode, EditorMode::Normal);
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn visual_charwise_yank_then_paste_round_trips_the_selection() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abcdef");

        editor.forward_key(key(KeyCode::Char('v')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('y')));

        assert_eq!(
            editor.mode,
            EditorMode::Normal,
            "y exits Visual mode back to Normal"
        );
        assert_eq!(editor.text(), "abcdef", "y must not mutate the buffer");

        // Cursor lands at the start of what was just yanked (column 0);
        // `P` pastes "abc" (the inclusive 3-char selection) before it.
        editor.forward_key(key(KeyCode::Char('P')));

        assert_eq!(editor.text(), "abcabcdef");
    }

    #[test]
    fn visual_charwise_delete_spanning_lines_joins_the_remainder() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("hello\nworld");
        // Select from column 3 of line 0 ("lo\nwor" inclusive) to column 2
        // of line 1: "hel" (kept) + "lo\nwor" (cut) + "ld" (kept) = "helld".
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('v')));
        editor.forward_key(key(KeyCode::Char('j')));
        editor.forward_key(key(KeyCode::Char('h')));

        editor.forward_key(key(KeyCode::Char('d')));

        assert_eq!(editor.text(), "helld");
    }

    #[test]
    fn visual_line_delete_removes_whole_lines_regardless_of_column() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("aaa\nbbb\nccc");
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('V')));
        editor.forward_key(key(KeyCode::Char('j')));

        editor.forward_key(key(KeyCode::Char('d')));

        assert_eq!(editor.text(), "ccc");
    }

    #[test]
    fn visual_line_yank_then_paste_inserts_whole_lines() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("aaa\nbbb\nccc");
        editor.forward_key(key(KeyCode::Char('V')));
        editor.forward_key(key(KeyCode::Char('j')));

        editor.forward_key(key(KeyCode::Char('y')));
        editor.forward_key(key(KeyCode::Char('G')));
        editor.forward_key(key(KeyCode::Char('p')));

        assert_eq!(editor.text(), "aaa\nbbb\nccc\naaa\nbbb");
    }

    #[test]
    fn visual_c_deletes_the_selection_and_enters_insert_mode() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abcdef");

        editor.forward_key(key(KeyCode::Char('v')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('c')));

        assert_eq!(editor.mode, EditorMode::Insert);
        assert_eq!(editor.text(), "def");

        type_str(&mut editor, "XYZ");
        assert_eq!(editor.text(), "XYZdef");
    }

    #[test]
    fn u_undoes_a_visual_delete() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abcdef");
        editor.forward_key(key(KeyCode::Char('v')));
        editor.forward_key(key(KeyCode::Char('l')));
        editor.forward_key(key(KeyCode::Char('d')));
        assert_eq!(editor.text(), "cdef");

        editor.forward_key(key(KeyCode::Char('u')));

        assert_eq!(editor.text(), "abcdef");
    }

    #[test]
    fn moving_in_visual_mode_past_a_folded_statement_skips_it() {
        let mut editor = QueryEditorComponent::new();
        editor.set_dialect(Dialect::Sql);
        editor.set_text("SELECT 1\nFROM users\nWHERE x = 1;\nSELECT 2;");
        editor.forward_key(key(KeyCode::Char('z')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Char('V')));
        editor.forward_key(key(KeyCode::Char('j')));

        assert_eq!(
            editor.cursor_row, 3,
            "Visual-mode movement funnels through the same move_row_to as Normal"
        );
    }

    // --- find (buffer search) ---

    #[test]
    fn find_moves_the_cursor_to_the_next_match() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("select id from users where id = 1");

        let found = editor.find("from", false);

        assert!(found);
        assert_eq!(editor.cursor_col, 10);
    }

    #[test]
    fn find_is_case_insensitive() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("SELECT * FROM users");

        assert!(editor.find("from", false));
        assert_eq!(editor.cursor_col, 9);
    }

    #[test]
    fn find_wraps_around_the_buffer() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("a\nneedle\nb");
        editor.forward_key(key(KeyCode::Char('G'))); // last line, past the match

        let found = editor.find("needle", false);

        assert!(found);
        assert_eq!(editor.cursor_row, 1);
    }

    #[test]
    fn find_backwards_finds_the_previous_match() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("needle a needle b");
        editor.forward_key(key(KeyCode::Char('$')));

        let found = editor.find("needle", true);

        assert!(found);
        assert_eq!(editor.cursor_col, 9);
    }

    #[test]
    fn find_with_no_match_leaves_the_cursor_untouched() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");
        editor.forward_key(key(KeyCode::Char('l')));

        let found = editor.find("zzz", false);

        assert!(!found);
        assert_eq!(editor.cursor_col, 1);
    }

    #[test]
    fn find_with_an_empty_pattern_is_a_no_op() {
        let mut editor = QueryEditorComponent::new();
        editor.set_text("abc");

        assert!(!editor.find("", false));
    }
}
