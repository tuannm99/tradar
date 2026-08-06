# Vim-Style Modal Query Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the query editor's plain-`String` input with a real vim-modal editor backed by `edtui`, wired into `QueryScreenComponent` exactly as today's plain-text editor is, with no other user-visible behavior change.

**Architecture:** `QueryEditorComponent` (`src/components/query_editor.rs`) swaps its `query_input: String` field for `edtui::EditorState` + `edtui::EditorEventHandler`, and gains a small surface (`text()`, `insert_at_cursor()`, `forward_key()`) that `QueryScreenComponent` uses instead of the old `push_char`/`backspace`/`query_input` accesses. `QueryScreenComponent::handle_key_event` keeps intercepting the same handful of keys it already does (`Esc`, `Tab`, `Ctrl+y`, submit, sidebar-focus arrows) and forwards everything else straight into the editor.

**Tech Stack:** Rust, `ratatui` (bumped `0.29` → `0.30`), `crossterm` `0.29` (unchanged), `edtui` `0.11.6` (new, `default-features = false`).

## Global Constraints

- No change to any other screen, keybinding, or rendering behavior (per the design spec's goal #2) — `Tab`, `F5`/`Ctrl+Enter`, `Ctrl+y`, and sidebar navigation must behave exactly as they do today.
- `edtui` is added with `default-features = false` — no `arboard` (clipboard), `mouse-support`, or `syntax-highlighting`. These pull in extra dependencies (notably `syntect`) not needed for modal editing itself.
- Schema-name insertion happens at the editor's current cursor position (not appended at the buffer's end), and leaves the editor in Insert mode afterward.
- `Esc` transitions Insert/Visual/Search → Normal mode inside the editor first; it only produces `Action::BackToPicker` once the editor is already in Normal mode.
- Every step in this plan has been hand-verified against the real `edtui 0.11.6` API (built, tested, `clippy`, and `fmt`-checked in a throwaway working-tree change before this plan was written) — the code blocks below are not speculative.

---

### Task 1: Bump `ratatui` to 0.30 and add the `edtui` dependency

**Files:**
- Modify: `Cargo.toml:8` (the `ratatui` line), and add a new line after `Cargo.toml:25` (`futures-util = "0.3"`)

**Interfaces:**
- Consumes: nothing.
- Produces: the `ratatui = "0.30"` and `edtui = { version = "0.11.6", default-features = false }` dependencies that Tasks 2 and 3 build on.

This bump was verified to compile and pass the full existing test suite with **zero** source changes elsewhere in the crate — this task is a pure dependency-version change.

- [ ] **Step 1: Bump `ratatui` and add `edtui` in `Cargo.toml`**

Change line 8 of `Cargo.toml` from:

```toml
ratatui = "0.29"
```

to:

```toml
ratatui = "0.30"
```

Then add this line right after `futures-util = "0.3"` (currently `Cargo.toml:25`), inside the `[dependencies]` table:

```toml
edtui = { version = "0.11.6", default-features = false }
```

- [ ] **Step 2: Update the lockfile and build**

Run: `cargo build`
Expected: succeeds (this will update `Cargo.lock`, pulling in `ratatui 0.30.x`, its `ratatui-core`/`ratatui-widgets`/`ratatui-crossterm` split crates, `edtui 0.11.6`, and `edtui-jagged`). No source file changes are needed for this to compile.

- [ ] **Step 3: Run the full non-Docker test suite**

Run: `cargo test --lib -- --skip drivers::elasticsearch --skip drivers::postgres --skip drivers::mongo --skip drivers::redis`
Expected: all tests pass, unchanged from before the bump (this confirms `ratatui 0.30` didn't silently change any rendering/layout behavior the existing tests cover).

- [ ] **Step 4: Run clippy and fmt**

Run: `cargo clippy --all-targets` — expected: no warnings.
Run: `cargo fmt --check` — expected: no diff.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "Bump ratatui to 0.30 and add edtui for the vim-modal query editor"
```

---

### Task 2: Rewrite `QueryEditorComponent` around `edtui::EditorState`

**Files:**
- Modify: `src/components/query_editor.rs` (full rewrite of both the implementation and its test module)

**Interfaces:**
- Consumes: `edtui::{EditorState, EditorEventHandler, EditorMode, EditorTheme, EditorView, Lines, actions::InsertChar}`, `crossterm::event::KeyEvent` (all from Task 1's new dependency).
- Produces (for Task 3):
  - `pub struct QueryEditorComponent { pub state: EditorState, .. }` — the `state` field, specifically `state.mode: EditorMode`, is read directly by `QueryScreenComponent`.
  - `pub fn text(&self) -> String` — flattened buffer content.
  - `pub fn insert_at_cursor(&mut self, text: &str)` — inserts at the cursor, switches to `EditorMode::Insert`.
  - `pub fn forward_key(&mut self, key: KeyEvent)` — hands a raw key event to the editor.
  - `pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str)` — same signature as today.
  - `pub fn new() -> Self` and `impl Default` — same as today.
  - Removed: `query_input` field, `push_char`, `backspace`.

- [ ] **Step 1: Replace the test module with tests against the new API**

Replace everything from `#[cfg(test)]` to the end of `src/components/query_editor.rs` with:

```rust
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
    fn insert_at_cursor_inserts_text_and_switches_to_insert_mode() {
        let mut editor = QueryEditorComponent::new();

        editor.insert_at_cursor("users");

        assert_eq!(editor.text(), "users");
        assert_eq!(editor.state.mode, EditorMode::Insert);
    }

    #[test]
    fn draw_shows_the_connection_name_and_input() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("x");
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "local-sqlite"))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }
}
```

- [ ] **Step 2: Verify it fails to compile against the current (pre-rewrite) implementation**

Run: `cargo test --lib components::query_editor`
Expected: **compile failure** — `no method named 'forward_key' found`, `no method named 'text' found`, `no method named 'insert_at_cursor' found`, and `no field 'state' on type QueryEditorComponent`. This confirms the tests exercise the new API, which doesn't exist yet.

- [ ] **Step 3: Replace the implementation**

Replace everything from the top of `src/components/query_editor.rs` up to (not including) `#[cfg(test)]` with:

```rust
use crossterm::event::KeyEvent;
use edtui::actions::InsertChar;
use edtui::{EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Lines};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

pub struct QueryEditorComponent {
    pub state: EditorState,
    event_handler: EditorEventHandler,
}

impl Default for QueryEditorComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryEditorComponent {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(Lines::default()),
            event_handler: EditorEventHandler::default(),
        }
    }

    pub fn text(&self) -> String {
        self.state
            .lines
            .iter_row()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\n")
    }

    pub fn insert_at_cursor(&mut self, text: &str) {
        for c in text.chars() {
            self.state.execute(InsertChar(c));
        }
        self.state.mode = EditorMode::Insert;
    }

    pub fn forward_key(&mut self, key: KeyEvent) {
        self.event_handler.on_key_event(key, &mut self.state);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Query — {connection_name}"));
        let view = EditorView::new(&mut self.state)
            .theme(EditorTheme::default().block(block))
            .wrap(true);
        frame.render_widget(view, area);
    }
}

```

(Keep the `#[cfg(test)] mod tests { ... }` block from Step 1 immediately after this.)

Note on `text()`: `state.lines` is `edtui::Lines` (a type alias for `edtui_jagged::Jagged<char>`). `Jagged<T>::iter_row(&self)` returns an iterator of `&Vec<char>`, one per row; `row.iter().collect::<String>()` relies on the standard library's `impl<'a> FromIterator<&'a char> for String`.

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib components::query_editor`
Expected: all 4 tests pass.

- [ ] **Step 5: Run clippy and fmt**

Run: `cargo clippy --all-targets` — expected: no warnings.
Run: `cargo fmt --check` — expected: no diff (run `cargo fmt` first if there is one, then re-check).

- [ ] **Step 6: Commit**

```bash
git add src/components/query_editor.rs
git commit -m "Rewrite QueryEditorComponent around edtui::EditorState for vim-modal editing"
```

---

### Task 3: Wire `QueryScreenComponent` to the vim-modal editor

**Files:**
- Modify: `src/components/query_screen.rs` (imports, `handle_key_event`, the `InsertSchemaSelection` and `SubmitQuery` arms of `update`, `draw`'s layout, and the test module)

**Interfaces:**
- Consumes (from Task 2): `QueryEditorComponent::{text, insert_at_cursor, forward_key}`, the `state: EditorState` field and `state.mode: EditorMode`.
- Produces: no new public interface — this task is the integration point; nothing else in the codebase depends on `QueryScreenComponent`'s internals beyond what `RootComponent` already uses (`active_connection`, `engine`, etc., all unchanged).

This is one cohesive change (routing, the two call sites that read editor text, and the layout height it depends on) — splitting it into smaller tasks would leave the crate in a non-compiling state partway through, so it's done as a single RED → GREEN cycle across the whole file.

- [ ] **Step 1: Update the failing/changed tests**

In `src/components/query_screen.rs`, apply these test-module changes (all verified against the real implementation from Step 3 below):

**1a.** Add `KeyEvent` to the crossterm import and add the `EditorMode` import, at the top of the file (`query_screen.rs:8`):

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use edtui::EditorMode;
use ratatui::Frame;
```

**1b.** Replace the `insert_schema_selection_appends_the_selected_name_and_returns_focus_to_editor` test (`query_screen.rs:291-303`) and the assertion in `insert_schema_selection_is_a_no_op_when_schema_is_empty` (`query_screen.rs:305-318`) with:

```rust
    fn type_chars(screen: &mut QueryScreenComponent, chars: &str) {
        for c in chars.chars() {
            screen
                .query_editor
                .forward_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn insert_schema_selection_inserts_the_selected_name_at_the_cursor() {
        let (mut screen, _rx) = screen();
        screen.schema_sidebar.set_schema(schema());
        screen.schema_sidebar.move_down();
        // Type "ab" then leave Insert mode -- vim leaves the cursor sitting
        // on the last-typed character ("b"), so the inserted name must land
        // between "a" and "b", not appended at the buffer's end.
        type_chars(&mut screen, "iab");
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        screen.focus = Focus::Sidebar;

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.text(), "aordersb");
        assert_eq!(screen.focus, Focus::Editor);
        assert_eq!(screen.query_editor.state.mode, EditorMode::Insert);
    }

    #[test]
    fn insert_schema_selection_is_a_no_op_when_schema_is_empty() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Sidebar;

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.text(), "");
        assert_eq!(
            screen.focus,
            Focus::Sidebar,
            "no-op must not change focus either"
        );
    }
```

**1c.** In `submit_query_runs_the_query_and_reports_query_completed` (`query_screen.rs:393`), replace:

```rust
        screen.query_editor.push_char('x');
```

with:

```rust
        screen.query_editor.insert_at_cursor("x");
```

**1d.** In both `ctrl_enter_runs_the_query_instead_of_inserting_the_schema_selection_when_sidebar_focused` and `f5_runs_the_query_instead_of_being_swallowed_by_the_sidebar_guard` (`query_screen.rs:509-529`), replace each:

```rust
        screen.query_editor.push_char('x');
```

with:

```rust
        screen.query_editor.insert_at_cursor("x");
```

and each:

```rust
        assert_eq!(screen.query_editor.query_input, "x");
```

with:

```rust
        assert_eq!(screen.query_editor.text(), "x");
```

**1e.** After `esc_returns_back_to_picker` (`query_screen.rs:560-567`), insert two new regression tests and one more (`Ctrl+y` while the sidebar has focus):

```rust
    #[test]
    fn esc_in_insert_mode_returns_to_normal_mode_instead_of_the_picker() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert_eq!(screen.query_editor.state.mode, EditorMode::Insert);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(
            action.is_none(),
            "Esc must be consumed by the editor, not bubble to BackToPicker"
        );
        assert_eq!(screen.query_editor.state.mode, EditorMode::Normal);
    }

    #[test]
    fn esc_in_normal_mode_returns_back_to_picker_even_after_leaving_insert_mode() {
        let (mut screen, _rx) = screen();
        screen
            .query_editor
            .forward_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(screen.query_editor.state.mode, EditorMode::Normal);

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    #[test]
    fn ctrl_y_exports_curl_even_while_the_sidebar_has_focus() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.active_connection = Some(connection());
        screen.query_editor.insert_at_cursor("select 1");

        let action = screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::CONTROL);

        match action {
            Some(Action::ExportCurl {
                connection: c,
                query,
            }) => {
                assert_eq!(c, connection());
                assert_eq!(query, "select 1");
            }
            other => panic!(
                "expected ExportCurl, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }
            ),
        }
    }
```

**1f.** In `draw_shows_active_connection_and_input` (`query_screen.rs:569-584`), replace:

```rust
        screen.query_editor.push_char('x');
```

with:

```rust
        screen.query_editor.insert_at_cursor("x");
```

- [ ] **Step 2: Verify the test suite fails to compile**

Run: `cargo test --lib components::query_screen`
Expected: compile failure — `no method named 'push_char'`, `no method named 'insert_at_cursor'`, `no field 'query_input'`, `no method named 'forward_key'`, unresolved import `edtui::EditorMode` (crate not yet used by this file). This confirms the tests target the not-yet-written integration code.

- [ ] **Step 3: Update `handle_key_event`**

Replace the `handle_key_event` function body (`query_screen.rs:58-90`) with:

```rust
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Esc if self.query_editor.state.mode != EditorMode::Normal => {
                self.query_editor
                    .forward_key(KeyEvent::new(code, modifiers));
                None
            }
            KeyCode::Esc => Some(Action::BackToPicker),
            KeyCode::Tab => Some(Action::ToggleFocus),
            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => self
                .active_connection
                .clone()
                .map(|connection| Action::ExportCurl {
                    connection,
                    query: self.query_editor.text(),
                }),
            _ if is_submit(code, modifiers) => Some(Action::SubmitQuery),
            _ if self.focus == Focus::Sidebar => match code {
                KeyCode::Down | KeyCode::Char('j') => Some(Action::SchemaMoveDown),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::SchemaMoveUp),
                KeyCode::Enter => Some(Action::InsertSchemaSelection),
                _ => None,
            },
            _ => {
                self.query_editor
                    .forward_key(KeyEvent::new(code, modifiers));
                None
            }
        }
    }
```

- [ ] **Step 4: Update `InsertSchemaSelection` and `SubmitQuery` in `update`**

In the `Action::InsertSchemaSelection` arm (`query_screen.rs:132-138`), replace:

```rust
                    self.query_editor.query_input.push_str(&name);
```

with:

```rust
                    self.query_editor.insert_at_cursor(&name);
```

In the `Action::SubmitQuery` arm (`query_screen.rs:142`), replace:

```rust
                let query = self.query_editor.query_input.clone();
```

with:

```rust
                let query = self.query_editor.text();
```

- [ ] **Step 5: Give the editor pane enough rows to show its content**

`edtui`'s default theme renders a one-row status line (showing the current mode, e.g. "Insert") inside the editor's content area. The current layout (`query_screen.rs:203-206`) gives the editor block a fixed height of 3 (one row of usable content after the top/bottom border) — with the status line taking that one row, typed text would never be visible. Replace:

```rust
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(outer[1]);
```

with:

```rust
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Min(1)])
            .split(outer[1]);
```

This gives the editor block 4 rows of usable content (1 for the status line, 3 for text) instead of 1, and leaves the results panel exactly as much room as before minus the extra 3 rows this takes (via `Constraint::Min(1)`, which absorbs whatever's left).

- [ ] **Step 6: Verify the tests pass**

Run: `cargo test --lib components::query_screen`
Expected: all tests pass, including the two new `Esc`-mode tests, the new `Ctrl+y`-while-sidebar-focused test, and the updated schema-insertion/submit/draw tests.

- [ ] **Step 7: Run the full test suite, clippy, and fmt**

Run: `cargo test --lib -- --skip drivers::elasticsearch --skip drivers::postgres --skip drivers::mongo --skip drivers::redis`
Expected: all tests pass (this crate-wide run catches anything in `components::mod` or elsewhere that touches `QueryScreenComponent`/`QueryEditorComponent`).

Run: `cargo clippy --all-targets` — expected: no warnings.
Run: `cargo fmt --check` — expected: no diff (run `cargo fmt` first if there is one, then re-check).

- [ ] **Step 8: Manually verify in a real terminal**

Run: `cargo run` against a saved SQLite connection (per `docs/superpowers/specs/2026-08-01-tradar-v1-design.md`'s connections-file setup if none exists yet). Confirm:
- The editor starts in Normal mode; pressing `i` enters Insert mode and typing appears in the box.
- `Esc` in Insert mode returns to Normal mode (the box stays open); `Esc` again returns to the connection picker.
- `Tab` still toggles focus to the schema sidebar and back.
- Selecting a schema item with `Enter` inserts it at the cursor and returns focus to the editor in Insert mode.
- `F5` and `Ctrl+Enter` still submit the query; `Ctrl+y` still writes `tradar-query.sh` for an Elasticsearch connection.

- [ ] **Step 9: Commit**

```bash
git add src/components/query_screen.rs
git commit -m "Wire QueryScreenComponent to the vim-modal query editor"
```

---

## Self-Review Notes

- **Spec coverage:** Goal 1 (real modal editor via `edtui`) — Task 2. Goal 2 (preserve every existing interaction) — Task 3's routing keeps `Tab`/`F5`/`Ctrl+Enter`/`Ctrl+y`/sidebar-arrows unconditional, only `Esc` gains a mode check. Goal 3 (context-sensitive `Esc`) — Task 3 Step 3 + regression tests in Step 1e. Goal 4 (minimal dependency footprint) — Task 1's `default-features = false`. The two backlog-flagged test gaps (`Esc` arm ordering, `Ctrl+y` under sidebar focus) are closed in Task 3 Step 1e.
- **Placeholder scan:** none — every step has runnable code or an exact command.
- **Type consistency:** `text()`, `insert_at_cursor()`, `forward_key()`, and `state` are named identically across Tasks 2 and 3; double-checked against the verified working code both tasks were extracted from.
