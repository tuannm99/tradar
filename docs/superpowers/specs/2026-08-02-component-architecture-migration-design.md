# Component Architecture Migration

Status: Approved
Date: 2026-08-02

## Context

The user asked for a broad set of DB-IDE-style features (vim keymap, save/load queries to files, sessions/tabs, general UI polish — see `docs/superpowers/specs/2026-08-02-schema-browsing-tui-design.md` for the immediately preceding feature and the app's current state). That request bundles multiple independent subsystems and was decomposed into an ordered sequence of sub-projects:

1. **Component Architecture migration (this spec)**
2. Vim-style modal query editor (using `edtui`)
3. Save/load query to file
4. Sessions/workspace state
5. General UI polish

Today, `tradar` is architecturally a simplified/ad-hoc **Elm Architecture (TEA)**: `App` (in `src/app/mod.rs`) is a single flat struct holding every piece of state (connections, selection, active connection, query input, schema list, focus, last result/error), `tui::draw(frame, app)` (in `src/tui/mod.rs`) is a pure view function, and `main.rs`'s `handle_key` plus its async helper functions (`connect_to_selected`, `run_query`, `export_curl`) are the update step, mixing side effects (driver I/O) directly into synchronous-per-keystroke async functions.

This has worked well through the walking-skeleton and schema-sidebar stages, but the requested trajectory — a modal vim editor (its own state machine), then sessions/tabs (multiple simultaneous editor+connection instances) — is exactly the kind of growth that turns a flat TEA `App` struct and a single ever-growing `handle_key` match into a maintenance burden. Per [ratatui's own documented application patterns](https://ratatui.rs/concepts/application-patterns/), **Component Architecture** — each UI part owns its private state and its own event handling/rendering, composed by a thin top-level coordinator — is the recommended pattern for apps past this size. Combined with the also-documented **Action + async-channel pattern** (an `Action` enum, an `mpsc` channel, components spawning `tokio` tasks for I/O and reporting results back as actions), this gives tradar a foundation that scales to concurrent multi-session I/O without a second migration later.

This spec covers the migration itself: **no user-visible behavior change**. Every existing screen, keybinding, and rendering behavior must work identically after this lands — it is purely an internal restructuring that the vim editor (sub-project 2) and sessions (sub-project 4) will build on.

## Goals

1. Replace the flat `App` struct + `handle_key` match with a `Component` trait (`handle_key_event`, `update`, `draw`) and a shared `Action` enum, following ratatui's documented Component + Action/channel pattern.
2. Preserve the driver-isolation rule (`docs/architecture.md`): only `main.rs` may depend on a concrete driver module (`drivers::postgres`, `drivers::mongo`, etc.). No component may construct a `Box<dyn Driver>`.
3. Preserve every existing behavior exactly: connection picker navigation, connect/error display, schema sidebar (load-on-connect, Tab focus, navigation, insert-on-enter, inline error), query editor (multi-line input, Ctrl+Enter/F5 submit, Ctrl+Y curl export, Esc-context routing per the sidebar-focus fix), and results/error rendering.
4. Port every existing test to its new component file with identical assertions — this migration must not silently drop coverage.

## Non-goals (explicitly deferred)

- The vim modal editor itself (sub-project 2) — this spec only prepares the `QueryEditorComponent` shell; its internals stay a plain `String` (identical to today's `App::query_input`) until sub-project 2 replaces it with `edtui::EditorState`.
- Save/load to file, sessions/tabs, UI visual polish — sub-projects 3-5.
- Any new keybinding, screen, or driver behavior.
- Mouse event support — tradar is keyboard-only by design; the `Component` trait intentionally omits `handle_mouse_event`, unlike ratatui's full template.

## Architecture

### File structure

New `src/components/` module tree (sibling to `drivers/`, `query_engine/`, `storage/`):

- `src/action.rs` (new) — the `Action` enum and the `Component` trait.
- `src/components/mod.rs` (new) — `RootComponent`: the top-level coordinator. Holds `ConnectionPickerComponent` and `QueryScreenComponent` as named fields (both always constructed, never `Option`— matches today's behavior where `App` always exists and screen-switching just changes which one is active/rendered) plus a `Screen` enum (`ConnectionPicker`/`Query`, same two variants as today) tracking which is active. Routes `handle_key_event`/`draw` to the active one; owns no business logic beyond that routing and the two screen-transition actions (`BackToPicker`, `Connected`/`ConnectFailed`).
- `src/components/connection_picker.rs` (new) — `ConnectionPickerComponent`: connection list, selection index, last error. Direct port of the connection-picker half of today's `App` + `draw_connection_picker`.
- `src/components/schema_sidebar.rs` (new) — `SchemaSidebarComponent`: schema list, selection, error. Direct port of today's `App`'s schema fields/methods + the sidebar half of `draw_query_screen`.
- `src/components/query_editor.rs` (new) — `QueryEditorComponent`: holds `query_input: String` (unchanged representation for this sub-project), same `push_char`/`backspace` methods. Direct port of today's `App::query_input` handling + the input half of `draw_query_screen`.
- `src/components/results.rs` (new) — `ResultsComponent`: `last_result: Option<QueryResult>`, `last_error: Option<String>`. Direct port of the results half of `draw_query_screen`.
- `src/components/query_screen.rs` (new) — `QueryScreenComponent`: composes `SchemaSidebarComponent` + `QueryEditorComponent` + `ResultsComponent`, holds `Option<QueryEngine>`, `active_connection: Option<SavedConnection>`, `focus: Focus` (unchanged enum), and `action_tx: UnboundedSender<Action>` (needed to spawn `engine.run()`/`engine.list_schema()` — both isolation-safe, since they only touch the `Driver` trait, not a concrete driver). Handles `Tab` focus routing between Sidebar/Editor exactly as today's `main.rs` `handle_key` does, and the exact key-arm ordering fixed in the schema-sidebar branch's final review (Esc, Tab, Ctrl+Y, submit, sidebar-guard, then Editor keys).
- `src/app/mod.rs` and `src/tui/mod.rs` are **deleted**; their content is fully absorbed into `components/`.
- `main.rs` shrinks to: terminal setup (unchanged), the action channel (`mpsc::unbounded_channel::<Action>()`), the event-read-to-action-send loop, the action-draining loop with its one special case (`Action::ConnectRequested` → the only place in the whole binary that matches on `DriverKind` to build a `Box<dyn Driver>`, spawns the connect+list_schema task, sends `Action::Connected`/`ConnectFailed` back), and forwarding every other drained action into `RootComponent::update`.

### `Component` trait and `Action` enum

```rust
pub trait Component {
    fn handle_key_event(&mut self, key: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
```

`Action` enumerates every state transition the app currently performs implicitly inside `handle_key`/`connect_to_selected`/`run_query`/`export_curl`:

```rust
pub enum Action {
    Quit,
    MoveSelectionUp,
    MoveSelectionDown,
    ConnectRequested(SavedConnection),
    Connected { engine: QueryEngine, schema: Result<Vec<SchemaInfo>, String> },
    ConnectFailed(String),
    ToggleFocus,
    SchemaMoveUp,
    SchemaMoveDown,
    InsertSchemaSelection,
    SubmitQuery,
    QueryCompleted(QueryResult),
    QueryFailed(String),
    ExportCurl,
    BackToPicker,
}
```

`Action` lives in `src/action.rs` alongside the `Component` trait; both depend only on `crate::drivers::{QueryResult, SchemaInfo}`, `crate::query_engine::QueryEngine`, and `crate::storage::SavedConnection` — never a concrete driver module, preserving the isolation rule for this new shared type too.

### Preserving the isolation rule through the action-interception point

`ConnectionPickerComponent::update()`, on receiving the action produced by its own `handle_key_event` for `Enter`, returns `Some(Action::ConnectRequested(connection))` — it never constructs a `Box<dyn Driver>`, since `components/` must not depend on any concrete driver module.

`main.rs`'s action-draining loop intercepts `Action::ConnectRequested` before it reaches `RootComponent::update`:

```rust
while let Ok(action) = action_rx.try_recv() {
    if let Action::ConnectRequested(connection) = &action {
        let tx = action_tx.clone();
        let connection = connection.clone();
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
                    let _ = tx.send(Action::Connected { engine, schema });
                }
                Err(e) => {
                    let _ = tx.send(Action::ConnectFailed(e.to_string()));
                }
            }
        });
        continue;
    }
    if let Some(next) = root.update(action) {
        let _ = action_tx.send(next);
    }
}
```

This is the **only** place in the binary that names a concrete driver type. `RootComponent`, `QueryScreenComponent`, and every other component see only `Action::Connected`/`ConnectFailed`. `QueryScreenComponent::update()` handling `Action::SubmitQuery` follows the same spawn-and-report-back shape, but does so itself (not through `main.rs`) since it only touches `QueryEngine`/`Driver`, which is isolation-safe — it holds its own cloned `action_tx` for this purpose.

### Main loop

Follows ratatui's documented async-template shape: read a crossterm event → convert to `Action` (via the focused component's `handle_key_event`) → send on `action_tx` → drain `action_rx` with `try_recv()` in a loop (terminates naturally when the channel is empty for this tick) → dispatch each drained action (special-casing `ConnectRequested` as above) → redraw. `Action::Quit` sets a `should_quit` flag on `RootComponent` checked by the outer loop, mirroring today's `App::should_quit`/`App::quit()`.

## Testing

- `update()` and `draw()` on every component are synchronous, pure(-ish) functions — testable exactly like today's `App`/`tui` tests: construct the component, call `update`/`handle_key_event`, assert on the component's own fields and the returned `Option<Action>`; `draw` tests keep using `TestBackend` + `Terminal` + buffer-content assertions.
- `QueryScreenComponent`'s `tokio::spawn` paths (`SubmitQuery`, and re-running `list_schema` if ever needed) are tested with `#[tokio::test]` against a real `mpsc` channel, asserting the expected `Action` arrives.
- The `DriverKind` → `Box<dyn Driver>` construction in `main.rs`'s action-draining loop is **not** unit-tested, consistent with the project's existing boundary (only pure helpers in `main.rs` get unit tests; this async orchestration is covered by the driver modules' own `testcontainers`-based integration tests, unchanged by this migration, plus a manual `tmux` verification pass at the end confirming the full connect → schema-load → query → results flow is unchanged).
- Every test currently in `src/app/mod.rs` and `src/tui/mod.rs` must be **ported** (not dropped and rewritten from scratch) to its new component's test module, keeping the same assertions, so this migration is verifiably behavior-preserving rather than merely "still has some tests."
