# Schema Browsing in the TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the already-implemented `Driver::list_schema()` into a persistent left-hand sidebar on the Query screen, so users can see and insert table/collection/index/key names instead of typing them blind.

**Architecture:** `QueryEngine` gains a thin `list_schema()` delegate to the active `Driver`. `App` gains schema list state, a selection index, an error slot, and a `Focus` enum (`Editor`/`Sidebar`). `main.rs` calls `list_schema()` right after a successful connect and routes `Tab`/navigation keys to the sidebar when it has focus. `tui` splits the Query screen horizontally to render the sidebar alongside the existing input/results layout.

**Tech Stack:** Rust (edition 2024), `ratatui`/`crossterm` for the TUI, `tokio` async runtime — no new dependencies.

## Global Constraints

- Follow this project's TDD convention: write the failing test first, run it to confirm the failure, then write the minimal implementation, per `superpowers:test-driven-development`.
- Isolation rule (from `docs/architecture.md`): code in `app`, `tui`, and `query_engine` must depend only on the `Driver` trait and its associated types (`SchemaInfo`, `QueryResult`) — never on a concrete driver module (`drivers::postgres`, `drivers::mongo`, etc.).
- Every driver-integration test (Postgres/Mongo/Elasticsearch use `testcontainers-modules` and require Docker) must keep passing, but this plan's tasks touch no driver code — verify with `cargo test --lib --bins -- --skip drivers::postgres --skip drivers::mongo --skip drivers::elasticsearch`, which needs no Docker daemon.
- `cargo clippy --all-targets` and `cargo fmt --check` must be clean before each task's commit.
- Update documentation whenever behavior changes (this plan's last task exists specifically for that).

---

### Task 1: `QueryEngine.list_schema()` delegate

**Files:**
- Modify: `src/drivers/mod.rs`
- Modify: `src/query_engine/mod.rs`

**Interfaces:**
- Consumes: `Driver::list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>` (already exists on the trait, implemented by all five drivers).
- Produces: `SchemaInfo` now derives `Debug, Clone, PartialEq`. `QueryEngine::list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>`, used by Task 3.

- [ ] **Step 1: Add derives to `SchemaInfo`**

In `src/drivers/mod.rs`, change:

```rust
pub struct SchemaInfo {
    pub name: String,
    // extended per-database (columns, indexes, etc.) in a later plan
}
```

to:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaInfo {
    pub name: String,
    // extended per-database (columns, indexes, etc.) in a later plan
}
```

- [ ] **Step 2: Write the failing test**

In `src/query_engine/mod.rs`, the test module currently defines:

```rust
struct FakeDriver {
    result: QueryResult,
}
```

Change it to carry a schema fixture, and update `list_schema` to return it instead of a hardcoded empty vec:

```rust
struct FakeDriver {
    result: QueryResult,
    schema: Vec<SchemaInfo>,
}

#[async_trait]
impl Driver for FakeDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        Ok(self.schema.clone())
    }

    async fn execute(&self, _query: &str) -> anyhow::Result<QueryResult> {
        Ok(self.result.clone())
    }
}
```

Update the two existing test literals that construct `FakeDriver` (in `run_delegates_to_the_active_driver` and `run_appends_the_query_to_history`) to add `schema: Vec::new()`.

Add a new test:

```rust
#[tokio::test]
async fn list_schema_delegates_to_the_active_driver() {
    let driver = FakeDriver {
        result: QueryResult::Table {
            columns: Vec::new(),
            rows: Vec::new(),
        },
        schema: vec![SchemaInfo {
            name: "users".to_string(),
        }],
    };
    let engine = QueryEngine::new(Box::new(driver));

    let schema = engine.list_schema().await.unwrap();

    assert_eq!(
        schema,
        vec![SchemaInfo {
            name: "users".to_string()
        }]
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib query_engine -- --skip drivers`
Expected: FAIL to compile — `QueryEngine` has no method `list_schema`.

- [ ] **Step 4: Write the minimal implementation**

In `src/query_engine/mod.rs`, update the import and add the method:

```rust
use crate::drivers::{Driver, QueryResult, SchemaInfo};
```

```rust
impl QueryEngine {
    // ... existing new() and run() unchanged ...

    pub async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        self.driver.list_schema().await
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib query_engine`
Expected: PASS, all `query_engine` tests including the new one.

- [ ] **Step 6: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`
Expected: both clean. If `cargo fmt` reports diffs, run `cargo fmt` and re-check.

- [ ] **Step 7: Commit**

```bash
git add src/drivers/mod.rs src/query_engine/mod.rs
git commit -m "Add QueryEngine::list_schema, delegating to the active driver"
```

---

### Task 2: `App` schema and focus state

**Files:**
- Modify: `src/app/mod.rs`

**Interfaces:**
- Consumes: `SchemaInfo` (from Task 1, now `Debug + Clone + PartialEq`).
- Produces: `pub enum Focus { Editor, Sidebar }` (`Debug, Clone, Copy, PartialEq, Eq`); `App` fields `schema: Vec<SchemaInfo>`, `schema_selected: usize`, `schema_error: Option<String>`, `focus: Focus`; methods `set_schema(&mut self, Vec<SchemaInfo>)`, `set_schema_error(&mut self, String)`, `schema_move_up(&mut self)`, `schema_move_down(&mut self)`, `toggle_focus(&mut self)`, `insert_schema_selection(&mut self)`. `back_to_picker` now also resets all four. Used by Task 3 (`main.rs`) and Task 4 (`tui`).

- [ ] **Step 1: Write the failing tests**

In `src/app/mod.rs`, change the top import to:

```rust
use crate::drivers::{QueryResult, SchemaInfo};
```

In the `tests` module, add a fixture next to `connections()`:

```rust
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
```

Add these tests (after the existing `set_error_keeps_the_query_input_so_it_can_be_fixed` test):

```rust
#[test]
fn set_schema_replaces_the_schema_and_resets_selection_and_error() {
    let mut app = App::new(connections());
    app.set_schema_error("boom".to_string());
    app.schema_selected = 1;

    app.set_schema(schema());

    assert_eq!(app.schema, schema());
    assert_eq!(app.schema_selected, 0);
    assert!(app.schema_error.is_none());
}

#[test]
fn schema_move_down_advances_and_stops_at_the_last_item() {
    let mut app = App::new(connections());
    app.set_schema(schema());

    app.schema_move_down();
    assert_eq!(app.schema_selected, 1);

    app.schema_move_down();
    assert_eq!(
        app.schema_selected, 1,
        "should stop at the last item, not wrap"
    );
}

#[test]
fn schema_move_up_retreats_and_stops_at_zero() {
    let mut app = App::new(connections());
    app.set_schema(schema());
    app.schema_move_down();

    app.schema_move_up();
    assert_eq!(app.schema_selected, 0);

    app.schema_move_up();
    assert_eq!(app.schema_selected, 0, "should stop at zero, not go negative");
}

#[test]
fn toggle_focus_flips_between_editor_and_sidebar() {
    let mut app = App::new(connections());
    assert_eq!(app.focus, Focus::Editor);

    app.toggle_focus();
    assert_eq!(app.focus, Focus::Sidebar);

    app.toggle_focus();
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn insert_schema_selection_appends_the_selected_name_and_returns_focus_to_editor() {
    let mut app = App::new(connections());
    app.set_schema(schema());
    app.schema_move_down();
    app.toggle_focus();
    app.push_char('x');

    app.insert_schema_selection();

    assert_eq!(app.query_input, "xorders");
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn insert_schema_selection_is_a_no_op_when_schema_is_empty() {
    let mut app = App::new(connections());
    app.toggle_focus();

    app.insert_schema_selection();

    assert_eq!(app.query_input, "");
    assert_eq!(
        app.focus,
        Focus::Sidebar,
        "no-op must not change focus either"
    );
}

#[test]
fn back_to_picker_clears_schema_state() {
    let mut app = App::new(connections());
    app.connect_to_selected();
    app.set_schema(schema());
    app.set_schema_error("boom".to_string());
    app.toggle_focus();

    app.back_to_picker();

    assert_eq!(app.schema, Vec::new());
    assert_eq!(app.schema_selected, 0);
    assert!(app.schema_error.is_none());
    assert_eq!(app.focus, Focus::Editor);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib app::`
Expected: FAIL to compile — `Focus` doesn't exist, `App` has no `schema`/`schema_selected`/`schema_error`/`focus` fields or the new methods.

- [ ] **Step 3: Write the minimal implementation**

Add the `Focus` enum above `App` in `src/app/mod.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Sidebar,
}
```

Add fields to the `App` struct:

```rust
pub struct App {
    pub screen: Screen,
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub active_connection: Option<SavedConnection>,
    pub query_input: String,
    pub should_quit: bool,
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub schema: Vec<SchemaInfo>,
    pub schema_selected: usize,
    pub schema_error: Option<String>,
    pub focus: Focus,
}
```

Initialize them in `App::new`:

```rust
impl App {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            screen: Screen::ConnectionPicker,
            connections,
            selected: 0,
            active_connection: None,
            query_input: String::new(),
            should_quit: false,
            last_result: None,
            last_error: None,
            schema: Vec::new(),
            schema_selected: 0,
            schema_error: None,
            focus: Focus::Editor,
        }
    }

    // ... existing methods unchanged ...

    pub fn set_schema(&mut self, schema: Vec<SchemaInfo>) {
        self.schema = schema;
        self.schema_selected = 0;
        self.schema_error = None;
    }

    pub fn set_schema_error(&mut self, error: String) {
        self.schema_error = Some(error);
    }

    pub fn schema_move_down(&mut self) {
        if self.schema_selected + 1 < self.schema.len() {
            self.schema_selected += 1;
        }
    }

    pub fn schema_move_up(&mut self) {
        self.schema_selected = self.schema_selected.saturating_sub(1);
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Editor => Focus::Sidebar,
            Focus::Sidebar => Focus::Editor,
        };
    }

    pub fn insert_schema_selection(&mut self) {
        let Some(item) = self.schema.get(self.schema_selected) else {
            return;
        };
        self.query_input.push_str(&item.name);
        self.focus = Focus::Editor;
    }
}
```

Update `back_to_picker` to reset the new state:

```rust
pub fn back_to_picker(&mut self) {
    self.active_connection = None;
    self.screen = Screen::ConnectionPicker;
    self.schema = Vec::new();
    self.schema_selected = 0;
    self.schema_error = None;
    self.focus = Focus::Editor;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib app::`
Expected: PASS, all `app` tests including the seven new ones.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`
Expected: both clean.

- [ ] **Step 6: Commit**

```bash
git add src/app/mod.rs
git commit -m "Add schema list, selection, and focus state to App"
```

---

### Task 3: Wire schema loading and sidebar navigation into `main.rs`

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `QueryEngine::list_schema()` (Task 1); `App::{set_schema, set_schema_error, schema_move_up, schema_move_down, toggle_focus, insert_schema_selection}` and `Focus` (Task 2).
- Produces: no new public interface — this task only changes runtime behavior. Task 4 does not depend on this task (it renders whatever state `App` holds, set directly in its own tests).

- [ ] **Step 1: Load schema after a successful connect**

In `src/main.rs`, change the import line:

```rust
use tradar::app::{App, Screen};
```

to:

```rust
use tradar::app::{App, Focus, Screen};
```

Change `connect_to_selected` from:

```rust
async fn connect_to_selected(app: &mut App, engine: &mut Option<QueryEngine>) {
    let Some(connection) = app.connections.get(app.selected).cloned() else {
        return;
    };
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
        DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
        DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
        DriverKind::Mongo => Box::new(MongoDriver::new(&connection.target)),
    };
    match driver.connect().await {
        Ok(()) => {
            app.connect_to_selected();
            *engine = Some(QueryEngine::new(driver));
        }
        Err(e) => app.set_error(e.to_string()),
    }
}
```

to:

```rust
async fn connect_to_selected(app: &mut App, engine: &mut Option<QueryEngine>) {
    let Some(connection) = app.connections.get(app.selected).cloned() else {
        return;
    };
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
        DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
        DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
        DriverKind::Mongo => Box::new(MongoDriver::new(&connection.target)),
    };
    match driver.connect().await {
        Ok(()) => {
            app.connect_to_selected();
            let new_engine = QueryEngine::new(driver);
            match new_engine.list_schema().await {
                Ok(schema) => app.set_schema(schema),
                Err(e) => app.set_schema_error(e.to_string()),
            }
            *engine = Some(new_engine);
        }
        Err(e) => app.set_error(e.to_string()),
    }
}
```

- [ ] **Step 2: Route `Tab` and sidebar navigation keys**

In `handle_key`, change the `Screen::Query` arm from:

```rust
Screen::Query => match code {
    KeyCode::Esc => {
        app.back_to_picker();
        *engine = None;
    }
    KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => export_curl(app),
    _ if is_submit(code, modifiers) => run_query(app, engine).await,
    KeyCode::Enter => app.push_char('\n'),
    KeyCode::Backspace => app.backspace(),
    KeyCode::Char(c) => app.push_char(c),
    _ => {}
},
```

to:

```rust
Screen::Query => match code {
    KeyCode::Esc => {
        app.back_to_picker();
        *engine = None;
    }
    KeyCode::Tab => app.toggle_focus(),
    _ if app.focus == Focus::Sidebar => match code {
        KeyCode::Down | KeyCode::Char('j') => app.schema_move_down(),
        KeyCode::Up | KeyCode::Char('k') => app.schema_move_up(),
        KeyCode::Enter => app.insert_schema_selection(),
        _ => {}
    },
    KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => export_curl(app),
    _ if is_submit(code, modifiers) => run_query(app, engine).await,
    KeyCode::Enter => app.push_char('\n'),
    KeyCode::Backspace => app.backspace(),
    KeyCode::Char(c) => app.push_char(c),
    _ => {}
},
```

- [ ] **Step 3: Run the full non-Docker test suite to confirm no regressions**

Run: `cargo test --lib --bins -- --skip drivers::postgres --skip drivers::mongo --skip drivers::elasticsearch`
Expected: PASS — the existing `is_submit` tests in `main.rs` are unaffected by this change (they test a pure helper function, not `handle_key`), and Task 1/2's tests still pass.

There is no new unit test for `handle_key`'s routing itself in this task — consistent with this file's existing test boundary, where only pure helpers (`is_submit`) are unit-tested. This will be verified manually via `tmux` at the end of Task 4, once the sidebar is visible.

- [ ] **Step 4: Build check**

Run: `cargo build`
Expected: succeeds with no warnings.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`
Expected: both clean.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "Load schema on connect and route sidebar navigation keys"
```

---

### Task 4: Render the schema sidebar

**Files:**
- Modify: `src/tui/mod.rs`

**Interfaces:**
- Consumes: `App::{schema, schema_selected, schema_error, focus}` and `Focus` (Task 2).
- Produces: no new public interface — this is the final rendering layer.

- [ ] **Step 1: Write the failing tests**

In `src/tui/mod.rs`, update the test module's import:

```rust
use crate::app::{App, Focus};
```

(replacing the existing `use crate::app::App;`).

Widen the `TestBackend` in the four existing Query-screen tests from `TestBackend::new(40, 10)` to `TestBackend::new(64, 10)` — the new sidebar takes a fixed 24 columns, so the remaining area needs the same 40 columns the assertions were written against. This affects exactly these four tests: `query_screen_shows_active_connection_and_input`, `query_screen_shows_the_last_result`, `query_screen_shows_documents_pretty_printed`, `query_screen_shows_the_last_error`. Do not change `connection_picker_lists_saved_connection_names` or `connection_picker_shows_a_connection_error` — the Connection Picker screen is untouched by this plan and keeps its `TestBackend::new(40, 10)`.

Add three new tests after `query_screen_shows_the_last_error`:

```rust
#[test]
fn query_screen_shows_schema_items_in_the_sidebar() {
    let mut app = App::new(vec![SavedConnection {
        name: "local-sqlite".to_string(),
        driver: DriverKind::Sqlite,
        target: "test.db".to_string(),
    }]);
    app.connect_to_selected();
    app.set_schema(vec![crate::drivers::SchemaInfo {
        name: "users".to_string(),
    }]);
    let backend = TestBackend::new(64, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| draw(frame, &app)).unwrap();

    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("users"), "buffer was: {text}");
}

#[test]
fn query_screen_marks_the_sidebar_as_focused_in_its_title() {
    let mut app = App::new(vec![SavedConnection {
        name: "local-sqlite".to_string(),
        driver: DriverKind::Sqlite,
        target: "test.db".to_string(),
    }]);
    app.connect_to_selected();
    app.toggle_focus();
    let backend = TestBackend::new(64, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| draw(frame, &app)).unwrap();

    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Schema [focused]"), "buffer was: {text}");
}

#[test]
fn query_screen_shows_a_schema_error_in_the_sidebar() {
    let mut app = App::new(vec![SavedConnection {
        name: "local-sqlite".to_string(),
        driver: DriverKind::Sqlite,
        target: "test.db".to_string(),
    }]);
    app.connect_to_selected();
    app.set_schema_error("scan failed".to_string());
    let backend = TestBackend::new(64, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| draw(frame, &app)).unwrap();

    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("scan failed"), "buffer was: {text}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib tui::`
Expected: FAIL — the three new tests fail because there is no sidebar yet (`text` won't contain "users"/"Schema [focused]"/"scan failed" the way the test expects, or the four widened existing tests still pass trivially since the layout hasn't changed yet). The three new tests are the meaningful RED signal here.

- [ ] **Step 3: Write the minimal implementation**

Update the imports at the top of `src/tui/mod.rs`:

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{App, Focus, Screen};
use crate::drivers::QueryResult;
```

Replace `draw_query_screen` with a version that splits horizontally first, and add the new `draw_schema_sidebar` function:

```rust
fn draw_query_screen(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(1)])
        .split(frame.area());

    draw_schema_sidebar(frame, app, outer[0]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(outer[1]);

    let connection_name = app
        .active_connection
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("");
    let input = Paragraph::new(app.query_input.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Query — {connection_name}")),
    );
    frame.render_widget(input, chunks[0]);

    let body_text = if let Some(error) = &app.last_error {
        error.clone()
    } else if let Some(result) = &app.last_result {
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
    let body =
        Paragraph::new(body_text).block(Block::default().borders(Borders::ALL).title("Results"));
    frame.render_widget(body, chunks[1]);
}

fn draw_schema_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .schema
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let item = ListItem::new(entry.name.clone());
            if i == app.schema_selected {
                item.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect();

    let title = if app.focus == Focus::Sidebar {
        "Schema [focused]"
    } else {
        "Schema"
    };

    let Some(error) = &app.schema_error else {
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        frame.render_widget(list, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, chunks[0]);

    let error_box =
        Paragraph::new(error.as_str()).block(Block::default().borders(Borders::ALL).title("Error"));
    frame.render_widget(error_box, chunks[1]);
}
```

`draw_connection_picker` is unchanged.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib tui::`
Expected: PASS, all `tui` tests including the three new ones and the four widened ones.

- [ ] **Step 5: Run the full non-Docker test suite**

Run: `cargo test --lib --bins -- --skip drivers::postgres --skip drivers::mongo --skip drivers::elasticsearch`
Expected: PASS, entire crate.

- [ ] **Step 6: Lint and format**

Run: `cargo clippy --all-targets` and `cargo fmt --check`
Expected: both clean.

- [ ] **Step 7: Manual verification via tmux**

Since `crossterm` needs a real pty, verify the sidebar and its keybindings live, per this project's established practice:

```bash
tmux new-session -d -s schema-sidebar-check -x 100 -y 30
tmux send-keys -t schema-sidebar-check 'cargo run' Enter
```

Wait for the connection picker to render (`tmux capture-pane -t schema-sidebar-check -p`), select a connection with `Enter` (`tmux send-keys -t schema-sidebar-check Enter`), then confirm via `tmux capture-pane -p` that:
- The left sidebar renders with a "Schema" title and (for a reachable database) item names, or a schema error box if unreachable.
- `tmux send-keys -t schema-sidebar-check Tab` then a capture shows the title change to "Schema [focused]".
- `tmux send-keys -t schema-sidebar-check Down` moves the highlighted item (if there's more than one).
- `tmux send-keys -t schema-sidebar-check Enter` inserts the selected name into the query input and the title reverts to "Schema" (focus back on the editor).

Kill the session when done: `tmux kill-session -t schema-sidebar-check`.

- [ ] **Step 8: Commit**

```bash
git add src/tui/mod.rs
git commit -m "Render a schema sidebar on the query screen"
```

---

### Task 5: Update documentation

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/architecture.md`

**Interfaces:**
- Consumes: nothing (docs only).
- Produces: nothing (docs only).

- [ ] **Step 1: Update `README.md`**

In `README.md`, find this sentence (in the pre-alpha status paragraph):

```
Schema browsing, multi-tab editing, and general export (beyond Elasticsearch's curl export) are not built yet.
```

Replace it with:

```
The query screen has a schema sidebar (`Tab` to focus it, `↑`/`↓` or `j`/`k` to move, `Enter` to insert the selected table/collection/index/key name into the query) that loads automatically on connect. Multi-tab editing and general export (beyond Elasticsearch's curl export) are not built yet.
```

- [ ] **Step 2: Update `CLAUDE.md`**

In `CLAUDE.md`, find this sentence in the "Project state" section:

```
Schema browsing (`SchemaInfo`/`list_schema` exist on `Driver` but aren't wired into the TUI), multi-tab editing, syntax highlighting, and export are not built yet.
```

Replace it with:

```
Schema browsing is wired into the TUI as a sidebar on the query screen (loads on connect, `Tab` to focus, `Enter` to insert a name into the query). Multi-tab editing, syntax highlighting, and general export (beyond Elasticsearch's curl export) are not built yet.
```

- [ ] **Step 3: Update `docs/architecture.md`**

In `docs/architecture.md`, find this line in "Notably thin/missing pieces":

```
- `Driver::list_schema` is implemented and tested for all five drivers, but nothing in the TUI calls it yet — there's no schema explorer pane.
```

Replace it with:

```
- `Driver::list_schema` is implemented and tested for all five drivers, and wired into the TUI as a schema sidebar on the query screen (loads automatically on connect; `Tab` to focus it, `Enter` to insert the selected name into the query).
```

- [ ] **Step 4: Commit**

```bash
git add README.md CLAUDE.md docs/architecture.md
git commit -m "Document the schema sidebar in README, CLAUDE.md, and architecture.md"
```
