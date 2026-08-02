# Schema Browsing in the TUI

Status: Approved
Date: 2026-08-02

## Context

`Driver::list_schema()` has been implemented and tested for all five drivers since the NoSQL drivers work landed (`docs/superpowers/specs/2026-08-01-nosql-drivers-design.md` explicitly deferred this: "schema browsing isn't wired into the TUI for any driver yet"). It returns table names (Postgres/SQLite), collection names (Mongo), index names (Elasticsearch), or key names (Redis, via `SCAN COUNT 100`) — but nothing in `app` or `tui` calls it. Users must already know and type the exact table/collection/index/key name by hand, which is a bigger gap for NoSQL databases where there's no fixed, memorized schema the way there often is for SQL.

This spec adds a schema sidebar to the Query screen so users can see and pick names instead of typing them blind.

## Goals

1. Load schema automatically right after a successful connection, for every driver.
2. Show it as a persistent left-hand sidebar on the Query screen (always visible, not a toggled overlay).
3. Selecting an item inserts its name into the query input (does not auto-run a query) and returns focus to the editor.
4. A failure to load schema must not block querying — the user can still type queries by hand.

## Non-goals (explicitly deferred)

- Any change to what `list_schema()` returns per driver (Redis's 100-key `SCAN` cap, no pagination, no richer per-item metadata like columns/types) — out of scope here.
- Auto-running a default query when an item is selected (e.g. `SELECT * FROM table`, `db.coll.find()`) — considered and rejected; selection only inserts the name.
- A toggled/overlay panel — rejected in favor of an always-visible sidebar, consistent with the k9s/lazygit-style keyboard-first browsing this project is modeled on.
- Live-refreshing the sidebar while connected (e.g. after `CREATE TABLE`) — schema is loaded once, at connect time.

## Architecture & data flow

- `QueryEngine` (`src/query_engine/mod.rs`) gains `list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>`, delegating to `self.driver.list_schema()` — mirrors the existing `run()` delegation, so `main.rs` keeps calling through `QueryEngine` and never touches a `Driver` directly.
- `App` (`src/app/mod.rs`) gains:
  - `schema: Vec<SchemaInfo>`, `schema_selected: usize`, `schema_error: Option<String>`
  - `focus: Focus`, a new enum `{ Editor, Sidebar }`, defaulting to `Editor`
  - `set_schema(Vec<SchemaInfo>)` — replaces `schema`, resets `schema_selected` to 0, clears `schema_error`
  - `set_schema_error(String)` — sets `schema_error`, leaves `schema` as-is (empty on first load)
  - `schema_move_up()` / `schema_move_down()` — bounds-checked like the existing `move_selection_up`/`move_selection_down` for the connection picker
  - `toggle_focus()` — flips `Editor` ⇄ `Sidebar`
  - `insert_schema_selection()` — appends `schema[schema_selected].name` to the end of `query_input`, then sets `focus = Editor`; no-op if `schema` is empty
  - `back_to_picker()` additionally resets `schema`, `schema_error`, and `focus` to their defaults
- `main.rs`'s `connect_to_selected()`: right after `driver.connect().await` succeeds and the `QueryEngine` is constructed, call `engine.list_schema().await`. On success, `app.set_schema(...)`; on failure, `app.set_schema_error(e.to_string())`. Either way, the app proceeds to the Query screen — this call is synchronous (awaited) before the first draw, so there's no separate loading state to render.

## Interaction & UI

- `draw_query_screen` (`src/tui/mod.rs`) splits horizontally: a fixed-width sidebar on the left (`Constraint::Length(24)`), with the existing input/results vertical layout occupying the remaining space (`Constraint::Min`) on the right.
- Sidebar is a `List` of `schema` item names; the item at `schema_selected` is highlighted with `Modifier::REVERSED`, matching the connection picker's existing selection style. The panel title reads `"Schema"` normally and `"Schema [focused]"` when `app.focus == Sidebar`, so focus state is visible without color.
- If `schema_error` is set, it's shown inline within the sidebar panel (short text, does not touch `last_error`/the Results panel — that stays reserved for query execution errors).
- New key handling on `Screen::Query`, branching on `app.focus`:
  - `Tab` → `app.toggle_focus()`, regardless of current focus.
  - `Esc` → always returns to the Connection Picker, regardless of focus (unchanged from today).
  - When `focus == Sidebar`: `Up`/`Char('k')` → `schema_move_up()`; `Down`/`Char('j')` → `schema_move_down()`; `Enter` → `insert_schema_selection()`; all other character keys are ignored (do not fall through to `push_char`).
  - When `focus == Editor`: unchanged — typing, `Backspace`, Ctrl+Enter/F5 submit, Ctrl+Y curl export all behave exactly as they do today.

## Error handling

- Schema-load failure (e.g. Redis `SCAN` error, Elasticsearch `_cat/indices` request failing) surfaces only in the sidebar via `schema_error`; it never populates `last_error` and never prevents typing or running a query by hand.
- Redis's existing 100-key `SCAN` cap is unchanged and not addressed by this spec.

## Testing

Per the project's TDD convention:

- `query_engine`: `list_schema` delegates to the driver — extend the existing `FakeDriver` test fixture.
- `app`: `set_schema` resets `schema_selected`/clears `schema_error`; `schema_move_up`/`schema_move_down` bounds-check like the connection picker's selection; `toggle_focus` flips both directions; `insert_schema_selection` appends the selected name to `query_input` and returns focus to `Editor` (including a no-selection/empty-schema no-op case); `back_to_picker` clears all schema state.
- `tui`: sidebar renders schema item names; panel title reflects `focus`.
- `main.rs`: no new unit test for `handle_key`'s routing — consistent with the current test boundary there (only pure helpers like `is_submit` are unit-tested); verified manually via `tmux`, as done for prior TUI changes in this project.
