# NoSQL Drivers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `ElasticsearchDriver`, `RedisDriver`, and `MongoDriver` to tradar, redesigning `QueryResult` as an enum so document-shaped results (not just SQL tables) can be rendered, and upgrading the query editor to multi-line input with a Ctrl+Enter/F5 run keybinding.

**Architecture:** Every new driver is a self-contained module under `src/drivers/` implementing the existing `Driver` trait — no changes to the trait's method signatures, only to the `QueryResult` type they return. `app`, `tui`, and `query_engine` keep depending only on `Driver`/`QueryResult`, never on a concrete driver module, per the existing isolation rule in `docs/architecture.md`.

**Tech Stack:** Rust edition 2024, `tokio`, `reqwest` (Elasticsearch — already a dependency), `redis` 1.5.0 with the `tokio-comp` feature (new), `mongodb` 3.8.0 with the `bson-3` feature (new), `futures-util` 0.3 (new, for draining Mongo cursors), `testcontainers-modules` 0.11.6's `elastic_search`, `mongo`, and `redis` modules (all confirmed to exist at the pinned version — no `GenericImage` fallback needed).

## Global Constraints

- Every production-code task uses TDD (RED: write the test, confirm it fails to compile or fails the assertion; GREEN: write the minimal implementation) — the one established exception is `main.rs`'s event-loop/glue code (key dispatch wiring, file writes), which is not unit-tested, consistent with how `connect_to_selected`/`run_query` are handled today.
- Isolation rule (from `docs/architecture.md`): code under `drivers/*` depends on nothing else in the app; code in `app`, `tui`, `query_engine` depends only on the `Driver` trait, never on `drivers::elasticsearch`, `drivers::redis`, or `drivers::mongo` directly.
- Real backends via `testcontainers-modules`, not mocks, matching the existing Postgres driver's test style. If Docker isn't available, these new drivers' tests should be skippable the same way Postgres's are: `cargo test --lib -- --skip drivers::postgres --skip drivers::elasticsearch --skip drivers::redis --skip drivers::mongo`.
- No comments unless the WHY is non-obvious. Match existing code style (see `src/drivers/sqlite/mod.rs` and `src/drivers/postgres/mod.rs` for the established shape: a driver struct holding connection state, a `new()` constructor, an `impl Driver` block, and a `stringify`/`shape`-style helper below it).
- **Deviation from the spec, made during planning:** the curl-export keybinding is `Ctrl+Y`, not bare `y`. The spec's literal text says `y`, but the query screen already treats every unmodified `Char` key as text input — a bare `y` binding would make it impossible to type the letter "y" into an Elasticsearch request body. `Ctrl+Y` follows the same modifier convention already established for `Ctrl+Enter` and steals no printable character.
- **Decisions made for the spec's open questions:** Elasticsearch's `connect()` ping hits the cluster root (`GET {base_url}/`), not `/_cluster/health` — the root always responds once the node is up, with no query-string tuning needed. MongoDB's BSON→JSON conversion uses `mongodb::bson::Bson::into_relaxed_extjson()` (confirmed present on the `bson-3` feature, which pins `mongodb`'s re-exported `bson` to crate version 3.1.0). `testcontainers-modules` 0.11.6 has dedicated modules for all three new backends (`elastic_search`, `mongo`, `redis`), so the "fall back to `GenericImage`" contingency in the spec is not needed.

---

### Task 1: `QueryResult` enum redesign

**Files:**
- Modify: `src/drivers/mod.rs`
- Modify: `src/drivers/sqlite/mod.rs`
- Modify: `src/drivers/postgres/mod.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/query_engine/mod.rs`
- Modify: `src/tui/mod.rs`

**Interfaces:**
- Produces: `pub enum QueryResult { Table { columns: Vec<String>, rows: Vec<Vec<String>> }, Documents(Vec<serde_json::Value>) }` with `#[derive(Debug, Clone, PartialEq)]` — every later task constructs `QueryResult::Table { .. }` or `QueryResult::Documents(vec![..])` and can `assert_eq!` against a whole value.

This task is a mechanical breaking refactor: change the type, then fix every call site until the crate compiles and tests pass. There's no new behavior to design, so RED here means "confirm the compiler rejects the old shape," not "write a new test for new behavior."

- [ ] **Step 1: Change `QueryResult` from a struct to an enum**

Edit `src/drivers/mod.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

(replaces the existing `pub struct QueryResult { pub columns: Vec<String>, pub rows: Vec<Vec<String>> }`)

- [ ] **Step 2: Run `cargo build` and confirm it fails**

Run: `cargo build`
Expected: FAIL — multiple "expected struct `QueryResult`, found enum" / "missing field" errors in `drivers/sqlite`, `drivers/postgres`, `app`, `query_engine`, `tui`. This is the RED state for a type-level refactor.

- [ ] **Step 3: Fix `src/drivers/sqlite/mod.rs`**

In `execute()`, change the return:

```rust
        Ok(QueryResult::Table { columns, rows })
```

In the test `execute_returns_columns_and_rows_for_a_select`, change the assertions:

```rust
        let result = driver.execute("SELECT id, name FROM users").await.unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string(), "name".to_string()],
                rows: vec![vec!["1".to_string(), "Ada".to_string()]],
            }
        );
```

- [ ] **Step 4: Fix `src/drivers/postgres/mod.rs`**

Same two changes as sqlite: `Ok(QueryResult::Table { columns, rows })` in `execute()`, and the matching test assertion updated to `QueryResult::Table { .. }`.

- [ ] **Step 5: Fix `src/query_engine/mod.rs`**

The `FakeDriver` currently reconstructs the struct field-by-field; with `Clone` derived on the new enum this simplifies:

```rust
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
```

Update both tests' `QueryResult { columns, rows }` construction to `QueryResult::Table { columns, rows }`, and the assertions in `run_delegates_to_the_active_driver`:

```rust
        let result = engine.run("SELECT id FROM users").await.unwrap();

        assert_eq!(
            result,
            QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            }
        );
```

- [ ] **Step 6: Fix `src/app/mod.rs`**

Update the three tests that construct `QueryResult { .. }` (`set_result_replaces_any_previous_error`, `set_result_clears_the_query_input`, `set_error_replaces_any_previous_result`) to use `QueryResult::Table { .. }`. For example:

```rust
    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut app = App::new(connections());
        app.set_error("boom".to_string());

        app.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
        });

        assert!(app.last_error.is_none());
        assert_eq!(
            app.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }
```

(This also drops the old `.columns` field access in favor of comparing the whole `Option<QueryResult>`, now that `QueryResult` derives `PartialEq`.)

- [ ] **Step 7: Fix `src/tui/mod.rs`'s renderer**

Replace the body of `draw_query_screen`'s result-formatting `if`/`else if` with a match on the enum, adding the `Documents` render path described in the spec (pretty-printed JSON, one block per document, blank-line separated):

```rust
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
```

Add `use crate::drivers::QueryResult;` to the top of `src/tui/mod.rs`.

- [ ] **Step 8: Fix `src/tui/mod.rs`'s existing test and add a `Documents` rendering test**

Update `query_screen_shows_the_last_result`'s construction to `QueryResult::Table { .. }`, and add:

```rust
    #[test]
    fn query_screen_shows_documents_pretty_printed() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_result(crate::drivers::QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada"}),
        ]));
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Ada"), "buffer was: {text}");
    }
```

- [ ] **Step 9: Run the full test suite and confirm GREEN**

Run: `cargo test --lib -- --skip drivers::postgres`
Expected: PASS (Postgres's own tests are skipped here only because they need Docker; if Docker is available, drop the `--skip` and run `cargo test --lib`).

- [ ] **Step 10: Commit**

```bash
git add src/drivers/mod.rs src/drivers/sqlite/mod.rs src/drivers/postgres/mod.rs src/app/mod.rs src/query_engine/mod.rs src/tui/mod.rs
git commit -m "Redesign QueryResult as an enum with Table and Documents variants"
```

---

### Task 2: Multi-line query input and the Ctrl+Enter/F5 run keybinding

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `App::push_char(&mut self, c: char)`, `App::backspace(&mut self)` (both already handle `'\n'` — no `App` changes needed, confirmed in the spec).
- Produces: a private `is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool` function in `main.rs`, used by `handle_key`.

- [ ] **Step 1: Write the failing test for the submit-key predicate**

Add to the bottom of `src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 2: Run the tests and confirm they fail to compile**

Run: `cargo test --bin tradar is_submit`
Expected: FAIL with "cannot find function `is_submit`".

- [ ] **Step 3: Add `is_submit` and rewire the query screen's key handling**

Add near the top of `src/main.rs`, alongside the other `use` statements:

```rust
use crossterm::event::KeyModifiers;
```

Add the function (near `handle_key`):

```rust
fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::F(5)) || (code == KeyCode::Enter && modifiers.contains(KeyModifiers::CONTROL))
}
```

Change `handle_key`'s signature to take modifiers, and update its `Screen::Query` arm:

```rust
async fn handle_key(
    app: &mut App,
    engine: &mut Option<QueryEngine>,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> anyhow::Result<()> {
    match app.screen {
        Screen::ConnectionPicker => match code {
            KeyCode::Char('q') => app.quit(),
            KeyCode::Down | KeyCode::Char('j') => app.move_selection_down(),
            KeyCode::Up | KeyCode::Char('k') => app.move_selection_up(),
            KeyCode::Enter => connect_to_selected(app, engine).await,
            _ => {}
        },
        Screen::Query => match code {
            KeyCode::Esc => {
                app.back_to_picker();
                *engine = None;
            }
            _ if is_submit(code, modifiers) => run_query(app, engine).await,
            KeyCode::Enter => app.push_char('\n'),
            KeyCode::Backspace => app.backspace(),
            KeyCode::Char(c) => app.push_char(c),
            _ => {}
        },
    }
    Ok(())
}
```

Update the call site in `run()`:

```rust
            handle_key(app, engine, key.code, key.modifiers).await?;
```

- [ ] **Step 4: Enable the Kitty keyboard protocol when the terminal supports it**

Add to the imports:

```rust
use crossterm::event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::terminal::supports_keyboard_enhancement;
```

In `main()`, between `EnterAlternateScreen` and building the `Terminal`:

```rust
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

    let result = run(&mut terminal, &mut app, &mut engine).await;

    if keyboard_enhancement {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
```

This replaces the existing block from `let mut stdout = io::stdout();` through the final `result`. `F(5)` works as the submit key regardless of whether this protocol is supported — it's the safety net the spec's Ctrl+Enter risk calls for, not a fallback that needs its own detection branch.

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test --bin tradar is_submit`
Expected: PASS (4 tests).

- [ ] **Step 6: Manually verify in a real terminal**

Run `cargo run` in a `tmux` pane (crossterm needs a real pty), connect to a saved connection, type a query across two lines (plain `Enter` between them), then press `Ctrl+Enter` (or `F5` if the terminal doesn't distinguish it) and confirm the query runs. This isn't part of the automated suite — it's the same kind of one-off manual pass used earlier in the project to verify TUI behavior.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "Support multi-line query input with Ctrl+Enter/F5 to run"
```

---

### Task 3: `App::active_connection` becomes `Option<SavedConnection>`

**Files:**
- Modify: `src/app/mod.rs`
- Modify: `src/tui/mod.rs`

**Interfaces:**
- Produces: `pub active_connection: Option<SavedConnection>` on `App` (was `Option<String>`). Later tasks (curl export, driver dispatch) read `.driver` and `.target` off this field to decide behavior per connection kind.

- [ ] **Step 1: Update the failing assertions first**

In `src/app/mod.rs`, change `connect_to_selected_switches_to_the_query_screen`:

```rust
    #[test]
    fn connect_to_selected_switches_to_the_query_screen() {
        let mut app = App::new(connections());
        app.move_selection_down();

        app.connect_to_selected();

        assert_eq!(app.screen, Screen::Query);
        assert_eq!(
            app.active_connection.as_ref().map(|c| c.name.as_str()),
            Some("local-postgres")
        );
    }
```

- [ ] **Step 2: Run the tests and confirm they fail to compile**

Run: `cargo test --lib active_connection`
Expected: FAIL — `active_connection` is still `Option<String>`, so `.map(|c| c.name.as_str())` doesn't type-check.

- [ ] **Step 3: Change the field type and `connect_to_selected`**

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
}
```

```rust
    pub fn connect_to_selected(&mut self) {
        self.active_connection = self.connections.get(self.selected).cloned();
        self.screen = Screen::Query;
    }
```

- [ ] **Step 4: Fix `src/tui/mod.rs`'s connection-name lookup**

```rust
    let connection_name = app
        .active_connection
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("");
```

- [ ] **Step 5: Run the full test suite and confirm GREEN**

Run: `cargo test --lib -- --skip drivers::postgres`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app/mod.rs src/tui/mod.rs
git commit -m "Track the whole SavedConnection as App::active_connection, not just its name"
```

---

### Task 4: Elasticsearch driver (connect, list_schema, execute)

**Files:**
- Create: `src/drivers/elasticsearch/mod.rs`
- Modify: `src/drivers/mod.rs`
- Modify: `src/storage/mod.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `pub struct ElasticsearchDriver` implementing `Driver`, constructed via `ElasticsearchDriver::new(base_url: &str)`; `pub fn parse_query(query: &str) -> Option<(String, String, Option<String>)>` (method, path, body) — a pure function Task 5's `to_curl` also uses.
- Produces: `DriverKind::Elasticsearch` variant on `storage::DriverKind`.

- [ ] **Step 1: Add the `testcontainers-modules` feature**

In `Cargo.toml`, change:

```toml
testcontainers-modules = { version = "0.11", features = ["postgres"] }
```

to:

```toml
testcontainers-modules = { version = "0.11", features = ["postgres", "elastic_search"] }
```

- [ ] **Step 2: Write the failing tests**

Create `src/drivers/elasticsearch/mod.rs`:

```rust
//! Elasticsearch driver, modeled on Kibana's Dev Tools console: the query
//! input is a `METHOD /path` line plus an optional JSON body, sent to the
//! cluster as-is rather than limited to the Search API.

use async_trait::async_trait;

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct ElasticsearchDriver {
    base_url: String,
}

impl ElasticsearchDriver {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

pub fn parse_query(query: &str) -> Option<(String, String, Option<String>)> {
    let mut lines = query.lines();
    let first = lines.next()?.trim();
    let mut parts = first.splitn(2, char::is_whitespace);
    let method = parts.next()?.to_string();
    let path = parts.next()?.trim().to_string();
    if method.is_empty() || path.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim();
    let body = if body.is_empty() { None } else { Some(body.to_string()) };
    Some((method, path, body))
}

#[async_trait]
impl Driver for ElasticsearchDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let response = reqwest::get(format!("{}/", self.base_url)).await?;
        if !response.status().is_success() {
            anyhow::bail!("elasticsearch ping failed with status {}", response.status());
        }
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let url = format!("{}/_cat/indices?format=json", self.base_url);
        let indices: Vec<serde_json::Value> = reqwest::get(&url).await?.json().await?;
        Ok(indices
            .into_iter()
            .filter_map(|entry| {
                entry
                    .get("index")
                    .and_then(|v| v.as_str())
                    .map(|name| SchemaInfo { name: name.to_string() })
            })
            .collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let (method, path, body) = parse_query(query)
            .ok_or_else(|| anyhow::anyhow!("expected \"METHOD /path\" on the first line"))?;
        let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
            .map_err(|_| anyhow::anyhow!("unknown HTTP method: {method}"))?;
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));

        let client = reqwest::Client::new();
        let mut request = client.request(method, &url);
        if let Some(body) = &body {
            request = request.header("Content-Type", "application/json").body(body.clone());
        }
        let response = request.send().await?;
        let json: serde_json::Value = response.json().await?;
        Ok(QueryResult::Documents(vec![json]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[test]
    fn parse_query_splits_method_path_and_body() {
        let (method, path, body) =
            parse_query("POST my-index/_search\n{\"query\": {\"match_all\": {}}}").unwrap();

        assert_eq!(method, "POST");
        assert_eq!(path, "my-index/_search");
        assert_eq!(body.as_deref(), Some("{\"query\": {\"match_all\": {}}}"));
    }

    #[test]
    fn parse_query_allows_a_missing_body() {
        let (method, path, body) = parse_query("GET _cat/indices?v").unwrap();

        assert_eq!(method, "GET");
        assert_eq!(path, "_cat/indices?v");
        assert_eq!(body, None);
    }

    #[test]
    fn parse_query_rejects_a_missing_path() {
        assert!(parse_query("GET").is_none());
    }

    #[tokio::test]
    async fn connect_succeeds_for_a_running_cluster() {
        let container = ElasticSearch::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(9200).await.unwrap();
        let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_runs_an_arbitrary_request_and_wraps_the_response_as_documents() {
        let container = ElasticSearch::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(9200).await.unwrap();
        let mut driver = ElasticsearchDriver::new(&format!("http://127.0.0.1:{port}"));
        driver.connect().await.unwrap();

        let result = driver.execute("GET _cluster/health").await.unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs.len(), 1);
                assert!(docs[0].get("status").is_some(), "response was: {docs:?}");
            }
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
    }

    #[tokio::test]
    async fn list_schema_returns_created_indices() {
        let container = ElasticSearch::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(9200).await.unwrap();
        let base_url = format!("http://127.0.0.1:{port}");
        let mut driver = ElasticsearchDriver::new(&base_url);
        driver.connect().await.unwrap();
        reqwest::Client::new()
            .put(format!("{base_url}/test-index"))
            .send()
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema.iter().any(|entry| entry.name == "test-index"),
            "schema was: {:?}",
            schema.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }
}
```

- [ ] **Step 3: Wire the module in and confirm RED**

In `src/drivers/mod.rs`, add:

```rust
pub mod elasticsearch;
```

Run: `cargo test --lib drivers::elasticsearch`
Expected: the `parse_query` unit tests FAIL (or don't compile) until Step 2's implementation above is actually saved — since the implementation was written in the same step as the tests here, treat this as a checkpoint: run it now and confirm it's GREEN already, since there's no separate stub phase for a brand-new module. (Unlike Task 1's refactor, a new module can be written test-and-implementation together in one file; the meaningful RED/GREEN boundary is the next step's compile check before vs. after the module is registered.)

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib drivers::elasticsearch`
Expected: PASS (6 tests: 3 pure `parse_query` tests, 3 container-backed tests). The container tests need Docker; first run pulls `docker.elastic.co/elasticsearch/elasticsearch:7.16.1`, which is slow the first time (same as the Postgres image pull noted during v1 development).

- [ ] **Step 5: Add `DriverKind::Elasticsearch`**

In `src/storage/mod.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Sqlite,
    Elasticsearch,
}
```

- [ ] **Step 6: Wire it into `main.rs`**

Add the import:

```rust
use tradar::drivers::elasticsearch::ElasticsearchDriver;
```

Add a match arm in `connect_to_selected`:

```rust
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
        DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
    };
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo build && cargo test --lib -- --skip drivers::postgres --skip drivers::elasticsearch`
Expected: PASS (compiles cleanly; the skipped groups need Docker and were already verified in Step 4).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/drivers/mod.rs src/drivers/elasticsearch/mod.rs src/storage/mod.rs src/main.rs
git commit -m "Add ElasticsearchDriver: Kibana-console-style connect/list_schema/execute"
```

---

### Task 5: Elasticsearch curl export (`Ctrl+Y`)

**Files:**
- Modify: `src/drivers/elasticsearch/mod.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `elasticsearch::parse_query` (Task 4).
- Produces: `pub fn to_curl(base_url: &str, query: &str) -> Option<String>` in `drivers::elasticsearch`.

- [ ] **Step 1: Write the failing tests**

Add to `src/drivers/elasticsearch/mod.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn to_curl_includes_the_body_when_present() {
        let curl = to_curl(
            "http://localhost:9200",
            "POST my-index/_search\n{\"query\":{\"match_all\":{}}}",
        )
        .unwrap();

        assert_eq!(
            curl,
            "curl -X POST \"http://localhost:9200/my-index/_search\" -H 'Content-Type: application/json' -d '{\"query\":{\"match_all\":{}}}'"
        );
    }

    #[test]
    fn to_curl_omits_the_body_flags_when_there_is_no_body() {
        let curl = to_curl("http://localhost:9200", "GET _cat/indices?v").unwrap();

        assert_eq!(curl, "curl -X GET \"http://localhost:9200/_cat/indices?v\"");
    }

    #[test]
    fn to_curl_returns_none_for_unparseable_queries() {
        assert!(to_curl("http://localhost:9200", "").is_none());
    }
```

- [ ] **Step 2: Run the tests and confirm they fail to compile**

Run: `cargo test --lib to_curl`
Expected: FAIL with "cannot find function `to_curl`".

- [ ] **Step 3: Implement `to_curl`**

Add to `src/drivers/elasticsearch/mod.rs`, below `parse_query`:

```rust
pub fn to_curl(base_url: &str, query: &str) -> Option<String> {
    let (method, path, body) = parse_query(query)?;
    let base_url = base_url.trim_end_matches('/');
    let url = format!("{base_url}/{}", path.trim_start_matches('/'));
    let method = method.to_uppercase();
    Some(match body {
        Some(body) => format!(
            "curl -X {method} \"{url}\" -H 'Content-Type: application/json' -d '{body}'"
        ),
        None => format!("curl -X {method} \"{url}\""),
    })
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib to_curl`
Expected: PASS (3 tests).

- [ ] **Step 5: Wire the `Ctrl+Y` keybinding into `main.rs`**

Add the import:

```rust
use tradar::drivers::elasticsearch;
```

Add a case to `Screen::Query`'s match in `handle_key` (above the `is_submit` guard so it isn't shadowed, and gated so it never fires without the Control modifier — a bare `y` still types the letter):

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

Add the function:

```rust
fn export_curl(app: &App) {
    let Some(connection) = &app.active_connection else {
        return;
    };
    if connection.driver != DriverKind::Elasticsearch {
        return;
    }
    let Some(curl) = elasticsearch::to_curl(&connection.target, &app.query_input) else {
        return;
    };
    let script = format!("#!/usr/bin/env bash\n{curl}\n");
    let _ = std::fs::write("./tradar-query.sh", script);
}
```

- [ ] **Step 6: Run the full test suite**

Run: `cargo build && cargo test --lib -- --skip drivers::postgres --skip drivers::elasticsearch`
Expected: PASS.

- [ ] **Step 7: Manually verify in `tmux`**

Connect to an Elasticsearch connection, type `GET _cluster/health`, press `Ctrl+Y`, and confirm `./tradar-query.sh` was written with the expected `curl` command. Confirm typing a literal "y" into the query box still works (i.e., pressing `y` without Control inserts the character).

- [ ] **Step 8: Commit**

```bash
git add src/drivers/elasticsearch/mod.rs src/main.rs
git commit -m "Add Ctrl+Y curl export for Elasticsearch queries"
```

---

### Task 6: Redis driver

**Files:**
- Create: `src/drivers/redis/mod.rs`
- Modify: `src/drivers/mod.rs`
- Modify: `src/storage/mod.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `pub struct RedisDriver` implementing `Driver`, via `RedisDriver::new(url: &str)`.
- Produces: `DriverKind::Redis` variant.

- [ ] **Step 1: Add dependencies**

In `Cargo.toml`, add to `[dependencies]`:

```toml
redis = { version = "1", features = ["tokio-comp"] }
```

Change the `testcontainers-modules` line in `[dev-dependencies]`:

```toml
testcontainers-modules = { version = "0.11", features = ["postgres", "elastic_search", "redis"] }
```

- [ ] **Step 2: Write the failing tests**

Create `src/drivers/redis/mod.rs`:

```rust
//! Redis driver: one command line per execution, naive whitespace parsing.
//! Most replies get a generic RESP-to-JSON conversion; HGETALL and
//! ZRANGE/ZREVRANGE ... WITHSCORES get type-aware formatting so their
//! flat arrays don't lose the field/value or member/score pairing.

use async_trait::async_trait;

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct RedisDriver {
    url: String,
    connection: Option<redis::aio::MultiplexedConnection>,
}

impl RedisDriver {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            connection: None,
        }
    }
}

#[async_trait]
impl Driver for RedisDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = redis::Client::open(self.url.as_str())?;
        self.connection = Some(client.get_multiplexed_async_connection().await?);
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let mut connection = self.connection.clone().expect("connect() must be called first");
        let (_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(0)
            .arg("COUNT")
            .arg(100)
            .query_async(&mut connection)
            .await?;
        Ok(keys.into_iter().map(|name| SchemaInfo { name }).collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let mut connection = self.connection.clone().expect("connect() must be called first");
        let parts: Vec<&str> = query.split_whitespace().collect();
        let (command, args) = parts
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;

        let mut cmd = redis::cmd(command);
        for arg in args {
            cmd.arg(*arg);
        }
        let value: redis::Value = cmd.query_async(&mut connection).await?;

        Ok(QueryResult::Documents(vec![shape_reply(command, args, &value)]))
    }
}

fn shape_reply(command: &str, args: &[&str], value: &redis::Value) -> serde_json::Value {
    match command.to_ascii_uppercase().as_str() {
        "HGETALL" => hgetall_to_object(value).unwrap_or_else(|| value_to_json(value)),
        "ZRANGE" | "ZREVRANGE" if args.iter().any(|a| a.eq_ignore_ascii_case("withscores")) => {
            zrange_withscores_to_pairs(value).unwrap_or_else(|| value_to_json(value))
        }
        _ => value_to_json(value),
    }
}

fn hgetall_to_object(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut object = serde_json::Map::new();
    for pair in items.chunks(2) {
        let [field, val] = pair else { return None };
        object.insert(value_to_string(field)?, value_to_json(val));
    }
    Some(serde_json::Value::Object(object))
}

fn zrange_withscores_to_pairs(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut pairs = Vec::new();
    for pair in items.chunks(2) {
        let [member, score] = pair else { return None };
        let mut entry = serde_json::Map::new();
        entry.insert(
            "member".to_string(),
            serde_json::Value::String(value_to_string(member)?),
        );
        entry.insert("score".to_string(), value_to_json(score));
        pairs.push(serde_json::Value::Object(entry));
    }
    Some(serde_json::Value::Array(pairs))
}

fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        redis::Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

fn value_to_json(value: &redis::Value) -> serde_json::Value {
    match value {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(i) => serde_json::Value::Number((*i).into()),
        redis::Value::BulkString(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).to_string())
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s.clone()),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
        redis::Value::Array(items) | redis::Value::Set(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        redis::Value::Map(pairs) => {
            let mut object = serde_json::Map::new();
            for (key, val) in pairs {
                if let Some(key) = value_to_string(key) {
                    object.insert(key, value_to_json(val));
                }
            }
            serde_json::Value::Object(object)
        }
        redis::Value::Double(d) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        redis::Value::Boolean(b) => serde_json::Value::Bool(*b),
        redis::Value::VerbatimString { text, .. } => serde_json::Value::String(text.clone()),
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::redis::{Redis, REDIS_PORT};
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[tokio::test]
    async fn connect_succeeds_for_a_running_redis() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_hgetall_returns_a_json_object() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("HSET user:1 name Ada age 36").await.unwrap();

        let result = driver.execute("HGETALL user:1").await.unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs[0]["name"], "Ada");
                assert_eq!(docs[0]["age"], "36");
            }
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
    }

    #[tokio::test]
    async fn execute_zrange_withscores_returns_member_score_pairs() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("ZADD leaderboard 10 alice 20 bob").await.unwrap();

        let result = driver
            .execute("ZRANGE leaderboard 0 -1 WITHSCORES")
            .await
            .unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(
                    docs[0],
                    serde_json::json!([
                        {"member": "alice", "score": "10"},
                        {"member": "bob", "score": "20"}
                    ])
                );
            }
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
    }

    #[tokio::test]
    async fn list_schema_returns_existing_keys() {
        let container = Redis::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        let mut driver = RedisDriver::new(&format!("redis://127.0.0.1:{port}"));
        driver.connect().await.unwrap();
        driver.execute("SET greeting hello").await.unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema.iter().any(|entry| entry.name == "greeting"),
            "schema was: {:?}",
            schema.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }
}
```

- [ ] **Step 3: Wire the module in**

In `src/drivers/mod.rs`, add:

```rust
pub mod redis;
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --lib drivers::redis`
Expected: PASS (4 tests; needs Docker, pulls the `redis:5.0` image on first run).

- [ ] **Step 5: Add `DriverKind::Redis`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Sqlite,
    Elasticsearch,
    Redis,
}
```

- [ ] **Step 6: Wire it into `main.rs`**

```rust
use tradar::drivers::redis::RedisDriver;
```

```rust
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
        DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
        DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
    };
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo build && cargo test --lib -- --skip drivers::postgres --skip drivers::elasticsearch --skip drivers::redis`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/drivers/mod.rs src/drivers/redis/mod.rs src/storage/mod.rs src/main.rs
git commit -m "Add RedisDriver with type-aware HGETALL/ZRANGE-WITHSCORES formatting"
```

---

### Task 7: MongoDB shell-subset parser (pure, no database)

**Files:**
- Create: `src/drivers/mongo/mod.rs`
- Modify: `src/drivers/mod.rs`

**Interfaces:**
- Produces: `pub struct ParsedQuery { pub collection: String, pub method: String, pub args: Vec<serde_json::Value> }` and `pub fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery>`, both consumed by Task 8's `execute()`.

- [ ] **Step 1: Write the failing tests**

Create `src/drivers/mongo/mod.rs`:

```rust
//! MongoDB driver: a minimal shell-subset parser for the literal shape
//! `db.<collection>.<method>(<json-args>)`, not a real JS engine. Anything
//! outside that shape — chained methods, `$where`, arbitrary expressions —
//! is rejected with a clear error rather than guessed at.

pub struct ParsedQuery {
    pub collection: String,
    pub method: String,
    pub args: Vec<serde_json::Value>,
}

pub fn parse_shell_query(query: &str) -> anyhow::Result<ParsedQuery> {
    let query = query.trim();
    let rest = query
        .strip_prefix("db.")
        .ok_or_else(|| anyhow::anyhow!("expected a query starting with \"db.<collection>.<method>(...)\""))?;

    let dot = rest
        .find('.')
        .ok_or_else(|| anyhow::anyhow!("missing collection name"))?;
    let collection = rest[..dot].to_string();
    let rest = &rest[dot + 1..];

    let paren = rest.find('(').ok_or_else(|| anyhow::anyhow!("missing method call"))?;
    let method = rest[..paren].to_string();
    let rest = rest[paren + 1..].trim_end();
    let args_text = rest
        .strip_suffix(')')
        .ok_or_else(|| anyhow::anyhow!("missing closing parenthesis"))?;

    let args = split_top_level_args(args_text)?
        .into_iter()
        .map(|arg| serde_json::from_str(arg.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("invalid JSON argument: {e}"))?;

    Ok(ParsedQuery { collection, method, args })
}

fn split_top_level_args(text: &str) -> anyhow::Result<Vec<&str>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        anyhow::bail!("unbalanced braces in arguments");
    }
    args.push(&text[start..]);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_find_with_a_filter() {
        let parsed = parse_shell_query(r#"db.users.find({"active": true})"#).unwrap();

        assert_eq!(parsed.collection, "users");
        assert_eq!(parsed.method, "find");
        assert_eq!(parsed.args, vec![serde_json::json!({"active": true})]);
    }

    #[test]
    fn parses_multiple_top_level_arguments() {
        let parsed =
            parse_shell_query(r#"db.users.updateOne({"_id": 1}, {"$set": {"name": "Ada"}})"#)
                .unwrap();

        assert_eq!(parsed.collection, "users");
        assert_eq!(parsed.method, "updateOne");
        assert_eq!(
            parsed.args,
            vec![
                serde_json::json!({"_id": 1}),
                serde_json::json!({"$set": {"name": "Ada"}})
            ]
        );
    }

    #[test]
    fn parses_a_method_call_with_no_arguments() {
        let parsed = parse_shell_query("db.users.find()").unwrap();

        assert_eq!(parsed.args, Vec::<serde_json::Value>::new());
    }

    #[test]
    fn rejects_input_that_does_not_start_with_db() {
        assert!(parse_shell_query("users.find({})").is_err());
    }

    #[test]
    fn rejects_malformed_json_arguments() {
        assert!(parse_shell_query("db.users.find({not json})").is_err());
    }
}
```

- [ ] **Step 2: Wire the module in as a placeholder and confirm it compiles**

In `src/drivers/mod.rs`, add:

```rust
pub mod mongo;
```

Run: `cargo build`
Expected: PASS (the parser has no external dependencies beyond `serde_json`, already present — this is a self-contained module, so there's no separate RED phase beyond "does it compile," which it does since the tests and implementation were written together above).

- [ ] **Step 3: Run the tests and confirm they pass**

Run: `cargo test --lib drivers::mongo::tests`
Expected: PASS (5 tests, no Docker needed).

- [ ] **Step 4: Commit**

```bash
git add src/drivers/mod.rs src/drivers/mongo/mod.rs
git commit -m "Add the MongoDB shell-subset query parser"
```

---

### Task 8: MongoDB driver (connect, list_schema, execute)

**Files:**
- Modify: `src/drivers/mongo/mod.rs`
- Modify: `src/storage/mod.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `parse_shell_query` (Task 7).
- Produces: `pub struct MongoDriver` implementing `Driver`, via `MongoDriver::new(uri: &str)`.
- Produces: `DriverKind::Mongo` variant.

- [ ] **Step 1: Add dependencies**

In `Cargo.toml`, add to `[dependencies]`:

```toml
mongodb = { version = "3", features = ["bson-3"] }
futures-util = "0.3"
```

The `bson-3` feature pins `mongodb`'s re-exported `mongodb::bson` to bson crate 3.1.0 (its default is bson 2.x) — that's the version this task's code was verified against, specifically `Bson::into_relaxed_extjson()` and `TryFrom<serde_json::Value> for Bson`.

Change the `testcontainers-modules` line in `[dev-dependencies]`:

```toml
testcontainers-modules = { version = "0.11", features = ["postgres", "elastic_search", "redis", "mongo"] }
```

- [ ] **Step 2: Write the failing tests**

Add to `src/drivers/mongo/mod.rs`, above the existing `#[cfg(test)] mod tests` block (merge into the same module — add these imports and items alongside the parser code already there):

```rust
use async_trait::async_trait;
use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document};

use crate::drivers::{Driver, QueryResult, SchemaInfo};

pub struct MongoDriver {
    uri: String,
    client: Option<mongodb::Client>,
}

impl MongoDriver {
    pub fn new(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
            client: None,
        }
    }

    fn database(&self) -> anyhow::Result<mongodb::Database> {
        let client = self.client.as_ref().expect("connect() must be called first");
        client
            .default_database()
            .ok_or_else(|| anyhow::anyhow!("connection string must include a default database"))
    }
}

#[async_trait]
impl Driver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = mongodb::Client::with_uri_str(&self.uri).await?;
        let db = client
            .default_database()
            .ok_or_else(|| anyhow::anyhow!("connection string must include a default database"))?;
        db.run_command(mongodb::bson::doc! { "ping": 1 }).await?;
        self.client = Some(client);
        Ok(())
    }

    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>> {
        let names = self.database()?.list_collection_names().await?;
        Ok(names.into_iter().map(|name| SchemaInfo { name }).collect())
    }

    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult> {
        let parsed = parse_shell_query(query)?;
        let db = self.database()?;
        let collection = db.collection::<Document>(&parsed.collection);
        run_method(&collection, &parsed.method, &parsed.args).await
    }
}

async fn run_method(
    collection: &mongodb::Collection<Document>,
    method: &str,
    args: &[serde_json::Value],
) -> anyhow::Result<QueryResult> {
    let doc_arg = |i: usize| -> anyhow::Result<Document> {
        let value = args
            .get(i)
            .ok_or_else(|| anyhow::anyhow!("{method} requires at least {} argument(s)", i + 1))?;
        json_to_document(value.clone())
    };

    match method {
        "find" => {
            let filter = if args.is_empty() { Document::new() } else { doc_arg(0)? };
            let mut cursor = collection.find(filter).await?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents(docs))
        }
        "aggregate" => {
            let pipeline = args
                .iter()
                .cloned()
                .map(json_to_document)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let mut cursor = collection.aggregate(pipeline).await?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                docs.push(Bson::Document(doc).into_relaxed_extjson());
            }
            Ok(QueryResult::Documents(docs))
        }
        "insertOne" => {
            let result = collection.insert_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "insertMany" => {
            let docs_arg = args
                .first()
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("insertMany requires an array argument"))?;
            let docs = docs_arg
                .iter()
                .cloned()
                .map(json_to_document)
                .collect::<anyhow::Result<Vec<_>>>()?;
            let result = collection.insert_many(docs).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "updateOne" => {
            let result = collection.update_one(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "updateMany" => {
            let result = collection.update_many(doc_arg(0)?, doc_arg(1)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "deleteOne" => {
            let result = collection.delete_one(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        "deleteMany" => {
            let result = collection.delete_many(doc_arg(0)?).await?;
            Ok(QueryResult::Documents(vec![serde_json::to_value(result)?]))
        }
        other => anyhow::bail!("unsupported query: db.<collection>.{other}(...) is not implemented"),
    }
}

fn json_to_document(value: serde_json::Value) -> anyhow::Result<Document> {
    let bson = Bson::try_from(value)?;
    bson.as_document()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("expected a JSON object"))
}
```

Add to the existing `#[cfg(test)] mod tests` block:

```rust
    use testcontainers_modules::mongo::Mongo;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    #[tokio::test]
    async fn connect_succeeds_for_a_running_mongo() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));

        let result = driver.connect().await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_insert_one_then_find_round_trips_a_document() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();
        let result = driver
            .execute(r#"db.users.find({"name": "Ada"})"#)
            .await
            .unwrap();

        match result {
            QueryResult::Documents(docs) => {
                assert_eq!(docs.len(), 1);
                assert_eq!(docs[0]["name"], "Ada");
            }
            QueryResult::Table { .. } => panic!("expected Documents"),
        }
    }

    #[tokio::test]
    async fn list_schema_returns_created_collections() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();
        driver
            .execute(r#"db.users.insertOne({"name": "Ada"})"#)
            .await
            .unwrap();

        let schema = driver.list_schema().await.unwrap();

        assert!(
            schema.iter().any(|entry| entry.name == "users"),
            "schema was: {:?}",
            schema.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn execute_rejects_an_unsupported_method() {
        let container = Mongo::new().start().await.unwrap();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        let mut driver = MongoDriver::new(&format!("mongodb://127.0.0.1:{port}/test"));
        driver.connect().await.unwrap();

        let result = driver.execute("db.users.drop()").await;

        assert!(result.is_err());
    }
```

- [ ] **Step 3: Run the tests and confirm they pass**

Run: `cargo test --lib drivers::mongo`
Expected: PASS (9 tests total — 5 pure parser tests from Task 7, plus 4 container-backed tests here; needs Docker, pulls the `mongo:5.0.6` image on first run).

- [ ] **Step 4: Add `DriverKind::Mongo`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Sqlite,
    Elasticsearch,
    Redis,
    Mongo,
}
```

- [ ] **Step 5: Wire it into `main.rs`**

```rust
use tradar::drivers::mongo::MongoDriver;
```

```rust
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
        DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
        DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
        DriverKind::Mongo => Box::new(MongoDriver::new(&connection.target)),
    };
```

- [ ] **Step 6: Run the full test suite**

Run: `cargo build && cargo test --lib -- --skip drivers::postgres --skip drivers::elasticsearch --skip drivers::redis --skip drivers::mongo`
Expected: PASS. If Docker is available, also run the full `cargo test --lib` (and `cargo test --bin tradar`) once to confirm every container-backed test across all four new/existing drivers passes together.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/drivers/mongo/mod.rs src/storage/mod.rs src/main.rs
git commit -m "Add MongoDriver: shell-subset CRUD dispatch over the official mongodb crate"
```

---

### Task 9: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`

- [ ] **Step 1: Update `README.md`'s Databases section**

Replace:

```markdown
## Databases

**v1 target:** PostgreSQL, SQLite

**Planned:** MySQL, MariaDB, MongoDB, Elasticsearch, Redis, ClickHouse

New database support is added as a `Driver` implementation without touching the rest of the application — see `docs/architecture.md`.
```

with:

```markdown
## Databases

**v1 target:** PostgreSQL, SQLite, MongoDB, Elasticsearch, Redis — each a `Driver` implementation with its own execution model:

- **PostgreSQL / SQLite** — real SQL, tabular results.
- **MongoDB** — a minimal shell-subset parser for `db.<collection>.<method>(<json-args>)` (`find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`); not a real JS engine.
- **Elasticsearch** — a Kibana Dev Tools-style console: type `METHOD /path` plus an optional JSON body and it's sent to the cluster as-is, not limited to the Search API.
- **Redis** — one command line per execution, naive whitespace parsing; `HGETALL` and `ZRANGE`/`ZREVRANGE ... WITHSCORES` get type-aware JSON formatting, everything else uses a generic RESP-to-JSON conversion.

**Planned:** MySQL, MariaDB, ClickHouse

New database support is added as a `Driver` implementation without touching the rest of the application — see `docs/architecture.md`.
```

- [ ] **Step 2: Update `README.md`'s Status section**

Replace the existing Status paragraph with:

```markdown
## Status

Pre-alpha, but runnable: `tradar` connects to a real PostgreSQL, SQLite, MongoDB, Elasticsearch, or Redis instance, runs queries, and shows results in the terminal — connection picker → query screen → results, all keyboard-driven. The query editor is multi-line: plain `Enter` inserts a newline, and `Ctrl+Enter` (or `F5`, since not every terminal reports Ctrl+Enter distinctly) runs the query. On an Elasticsearch connection, `Ctrl+Y` writes the current request as a `curl` command to `./tradar-query.sh` in the working directory. There's no interactive "add connection" screen yet, so saved connections must be added by hand to the TOML file at the path `tradar` prints when none exist (see `src/storage/mod.rs`). Schema browsing, multi-tab editing, and general export (beyond Elasticsearch's curl export) are not built yet. See `docs/architecture.md` for the shape of the system, the [v1 design spec](docs/superpowers/specs/2026-08-01-tradar-v1-design.md), and the [NoSQL drivers spec](docs/superpowers/specs/2026-08-01-nosql-drivers-design.md).
```

- [ ] **Step 3: Update `docs/architecture.md`'s module layout**

Replace the `drivers/` block in the module tree:

```
  drivers/
    mod.rs        — the Driver trait (connect, list_schema, execute, ...)
    postgres/
    sqlite/
    elasticsearch/
    redis/
    mongo/
```

- [ ] **Step 4: Update `docs/architecture.md`'s `Driver` trait section**

Replace the `QueryResult`/`SchemaInfo` sentence and add the enum definition:

```markdown
`SchemaInfo` and `QueryResult` are the normalized shapes the rest of the app renders — a driver is responsible for translating its database's native results into these types. `QueryResult` is an enum, not a single struct, because SQL results and document-shaped results (MongoDB, Elasticsearch, Redis) don't fit the same shape:

```rust
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

`Table` is what Postgres and SQLite return. `Documents` is shared by the other three drivers: each Elasticsearch hit/response, MongoDB document, or Redis reply becomes one `serde_json::Value` in the vec. `tui` renders `Table` as a text table and `Documents` as pretty-printed JSON blocks.
```

- [ ] **Step 5: Update `docs/architecture.md`'s "Current state" section**

Update the opening sentence to mention all five drivers:

```markdown
The v1 walking skeleton works end to end: `tradar` loads saved connections from `storage`, connects via the selected `Driver` (Postgres, SQLite, Elasticsearch, Redis, or MongoDB, all fully implemented against real backends), and runs queries typed into the `tui`'s query screen through `query_engine`, rendering real results or errors.
```

- [ ] **Step 6: Commit**

```bash
git add README.md docs/architecture.md
git commit -m "Document the NoSQL drivers, multi-line input, and curl export in README/architecture"
```

---

## Self-Review Notes

- **Spec coverage:** all 7 goals from the spec map to tasks — `QueryResult` redesign (Task 1), multi-line input + Ctrl+Enter (Task 2), Elasticsearch Kibana console (Task 4), curl export (Task 5), Redis type-aware formatting (Task 6), Mongo shell-subset parser (Tasks 7–8), and updating every existing `QueryResult` call site (Task 1). Documentation deliverables are Task 9.
- **Deviations flagged up front** in Global Constraints: `Ctrl+Y` instead of bare `y` (fixes a real typing-conflict bug in the literal spec text), ES ping endpoint (`/` root), Mongo BSON conversion path (`into_relaxed_extjson` via the `bson-3` feature) — all three were open questions in the spec and are now resolved with reasoning, not left as TODOs.
- **Type consistency checked:** `QueryResult::Table { columns, rows }` / `QueryResult::Documents(Vec<Value>)` used identically across Tasks 1, 4, 6, 8. `App::active_connection: Option<SavedConnection>` (Task 3) is read the same way in Task 5's `export_curl` and Task 2 doesn't touch it. `DriverKind` variants are added incrementally (Elasticsearch in Task 4, Redis in Task 6, Mongo in Task 8) and `main.rs`'s `connect_to_selected` match is kept exhaustive at the end of each of those tasks, so the crate compiles after every task, not just at the end of the plan.
