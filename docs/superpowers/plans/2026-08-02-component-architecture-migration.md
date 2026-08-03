# Component Architecture Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat `App`(`src/app/mod.rs`) + `handle_key`(`main.rs`) + `tui::draw`(`src/tui/mod.rs`) design with a `Component` trait + `Action` enum + `mpsc`-channel architecture, with **zero user-visible behavior change**.

**Architecture:** New `src/action.rs` defines `Action` and the `Component` trait. New `src/components/` holds `RootComponent` (screen switcher), `ConnectionPickerComponent` and `QueryScreenComponent` (both implement `Component`), and three plain state+draw structs composed by `QueryScreenComponent`: `SchemaSidebarComponent`, `QueryEditorComponent`, `ResultsComponent`. `main.rs` becomes: terminal setup, an `mpsc::unbounded_channel::<Action>()`, an event→action→drain→dispatch loop, and the sole place a concrete `Box<dyn Driver>` gets constructed (intercepting `Action::ConnectRequested`) or a concrete driver helper gets called (intercepting `Action::ExportCurl`, for `elasticsearch::to_curl`).

**Tech Stack:** Rust (edition 2024), `ratatui`/`crossterm` (unchanged versions — this migration does not touch driver code or dependencies), `tokio::sync::mpsc::unbounded_channel` (new use of an existing dependency — `tokio` is already a full-featured dependency).

## Global Constraints

- Isolation rule (`docs/architecture.md`): nothing under `src/components/` or `src/action.rs` may depend on a concrete driver module (`drivers::postgres`, `drivers::mongo`, `drivers::elasticsearch`, `drivers::redis`, `drivers::sqlite`). Only `main.rs` may. This governs two specific escape valves: `Action::ConnectRequested` (driver construction) and `Action::ExportCurl` (calls `drivers::elasticsearch::to_curl`) are both handled by `main.rs`'s action-draining loop, never inside any `Component`.
- No user-visible behavior change: every existing keybinding, screen transition, and rendered output must be identical after this migration. This includes the exact key-arm order fixed in the schema-sidebar work (`Esc`, `Tab`, `Ctrl+Y`, submit (`F5`/`Ctrl+Enter`), sidebar-focus guard, then plain `Enter`/`Backspace`/`Char`).
- TDD: write the failing test first, run it to confirm the failure, then the minimal implementation, per `superpowers:test-driven-development`.
- Every existing test must be **ported** (same assertions, new location) — not dropped and rewritten from scratch — except two tests that become structurally redundant by the new component split (`set_result_preserves_the_query_input`, `set_error_keeps_the_query_input_so_it_can_be_fixed`): these asserted that setting a result/error on `App` didn't touch `query_input`, which is now guaranteed by construction (results and the query editor are separate structs with no shared field) rather than by a runtime check. Task 2 notes exactly where these are dropped and why.
- `cargo clippy --all-targets` and `cargo fmt --check` must be clean before every task's commit.
- Every driver-integration test (Postgres/Mongo/Elasticsearch use `testcontainers-modules`, need Docker) must keep passing unmodified — this plan touches no file under `src/drivers/`. Verify the rest with `cargo test --lib --bins -- --skip drivers::postgres --skip drivers::mongo --skip drivers::elasticsearch`.

---

### Task 1: `Action` enum and `Component` trait

**Files:**
- Create: `src/action.rs`
- Modify: `src/lib.rs` (add `pub mod action;`)

**Interfaces:**
- Consumes: `crate::drivers::{QueryResult, SchemaInfo}`, `crate::query_engine::QueryEngine`, `crate::storage::SavedConnection`.
- Produces (used by every later task): the `Action` enum (all variants below) and:
  ```rust
  pub trait Component {
      fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
      fn update(&mut self, action: Action) -> Option<Action>;
      fn draw(&mut self, frame: &mut Frame, area: Rect);
  }
  ```

- [ ] **Step 1: Write `src/action.rs`**

```rust
//! The `Action` message type and the `Component` trait every top-level
//! screen implements. Depends only on the `Driver` trait's associated
//! types and `QueryEngine` — never a concrete driver module, so this
//! stays safe for every `components/` file to depend on.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::drivers::{QueryResult, SchemaInfo};
use crate::query_engine::QueryEngine;
use crate::storage::SavedConnection;

pub enum Action {
    Quit,
    ConnectRequested(SavedConnection),
    Connected {
        connection: SavedConnection,
        engine: QueryEngine,
        schema: Result<Vec<SchemaInfo>, String>,
    },
    ConnectFailed(String),
    ToggleFocus,
    SchemaMoveUp,
    SchemaMoveDown,
    InsertSchemaSelection,
    SubmitQuery,
    QueryCompleted {
        engine: QueryEngine,
        result: QueryResult,
    },
    QueryFailed {
        engine: QueryEngine,
        error: String,
    },
    ExportCurl {
        connection: SavedConnection,
        query: String,
    },
    BackToPicker,
}

pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
```

No test for this step — it's a type/trait declaration with no behavior of its own; its correctness is exercised by every component that implements/uses it in later tasks.

- [ ] **Step 2: Wire the module into the crate**

In `src/lib.rs`, add `pub mod action;` (keep the existing `pub mod app;` and `pub mod tui;` for now — they're deleted in Task 7 once nothing references them).

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --lib`
Expected: succeeds (this file has no callers yet, so it just needs to type-check standalone).

- [ ] **Step 4: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check` (run `cargo fmt` and re-check if it reports diffs).

- [ ] **Step 5: Commit**

```bash
git add src/action.rs src/lib.rs
git commit -m "Add the Action enum and Component trait"
```

---

### Task 2: `ResultsComponent`

**Files:**
- Create: `src/components/results.rs`
- Modify: `src/lib.rs` (add `pub mod components;` if not already present from a prior task in this plan — Task 2 is the first to create a file under `src/components/`, so add both the `pub mod components;` line here and create `src/components/mod.rs` with just `pub mod results;` for now; later tasks add more `pub mod` lines to it)

**Interfaces:**
- Consumes: `crate::drivers::QueryResult`.
- Produces (used by Task 6): `pub struct ResultsComponent { pub last_result: Option<QueryResult>, pub last_error: Option<String> }`, `ResultsComponent::new() -> Self`, `set_result(&mut self, result: QueryResult)`, `set_error(&mut self, error: String)`, `draw(&mut self, frame: &mut Frame, area: Rect)`. This struct does **not** implement `Component` — it has no keys routed to it directly; `QueryScreenComponent` (Task 6) drives it via direct method calls.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;
    use crate::drivers::QueryResult;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn draw_component(component: &mut ResultsComponent, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, Rect::new(0, 0, width, height)))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());

        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
        });

        assert!(results.last_error.is_none());
        assert_eq!(
            results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
        });

        results.set_error("boom".to_string());

        assert!(results.last_result.is_none());
        assert_eq!(results.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn draw_shows_the_last_table_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["42".to_string()]],
        });

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("42"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_documents_pretty_printed() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![serde_json::json!({"name": "Ada"})]));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("Ada"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_the_last_error() {
        let mut results = ResultsComponent::new();
        results.set_error("syntax error".to_string());

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("syntax error"), "buffer was: {text}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components::results`
Expected: FAIL to compile — `ResultsComponent` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

```rust
//! The results/error pane on the query screen. Owns no keys of its own —
//! driven entirely by `QueryScreenComponent` calling `set_result`/`set_error`
//! in reaction to `Action::QueryCompleted`/`Action::QueryFailed`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::drivers::QueryResult;

pub struct ResultsComponent {
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
}

impl ResultsComponent {
    pub fn new() -> Self {
        Self {
            last_result: None,
            last_error: None,
        }
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.last_result = Some(result);
        self.last_error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.last_result = None;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let body_text = if let Some(error) = &self.last_error {
            error.clone()
        } else if let Some(result) = &self.last_result {
            match result {
                QueryResult::Table { columns, rows } => {
                    let header = columns.join(" | ");
                    let rows = rows
                        .iter()
                        .map(|row| row.join(" | "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("{header}\n{rows}")
                }
                QueryResult::Documents(docs) => docs
                    .iter()
                    .map(|doc| serde_json::to_string_pretty(doc).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            }
        } else {
            String::new()
        };
        let body = Paragraph::new(body_text)
            .block(Block::default().borders(Borders::ALL).title("Results"));
        frame.render_widget(body, area);
    }
}
```

In `src/components/mod.rs` (new file):

```rust
pub mod results;
```

In `src/lib.rs`, add `pub mod components;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::results`
Expected: PASS, all 5 tests.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`.

- [ ] **Step 6: Commit**

```bash
git add src/components/ src/lib.rs
git commit -m "Add ResultsComponent, ported from App's result/error state"
```

**Note for the implementer:** the original `App` tests `set_result_preserves_the_query_input` and `set_error_keeps_the_query_input_so_it_can_be_fixed` are intentionally **not** ported anywhere — they asserted that setting a result/error didn't touch `query_input`, which is now structurally guaranteed: `ResultsComponent` has no `query_input` field to touch. Do not add a test trying to recreate this check.

---

### Task 3: `SchemaSidebarComponent`

**Files:**
- Create: `src/components/schema_sidebar.rs`
- Modify: `src/components/mod.rs` (add `pub mod schema_sidebar;`)

**Interfaces:**
- Consumes: `crate::drivers::SchemaInfo`.
- Produces (used by Task 6): `pub struct SchemaSidebarComponent { pub schema: Vec<SchemaInfo>, pub schema_selected: usize, pub schema_error: Option<String> }`, `new() -> Self`, `set_schema(&mut self, schema: Vec<SchemaInfo>)`, `set_schema_error(&mut self, error: String)`, `move_down(&mut self)`, `move_up(&mut self)`, `selected_name(&self) -> Option<&str>`, `reset(&mut self)`, `draw(&mut self, frame: &mut Frame, area: Rect, focused: bool)`. Not a `Component` — driven by `QueryScreenComponent`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::*;
    use crate::drivers::SchemaInfo;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo { name: "users".to_string() },
            SchemaInfo { name: "orders".to_string() },
        ]
    }

    fn draw_component(component: &mut SchemaSidebarComponent, focused: bool) -> (String, Buffer) {
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, Rect::new(0, 0, 64, 10), focused))
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
        assert_eq!(sidebar.schema_selected, 1, "should stop at the last item, not wrap");
    }

    #[test]
    fn move_up_retreats_and_stops_at_zero() {
        let mut sidebar = SchemaSidebarComponent::new();
        sidebar.set_schema(schema());
        sidebar.move_down();

        sidebar.move_up();
        assert_eq!(sidebar.schema_selected, 0);

        sidebar.move_up();
        assert_eq!(sidebar.schema_selected, 0, "should stop at zero, not go negative");
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
        sidebar.set_schema(vec![SchemaInfo { name: "users".to_string() }]);

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
            SchemaInfo { name: "aaa".to_string() },
            SchemaInfo { name: "bbb".to_string() },
            SchemaInfo { name: "ccc".to_string() },
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components::schema_sidebar`
Expected: FAIL to compile — `SchemaSidebarComponent` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

```rust
//! The schema (table/collection/index/key) sidebar on the query screen.
//! Not a `Component` — driven entirely by `QueryScreenComponent`, which
//! owns whether it currently has keyboard focus.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::drivers::SchemaInfo;

pub struct SchemaSidebarComponent {
    pub schema: Vec<SchemaInfo>,
    pub schema_selected: usize,
    pub schema_error: Option<String>,
}

impl SchemaSidebarComponent {
    pub fn new() -> Self {
        Self {
            schema: Vec::new(),
            schema_selected: 0,
            schema_error: None,
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

    pub fn move_down(&mut self) {
        if self.schema_selected + 1 < self.schema.len() {
            self.schema_selected += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.schema_selected = self.schema_selected.saturating_sub(1);
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.schema.get(self.schema_selected).map(|s| s.name.as_str())
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

        let title = if focused { "Schema [focused]" } else { "Schema" };

        let Some(error) = &self.schema_error else {
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
```

Add `pub mod schema_sidebar;` to `src/components/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::schema_sidebar`
Expected: PASS, all 9 tests.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`.

- [ ] **Step 6: Commit**

```bash
git add src/components/
git commit -m "Add SchemaSidebarComponent, ported from App's schema state"
```

---

### Task 4: `QueryEditorComponent`

**Files:**
- Create: `src/components/query_editor.rs`
- Modify: `src/components/mod.rs` (add `pub mod query_editor;`)

**Interfaces:**
- Consumes: nothing beyond `std`/`ratatui`.
- Produces (used by Task 6): `pub struct QueryEditorComponent { pub query_input: String }`, `new() -> Self`, `push_char(&mut self, c: char)`, `backspace(&mut self)`, `draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str)`. Not a `Component`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn push_char_and_backspace_edit_the_query_input() {
        let mut editor = QueryEditorComponent::new();

        editor.push_char('a');
        editor.push_char('b');
        assert_eq!(editor.query_input, "ab");

        editor.backspace();
        assert_eq!(editor.query_input, "a");
    }

    #[test]
    fn backspace_on_empty_input_does_nothing() {
        let mut editor = QueryEditorComponent::new();

        editor.backspace();

        assert_eq!(editor.query_input, "");
    }

    #[test]
    fn draw_shows_the_connection_name_and_input() {
        let mut editor = QueryEditorComponent::new();
        editor.push_char('x');
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

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components::query_editor`
Expected: FAIL to compile — `QueryEditorComponent` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

```rust
//! The query input box. Not a `Component` — driven entirely by
//! `QueryScreenComponent`. `query_input` will change representation
//! (to a vim-modal `edtui` editor) in a later sub-project; this
//! sub-project keeps it an unmodified `String`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

pub struct QueryEditorComponent {
    pub query_input: String,
}

impl QueryEditorComponent {
    pub fn new() -> Self {
        Self {
            query_input: String::new(),
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.query_input.push(c);
    }

    pub fn backspace(&mut self) {
        self.query_input.pop();
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str) {
        let input = Paragraph::new(self.query_input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Query — {connection_name}")),
        );
        frame.render_widget(input, area);
    }
}
```

Add `pub mod query_editor;` to `src/components/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::query_editor`
Expected: PASS, all 3 tests.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`.

- [ ] **Step 6: Commit**

```bash
git add src/components/
git commit -m "Add QueryEditorComponent, ported from App's query_input"
```

---

### Task 5: `ConnectionPickerComponent`

**Files:**
- Create: `src/components/connection_picker.rs`
- Modify: `src/components/mod.rs` (add `pub mod connection_picker;`)

**Interfaces:**
- Consumes: `crate::action::{Action, Component}`, `crate::storage::SavedConnection`, `crossterm::event::{KeyCode, KeyModifiers}`.
- Produces (used by Task 7): `pub struct ConnectionPickerComponent { pub connections: Vec<SavedConnection>, pub selected: usize, pub last_error: Option<String> }` implementing `Component`. `new(connections: Vec<SavedConnection>) -> Self`. `handle_key_event` maps `q`→`Action::Quit`, `Down`/`j`→moves selection (no action), `Up`/`k`→moves selection (no action), `Enter`→`Some(Action::ConnectRequested(connection))` (cloning the selected connection) or `None` if the list is empty. `update` handles `Action::ConnectFailed(error)` by setting `last_error`, ignores everything else, always returns `None`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;
    use crate::storage::DriverKind;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                name: "local-sqlite".to_string(),
                driver: DriverKind::Sqlite,
                target: "test.db".to_string(),
            },
            SavedConnection {
                name: "local-postgres".to_string(),
                driver: DriverKind::Postgres,
                target: "postgres://localhost/test".to_string(),
            },
        ]
    }

    #[test]
    fn starts_with_nothing_selected() {
        let picker = ConnectionPickerComponent::new(connections());
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn move_selection_down_advances_and_stops_at_the_last_connection() {
        let mut picker = ConnectionPickerComponent::new(connections());

        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected, 1);

        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected, 1, "should stop at the last connection, not wrap");
    }

    #[test]
    fn move_selection_up_retreats_and_stops_at_zero() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0);

        picker.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.selected, 0, "should stop at zero, not go negative");
    }

    #[test]
    fn q_returns_quit() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let action = picker.handle_key_event(KeyCode::Char('q'), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn enter_returns_connect_requested_for_the_selected_connection() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        let action = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match action {
            Some(Action::ConnectRequested(connection)) => {
                assert_eq!(connection.name, "local-postgres");
            }
            other => panic!("expected ConnectRequested, got a different action or none: {}",
                if other.is_some() { "Some(_)" } else { "None" }),
        }
    }

    #[test]
    fn connect_failed_sets_the_last_error() {
        let mut picker = ConnectionPickerComponent::new(connections());

        let next = picker.update(Action::ConnectFailed("connection refused".to_string()));

        assert_eq!(picker.last_error.as_deref(), Some("connection refused"));
        assert!(next.is_none());
    }

    #[test]
    fn draw_lists_saved_connection_names() {
        let mut picker = ConnectionPickerComponent::new(connections());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_a_connection_error() {
        let mut picker = ConnectionPickerComponent::new(connections());
        picker.update(Action::ConnectFailed("connection refused".to_string()));
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("connection refused"), "buffer was: {text}");
    }
}
```

Note: `frame.area()` is read before the closure borrows `frame` mutably for `.draw(frame, ...)` in the same statement — the exact same pattern the current `tui::draw` tests already use is a single `terminal.draw(|frame| draw(frame, &app))` call; here `picker.draw(frame, frame.area())` evaluates `frame.area()` (an immutable read) before the mutable borrow needed to pass `frame` itself, which is legal Rust (argument evaluation order), and mirrors how `main.rs`'s real loop will call it (see Task 8).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components::connection_picker`
Expected: FAIL to compile — `ConnectionPickerComponent` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

```rust
//! The connection-picker screen: list saved connections, select one,
//! request a connect. Implements `Component` because `RootComponent`
//! routes keys to it directly whenever it's the active screen.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::action::{Action, Component};
use crate::storage::SavedConnection;

pub struct ConnectionPickerComponent {
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub last_error: Option<String>,
}

impl ConnectionPickerComponent {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected: 0,
            last_error: None,
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected + 1 < self.connections.len() {
            self.selected += 1;
        }
    }

    fn move_selection_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

impl Component for ConnectionPickerComponent {
    fn handle_key_event(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection_down();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection_up();
                None
            }
            KeyCode::Enter => self
                .connections
                .get(self.selected)
                .cloned()
                .map(Action::ConnectRequested),
            _ => None,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if let Action::ConnectFailed(error) = action {
            self.last_error = Some(error);
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .connections
            .iter()
            .enumerate()
            .map(|(i, connection)| {
                let item = ListItem::new(connection.name.clone());
                if i == self.selected {
                    item.style(Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    item
                }
            })
            .collect();

        let list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Connections"));

        let Some(error) = &self.last_error else {
            frame.render_widget(list, area);
            return;
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        frame.render_widget(list, chunks[0]);

        let error_box = Paragraph::new(error.as_str())
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(error_box, chunks[1]);
    }
}
```

Add `pub mod connection_picker;` to `src/components/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::connection_picker`
Expected: PASS, all 8 tests.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`.

- [ ] **Step 6: Commit**

```bash
git add src/components/
git commit -m "Add ConnectionPickerComponent, ported from App's connection-picker state"
```

---

### Task 6: `QueryScreenComponent`

**Files:**
- Create: `src/components/query_screen.rs`
- Modify: `src/components/mod.rs` (add `pub mod query_screen;`)

**Interfaces:**
- Consumes: `crate::action::{Action, Component}`, `SchemaSidebarComponent` (Task 3), `QueryEditorComponent` (Task 4), `ResultsComponent` (Task 2), `crate::query_engine::QueryEngine`, `crate::storage::SavedConnection`, `tokio::sync::mpsc::UnboundedSender`.
- Produces (used by Task 7): `pub struct QueryScreenComponent` implementing `Component`, with `pub focus: Focus`, `pub active_connection: Option<SavedConnection>`, `pub schema_sidebar: SchemaSidebarComponent`, `pub query_editor: QueryEditorComponent`, `pub results: ResultsComponent`, `pub engine: Option<QueryEngine>`. `new(action_tx: UnboundedSender<Action>) -> Self`. Also defines and exports `pub enum Focus { Editor, Sidebar }` (moved here from the old `App`, since focus is entirely a query-screen concern).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use super::*;
    use crate::drivers::{Driver, QueryResult, SchemaInfo};
    use crate::storage::DriverKind;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn connection() -> SavedConnection {
        SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo { name: "users".to_string() },
            SchemaInfo { name: "orders".to_string() },
        ]
    }

    struct FakeDriver {
        result: QueryResult,
    }

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(self.result.clone())
        }
    }

    fn fake_engine(result: QueryResult) -> QueryEngine {
        QueryEngine::new(Box::new(FakeDriver { result }))
    }

    fn screen() -> (QueryScreenComponent, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (QueryScreenComponent::new(tx), rx)
    }

    #[test]
    fn toggle_focus_flips_between_editor_and_sidebar() {
        let (mut screen, _rx) = screen();
        assert_eq!(screen.focus, Focus::Editor);

        screen.update(Action::ToggleFocus);
        assert_eq!(screen.focus, Focus::Sidebar);

        screen.update(Action::ToggleFocus);
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[test]
    fn insert_schema_selection_appends_the_selected_name_and_returns_focus_to_editor() {
        let (mut screen, _rx) = screen();
        screen.schema_sidebar.set_schema(schema());
        screen.schema_sidebar.move_down();
        screen.focus = Focus::Sidebar;
        screen.query_editor.push_char('x');

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.query_input, "xorders");
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[test]
    fn insert_schema_selection_is_a_no_op_when_schema_is_empty() {
        let (mut screen, _rx) = screen();
        screen.focus = Focus::Sidebar;

        screen.update(Action::InsertSchemaSelection);

        assert_eq!(screen.query_editor.query_input, "");
        assert_eq!(screen.focus, Focus::Sidebar, "no-op must not change focus either");
    }

    #[test]
    fn connected_stores_the_connection_engine_and_schema() {
        let (mut screen, _rx) = screen();

        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table { columns: vec![], rows: vec![] }),
            schema: Ok(schema()),
        });

        assert_eq!(screen.active_connection, Some(connection()));
        assert!(screen.engine.is_some());
        assert_eq!(screen.schema_sidebar.schema, schema());
    }

    #[test]
    fn connected_with_a_schema_error_sets_the_sidebar_error() {
        let (mut screen, _rx) = screen();

        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table { columns: vec![], rows: vec![] }),
            schema: Err("scan failed".to_string()),
        });

        assert_eq!(screen.schema_sidebar.schema_error.as_deref(), Some("scan failed"));
    }

    #[test]
    fn back_to_picker_clears_connection_engine_schema_and_focus() {
        let (mut screen, _rx) = screen();
        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table { columns: vec![], rows: vec![] }),
            schema: Ok(schema()),
        });
        screen.focus = Focus::Sidebar;

        screen.update(Action::BackToPicker);

        assert_eq!(screen.active_connection, None);
        assert!(screen.engine.is_none());
        assert_eq!(screen.schema_sidebar.schema, Vec::new());
        assert_eq!(screen.focus, Focus::Editor);
    }

    #[tokio::test]
    async fn submit_query_runs_the_query_and_reports_query_completed() {
        let (mut screen, mut rx) = screen();
        screen.update(Action::Connected {
            connection: connection(),
            engine: fake_engine(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            }),
            schema: Ok(Vec::new()),
        });
        screen.query_editor.push_char('x');

        screen.update(Action::SubmitQuery);
        assert!(screen.engine.is_none(), "engine is taken while the query runs");

        let action = rx.recv().await.expect("expected a completion action");
        match action {
            Action::QueryCompleted { result, .. } => {
                assert_eq!(
                    result,
                    QueryResult::Table {
                        columns: vec!["id".to_string()],
                        rows: vec![vec!["1".to_string()]],
                    }
                );
            }
            _ => panic!("expected QueryCompleted"),
        }
    }

    #[test]
    fn query_completed_puts_the_engine_back_and_sets_the_result() {
        let (mut screen, _rx) = screen();

        screen.update(Action::QueryCompleted {
            engine: fake_engine(QueryResult::Table { columns: vec![], rows: vec![] }),
            result: QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            },
        });

        assert!(screen.engine.is_some());
        assert_eq!(
            screen.results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }

    #[test]
    fn query_failed_puts_the_engine_back_and_sets_the_error() {
        let (mut screen, _rx) = screen();

        screen.update(Action::QueryFailed {
            engine: fake_engine(QueryResult::Table { columns: vec![], rows: vec![] }),
            error: "syntax error".to_string(),
        });

        assert!(screen.engine.is_some());
        assert_eq!(screen.results.last_error.as_deref(), Some("syntax error"));
    }

    fn sidebar_focused_screen_with_schema() -> QueryScreenComponent {
        let (mut screen, _rx) = screen();
        screen.schema_sidebar.set_schema(vec![SchemaInfo { name: "users".to_string() }]);
        screen.focus = Focus::Sidebar;
        screen
    }

    #[test]
    fn ctrl_enter_runs_the_query_instead_of_inserting_the_schema_selection_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.push_char('x');

        let action = screen.handle_key_event(KeyCode::Enter, KeyModifiers::CONTROL);

        assert!(matches!(action, Some(Action::SubmitQuery)));
        assert_eq!(screen.query_editor.query_input, "x");
    }

    #[test]
    fn f5_runs_the_query_instead_of_being_swallowed_by_the_sidebar_guard() {
        let mut screen = sidebar_focused_screen_with_schema();
        screen.query_editor.push_char('x');

        let action = screen.handle_key_event(KeyCode::F(5), KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::SubmitQuery)));
        assert_eq!(screen.query_editor.query_input, "x");
    }

    #[test]
    fn plain_enter_still_returns_insert_schema_selection_when_sidebar_focused() {
        let mut screen = sidebar_focused_screen_with_schema();

        let action = screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::InsertSchemaSelection)));
    }

    #[test]
    fn ctrl_enter_submits() {
        assert!(is_submit(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    #[test]
    fn plain_enter_does_not_submit() {
        assert!(!is_submit(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn f5_submits_regardless_of_modifiers() {
        assert!(is_submit(KeyCode::F(5), KeyModifiers::NONE));
    }

    #[test]
    fn plain_characters_do_not_submit() {
        assert!(!is_submit(KeyCode::Char('a'), KeyModifiers::NONE));
    }

    #[test]
    fn esc_returns_back_to_picker() {
        let (mut screen, _rx) = screen();

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    #[test]
    fn draw_shows_active_connection_and_input() {
        let (mut screen, _rx) = screen();
        screen.active_connection = Some(connection());
        screen.query_editor.push_char('x');
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| screen.draw(frame, frame.area())).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components::query_screen`
Expected: FAIL to compile — `QueryScreenComponent`, `Focus`, and `is_submit` don't exist in this module yet.

- [ ] **Step 3: Write the minimal implementation**

```rust
//! The post-connect screen: schema sidebar + query editor + results,
//! composed. Implements `Component` because `RootComponent` routes keys
//! to it directly whenever it's the active screen. Owns the `QueryEngine`
//! and is the only place besides `main.rs` that spawns async work — safe
//! because it only touches the `Driver` trait via `QueryEngine`, never a
//! concrete driver module.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc::UnboundedSender;

use crate::action::{Action, Component};
use crate::components::query_editor::QueryEditorComponent;
use crate::components::results::ResultsComponent;
use crate::components::schema_sidebar::SchemaSidebarComponent;
use crate::query_engine::QueryEngine;
use crate::storage::SavedConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Sidebar,
}

pub struct QueryScreenComponent {
    pub focus: Focus,
    pub active_connection: Option<SavedConnection>,
    pub schema_sidebar: SchemaSidebarComponent,
    pub query_editor: QueryEditorComponent,
    pub results: ResultsComponent,
    pub engine: Option<QueryEngine>,
    action_tx: UnboundedSender<Action>,
}

fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::F(5))
        || (code == KeyCode::Enter && modifiers.contains(KeyModifiers::CONTROL))
}

impl QueryScreenComponent {
    pub fn new(action_tx: UnboundedSender<Action>) -> Self {
        Self {
            focus: Focus::Editor,
            active_connection: None,
            schema_sidebar: SchemaSidebarComponent::new(),
            query_editor: QueryEditorComponent::new(),
            results: ResultsComponent::new(),
            engine: None,
            action_tx,
        }
    }
}

impl Component for QueryScreenComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match code {
            KeyCode::Esc => Some(Action::BackToPicker),
            KeyCode::Tab => Some(Action::ToggleFocus),
            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.active_connection.clone().map(|connection| Action::ExportCurl {
                    connection,
                    query: self.query_editor.query_input.clone(),
                })
            }
            _ if is_submit(code, modifiers) => Some(Action::SubmitQuery),
            _ if self.focus == Focus::Sidebar => match code {
                KeyCode::Down | KeyCode::Char('j') => Some(Action::SchemaMoveDown),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::SchemaMoveUp),
                KeyCode::Enter => Some(Action::InsertSchemaSelection),
                _ => None,
            },
            KeyCode::Enter => {
                self.query_editor.push_char('\n');
                None
            }
            KeyCode::Backspace => {
                self.query_editor.backspace();
                None
            }
            KeyCode::Char(c) => {
                self.query_editor.push_char(c);
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Connected { connection, engine, schema } => {
                self.active_connection = Some(connection);
                self.engine = Some(engine);
                match schema {
                    Ok(schema) => self.schema_sidebar.set_schema(schema),
                    Err(e) => self.schema_sidebar.set_schema_error(e),
                }
                None
            }
            Action::BackToPicker => {
                self.active_connection = None;
                self.engine = None;
                self.schema_sidebar.reset();
                self.focus = Focus::Editor;
                None
            }
            Action::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Sidebar,
                    Focus::Sidebar => Focus::Editor,
                };
                None
            }
            Action::SchemaMoveDown => {
                self.schema_sidebar.move_down();
                None
            }
            Action::SchemaMoveUp => {
                self.schema_sidebar.move_up();
                None
            }
            Action::InsertSchemaSelection => {
                if let Some(name) = self.schema_sidebar.selected_name() {
                    let name = name.to_string();
                    self.query_editor.query_input.push_str(&name);
                    self.focus = Focus::Editor;
                }
                None
            }
            Action::SubmitQuery => {
                let Some(engine) = self.engine.take() else {
                    return None;
                };
                let query = self.query_editor.query_input.clone();
                let tx = self.action_tx.clone();
                tokio::spawn(async move {
                    let mut engine = engine;
                    match engine.run(&query).await {
                        Ok(result) => {
                            let _ = tx.send(Action::QueryCompleted { engine, result });
                        }
                        Err(e) => {
                            let _ = tx.send(Action::QueryFailed {
                                engine,
                                error: e.to_string(),
                            });
                        }
                    }
                });
                None
            }
            Action::QueryCompleted { engine, result } => {
                self.engine = Some(engine);
                self.results.set_result(result);
                None
            }
            Action::QueryFailed { engine, error } => {
                self.engine = Some(engine);
                self.results.set_error(error);
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let outer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(1)])
            .split(area);

        self.schema_sidebar.draw(frame, outer[0], self.focus == Focus::Sidebar);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(outer[1]);

        let connection_name = self
            .active_connection
            .as_ref()
            .map(|c| c.name.as_str())
            .unwrap_or("");
        self.query_editor.draw(frame, chunks[0], connection_name);
        self.results.draw(frame, chunks[1]);
    }
}
```

Add `pub mod query_screen;` to `src/components/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::query_screen`
Expected: PASS, all 20 tests.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`.

- [ ] **Step 6: Commit**

```bash
git add src/components/
git commit -m "Add QueryScreenComponent, composing schema sidebar, editor, and results"
```

---

### Task 7: `RootComponent`, delete `App`/`tui`

**Files:**
- Create/modify: `src/components/mod.rs` (add `RootComponent`, `Screen` enum, alongside the existing `pub mod` lines)
- Delete: `src/app/mod.rs`, `src/tui/mod.rs`
- Modify: `src/lib.rs` (remove `pub mod app;` and `pub mod tui;`)

**Interfaces:**
- Consumes: `ConnectionPickerComponent` (Task 5), `QueryScreenComponent` (Task 6), `crate::action::{Action, Component}`, `crate::storage::SavedConnection`, `tokio::sync::mpsc::UnboundedSender`.
- Produces (used by Task 8): `pub enum Screen { ConnectionPicker, Query }`, `pub struct RootComponent { pub screen: Screen, pub connection_picker: ConnectionPickerComponent, pub query_screen: QueryScreenComponent, pub should_quit: bool }` implementing `Component`. `new(connections: Vec<SavedConnection>, action_tx: UnboundedSender<Action>) -> Self`.

- [ ] **Step 1: Write the failing tests**

Add to `src/components/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use super::*;
    use crate::drivers::{Driver, QueryResult, SchemaInfo};
    use crate::query_engine::QueryEngine;
    use crate::storage::DriverKind;

    struct FakeDriver;

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
            Ok(Vec::new())
        }
        async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
            Ok(QueryResult::Table { columns: vec![], rows: vec![] })
        }
    }

    fn connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                name: "local-sqlite".to_string(),
                driver: DriverKind::Sqlite,
                target: "test.db".to_string(),
            },
            SavedConnection {
                name: "local-postgres".to_string(),
                driver: DriverKind::Postgres,
                target: "postgres://localhost/test".to_string(),
            },
        ]
    }

    fn root() -> (RootComponent, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (RootComponent::new(connections(), tx), rx)
    }

    #[test]
    fn starts_on_the_connection_picker_with_nothing_selected() {
        let (root, _rx) = root();

        assert_eq!(root.screen, Screen::ConnectionPicker);
        assert_eq!(root.connection_picker.selected, 0);
    }

    #[test]
    fn quit_sets_should_quit() {
        let (mut root, _rx) = root();

        assert!(!root.should_quit);
        root.update(Action::Quit);

        assert!(root.should_quit);
    }

    #[test]
    fn connected_switches_to_the_query_screen() {
        let (mut root, _rx) = root();
        let connection = connections()[1].clone();

        root.update(Action::Connected {
            connection: connection.clone(),
            engine: QueryEngine::new(Box::new(FakeDriver)),
            schema: Ok(Vec::new()),
        });

        assert_eq!(root.screen, Screen::Query);
        assert_eq!(root.query_screen.active_connection, Some(connection));
    }

    #[test]
    fn back_to_picker_returns_to_the_connection_picker() {
        let (mut root, _rx) = root();
        root.update(Action::Connected {
            connection: connections()[0].clone(),
            engine: QueryEngine::new(Box::new(FakeDriver)),
            schema: Ok(Vec::new()),
        });

        root.update(Action::BackToPicker);

        assert_eq!(root.screen, Screen::ConnectionPicker);
        assert_eq!(root.query_screen.active_connection, None);
    }

    #[test]
    fn connect_failed_while_on_the_picker_sets_its_error() {
        let (mut root, _rx) = root();

        root.update(Action::ConnectFailed("connection refused".to_string()));

        assert_eq!(
            root.connection_picker.last_error.as_deref(),
            Some("connection refused")
        );
    }

    #[test]
    fn handle_key_event_routes_to_the_active_screen() {
        let (mut root, _rx) = root();

        let action = root.handle_key_event(crossterm::event::KeyCode::Char('q'), crossterm::event::KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::Quit)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib components:: `
Expected: FAIL to compile — `RootComponent` and `Screen` don't exist yet.

- [ ] **Step 3: Write the minimal implementation**

Add to the top of `src/components/mod.rs` (above the existing `pub mod` lines, which stay as-is):

```rust
//! The component tree: `RootComponent` switches between the connection
//! picker and the query screen, routing keys and actions to whichever is
//! active. This module — like every file under `components/` — must
//! never depend on a concrete driver module; only `main.rs` may.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::action::{Action, Component};
use crate::components::connection_picker::ConnectionPickerComponent;
use crate::components::query_screen::QueryScreenComponent;
use crate::storage::SavedConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    ConnectionPicker,
    Query,
}

pub struct RootComponent {
    pub screen: Screen,
    pub connection_picker: ConnectionPickerComponent,
    pub query_screen: QueryScreenComponent,
    pub should_quit: bool,
}

impl RootComponent {
    pub fn new(connections: Vec<SavedConnection>, action_tx: UnboundedSender<Action>) -> Self {
        Self {
            screen: Screen::ConnectionPicker,
            connection_picker: ConnectionPickerComponent::new(connections),
            query_screen: QueryScreenComponent::new(action_tx),
            should_quit: false,
        }
    }
}

impl Component for RootComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.handle_key_event(code, modifiers),
            Screen::Query => self.query_screen.handle_key_event(code, modifiers),
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return None;
        }
        if matches!(action, Action::Connected { .. }) {
            self.query_screen.update(action);
            self.screen = Screen::Query;
            return None;
        }
        if matches!(action, Action::BackToPicker) {
            self.query_screen.update(action);
            self.screen = Screen::ConnectionPicker;
            return None;
        }
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.update(action),
            Screen::Query => self.query_screen.update(action),
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        match self.screen {
            Screen::ConnectionPicker => self.connection_picker.draw(frame, area),
            Screen::Query => self.query_screen.draw(frame, area),
        }
    }
}

pub mod connection_picker;
pub mod query_editor;
pub mod query_screen;
pub mod results;
pub mod schema_sidebar;
```

(The five `pub mod` lines already exist from Tasks 2-6 in whatever order they were added — this step just confirms all five are present; don't duplicate any that are already there.)

Delete `src/app/mod.rs` and `src/tui/mod.rs`:

```bash
rm src/app/mod.rs src/tui/mod.rs
rmdir src/app src/tui
```

In `src/lib.rs`, remove the `pub mod app;` and `pub mod tui;` lines, leaving:

```rust
pub mod action;
pub mod components;
pub mod config;
pub mod drivers;
pub mod query_engine;
pub mod storage;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib components::`
Expected: PASS, all 6 new tests. (`cargo build --lib` will also fail at this point if `main.rs` still references `tradar::app`/`tradar::tui` — that's expected and fixed in Task 8, which is the next task; do not attempt to fix `main.rs` in this task.)

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --lib --tests` (not `--all-targets` — the `main.rs` binary target will not compile until Task 8, so scope this task's lint/fmt check to the library only) and `cargo fmt --check -- src/action.rs src/components/**/*.rs src/lib.rs` (or simply `cargo fmt` across the whole tree, then `git diff` to confirm only files touched by this task changed).

- [ ] **Step 6: Commit**

```bash
git add -A src/components/ src/lib.rs
git rm -r src/app src/tui
git commit -m "Add RootComponent; delete App and tui, now fully absorbed into components/"
```

**Note for the implementer:** this task intentionally leaves `main.rs` broken (it still imports the now-deleted `tradar::app`/`tradar::tui`). This is expected — Task 8 is next and fixes it. Do not skip ahead and do not leave this task un-committed waiting for Task 8; the plan's task boundaries are drawn here because `src/lib.rs`'s module list and the component tree are a complete, independently-reviewable unit even though the binary doesn't build yet.

---

### Task 8: Rewire `main.rs`

**Files:**
- Modify: `main.rs` (full rewrite of `run`, `handle_key`, `connect_to_selected`, `run_query`, `export_curl`, and the test module)

**Interfaces:**
- Consumes: `tradar::action::Action`, `tradar::components::RootComponent`, `tradar::drivers::Driver`, `tradar::drivers::{elasticsearch, elasticsearch::ElasticsearchDriver, mongo::MongoDriver, postgres::PostgresDriver, redis::RedisDriver, sqlite::SqliteDriver}`, `tradar::query_engine::QueryEngine`, `tradar::storage::{ConnectionStore, DriverKind, SavedConnection, default_connections_path}`, `tokio::sync::mpsc`.
- Produces: nothing (this is the final task; `main.rs` is the binary entry point, not a library other tasks build on).

- [ ] **Step 1: Rewrite `main.rs`**

```rust
use std::io;

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use tradar::action::{Action, Component};
use tradar::components::RootComponent;
use tradar::drivers::Driver;
use tradar::drivers::elasticsearch::{self, ElasticsearchDriver};
use tradar::drivers::mongo::MongoDriver;
use tradar::drivers::postgres::PostgresDriver;
use tradar::drivers::redis::RedisDriver;
use tradar::drivers::sqlite::SqliteDriver;
use tradar::storage::{ConnectionStore, DriverKind, default_connections_path};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let connections_path = default_connections_path()?;
    let store = ConnectionStore::at(connections_path.clone());
    let connections = store.load()?;

    if connections.is_empty() {
        println!(
            "No saved connections found. Add one to {} and re-run tradar.\n\
             (There's no interactive \"add connection\" screen yet -- see \
             docs/superpowers/specs/2026-08-01-tradar-v1-design.md.)",
            connections_path.display()
        );
        return Ok(());
    }

    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let mut root = RootComponent::new(connections, action_tx.clone());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let keyboard_enhancement = supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhancement {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut root, action_tx, action_rx).await;

    if keyboard_enhancement {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    root: &mut RootComponent,
    action_tx: mpsc::UnboundedSender<Action>,
    mut action_rx: mpsc::UnboundedReceiver<Action>,
) -> anyhow::Result<()> {
    while !root.should_quit {
        terminal.draw(|frame| root.draw(frame, frame.area()))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if let Some(action) = root.handle_key_event(key.code, key.modifiers) {
                let _ = action_tx.send(action);
            }
        }

        while let Ok(action) = action_rx.try_recv() {
            match action {
                Action::ConnectRequested(connection) => {
                    spawn_connect(action_tx.clone(), connection);
                }
                Action::ExportCurl { connection, query } => {
                    export_curl(&connection, &query);
                }
                other => {
                    if let Some(next) = root.update(other) {
                        let _ = action_tx.send(next);
                    }
                }
            }
        }
    }
    Ok(())
}

fn spawn_connect(action_tx: mpsc::UnboundedSender<Action>, connection: SavedConnection) {
    tokio::spawn(async move {
        let mut driver: Box<dyn Driver> = match connection.driver {
            DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
            DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
            DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
            DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
            DriverKind::Mongo => Box::new(MongoDriver::new(&connection.target)),
        };
        match driver.connect().await {
            Ok(()) => {
                let engine = QueryEngine::new(driver);
                let schema = engine.list_schema().await.map_err(|e| e.to_string());
                let _ = action_tx.send(Action::Connected {
                    connection,
                    engine,
                    schema,
                });
            }
            Err(e) => {
                let _ = action_tx.send(Action::ConnectFailed(e.to_string()));
            }
        }
    });
}

fn export_curl(connection: &SavedConnection, query: &str) {
    if connection.driver != DriverKind::Elasticsearch {
        return;
    }
    let Some(curl) = elasticsearch::to_curl(&connection.target, query) else {
        return;
    };
    let script = format!("#!/usr/bin/env bash\n{curl}\n");
    let _ = std::fs::write("./tradar-query.sh", script);
}
```

Note the added `use tradar::query_engine::QueryEngine;` needed by `spawn_connect` — add it to the import block above (it was omitted from the earlier `Action`/`components` design discussion, but `main.rs` needs it directly to construct the `QueryEngine` after `driver.connect()` succeeds, exactly as `connect_to_selected` did before this migration).

- [ ] **Step 2: Delete the old test module and confirm no tests remain to port here**

`main.rs`'s previous test module (`is_submit` x4 plus the three sidebar-focus-key-ordering tests) is fully removed — all seven were ported to `src/components/query_screen.rs` in Task 6, since `is_submit` and the key-arm-ordering logic now live there, not in `main.rs`. `main.rs` after this task contains no `#[cfg(test)] mod tests` at all — this is expected and matches the project's existing boundary that `main.rs` doesn't unit-test its own async orchestration (`spawn_connect`, the event loop) any more than `connect_to_selected`/`handle_key` were unit-tested before this migration; that orchestration is covered by the driver modules' own integration tests (unchanged) plus the manual `tmux` pass in Step 5 below.

- [ ] **Step 3: Build and run the full non-Docker test suite**

Run: `cargo build`
Expected: succeeds with no warnings.

Run: `cargo test --lib --bins -- --skip drivers::postgres --skip drivers::mongo --skip drivers::elasticsearch`
Expected: PASS — every test ported across Tasks 2-7 passes, and there are zero tests left in the `main.rs` binary target (expected, per Step 2).

- [ ] **Step 4: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check` (run `cargo fmt` and re-check if it reports diffs). Both must be clean — this is the first point in the plan where `--all-targets` (which includes the `main.rs` binary) can run clean, since Task 7 deliberately left the binary broken.

- [ ] **Step 5: Manual verification via tmux**

This migration's entire point is "no user-visible behavior change" — verify the full flow live, per this project's established practice (`crossterm` needs a real pty):

```bash
tmux new-session -d -s component-migration-check -x 100 -y 30
tmux send-keys -t component-migration-check 'cargo run' Enter
```

Wait for the connection picker to render (`tmux capture-pane -t component-migration-check -p`), then confirm via further `capture-pane` checks after each step:
- Select a connection with `Down`/`Enter` — confirm it reaches the Query screen with the schema sidebar, editor, and results panel all rendering (or a connection-refused error box on the picker screen, if nothing is listening — either is fine, just confirm it's not blank/frozen).
- `Tab` — confirm the sidebar title changes to "Schema [focused]".
- Type a query, `Ctrl+Enter` or `F5` — confirm it runs and results/errors render.
- `Esc` — confirm it returns to the connection picker.
- `q` on the picker — confirm the process exits.

Kill the session when done: `tmux kill-session -t component-migration-check`.

- [ ] **Step 6: Commit**

```bash
git add main.rs
git commit -m "Rewire main.rs onto the Action/Component event loop"
```

---

### Task 9: Update documentation

**Files:**
- Modify: `docs/architecture.md`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: nothing (docs only).
- Produces: nothing (docs only).

- [ ] **Step 1: Update `docs/architecture.md`**

Find the module-layout description that names `app`, `tui`, and the isolation rule referencing them (e.g. "Code in `app`, `tui`, and `query_engine` depends only on the `Driver` trait — never on `drivers::postgres`, `drivers::sqlite`, or any other concrete driver module."). Replace every reference to `app`/`tui` as separate modules with a description of the new architecture: `action.rs` (the `Action` enum and `Component` trait), `components/` (`RootComponent`, `ConnectionPickerComponent`, `QueryScreenComponent` composing `SchemaSidebarComponent`/`QueryEditorComponent`/`ResultsComponent`), and `main.rs` as the sole place a concrete driver is constructed (`Action::ConnectRequested`) or a concrete driver helper is called (`Action::ExportCurl`). State the isolation rule in terms of `components/` and `action.rs` instead of `app`/`tui`.

- [ ] **Step 2: Update `CLAUDE.md`**

Find the "Architecture" section's isolation-rule paragraph (currently: "code in `app`, `tui`, and `query_engine` depends only on the `Driver` trait, never on a concrete driver module"). Update it to name `components/` and `action.rs` instead of `app`/`tui`, matching the wording landed in `docs/architecture.md`.

- [ ] **Step 3: Commit**

```bash
git add docs/architecture.md CLAUDE.md
git commit -m "Document the Component/Action architecture in architecture.md and CLAUDE.md"
```
