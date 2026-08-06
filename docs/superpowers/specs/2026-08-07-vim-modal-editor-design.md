# Vim-Style Modal Query Editor

Status: Approved
Date: 2026-08-07

## Context

This is sub-project 2 of the DB-IDE feature roadmap (`docs/backlog.md`), decomposed during the Component Architecture migration's brainstorming (`docs/superpowers/specs/2026-08-02-component-architecture-migration-design.md`). That migration deliberately left `QueryEditorComponent`'s `query_input` as a plain `String` and prepared the shell this spec now replaces: "its internals stay a plain `String` ... until sub-project 2 replaces it with `edtui::EditorState`."

The backlog already recorded two design decisions from that earlier brainstorming, both carried forward here unchanged:

- `edtui::EditorState` lives outside the `Action`/`Component` machinery, as a plain field owned by `QueryEditorComponent` (the component `QueryScreenComponent` already drives directly — `QueryEditorComponent` implements no `Component` trait itself).
- `Esc` is context-sensitive: it must transition Insert→Normal inside the editor, and only bubble up to `Action::BackToPicker` once the editor is already in Normal mode. This lands in the same `handle_key_event` arm ordering that both the schema-sidebar and Component-migration final reviews had to fight to keep correct, so it gets a dedicated regression test.

`edtui` 0.11.6's core (modal editing, motions, `EditorState`/`EditorEventHandler`) depends on `ratatui-core 0.1` / `ratatui-widgets 0.3`, which only resolve against `ratatui ^0.30`. The project currently pins `ratatui = "0.29"`, so this sub-project also carries the version bump the backlog flagged as a prerequisite.

## Goals

1. Replace the query editor's plain-text input with a real vim-modal editor (Normal/Insert/Visual modes, motions, operators) via `edtui`, without reimplementing vim behavior ourselves.
2. Preserve every existing query-screen interaction exactly: `Tab` toggles sidebar/editor focus, `F5`/`Ctrl+Enter` submits, `Ctrl+y` exports curl, schema selection inserts a name and returns focus to the editor.
3. `Esc` behaves like it does in a real vim editor (Insert→Normal first) while still letting the existing "go back to the connection picker" gesture work once in Normal mode.
4. Keep the crate's dependency footprint minimal — no clipboard/mouse/syntax-highlighting extras pulled in just to get modal editing.

## Non-goals (explicitly deferred)

- SQL syntax highlighting — `edtui` bundles a `syntax-highlighting` feature (via `syntect`), but this is a distinct, not-yet-scoped feature (per `CLAUDE.md`) and adds a heavy dependency; left disabled here.
- System clipboard integration (`arboard`) and mouse support — not requested, not enabled.
- Any change to `QueryEngine`, drivers, or the results/schema-sidebar components beyond how they hand text to/from the editor.
- A fallback/toggle to the old plain-`String` editor — rejected as unnecessary (YAGNI); nothing asks for an opt-out and it would double the surface area to maintain.
- Auditing every `ratatui` 0.29→0.30 API difference up front — tracked as an implementation-time risk (see Risks), not designed around here.

## Architecture & data flow

**Dependencies (`Cargo.toml`):**
- `ratatui = "0.29"` → `ratatui = "0.30"`.
- Add `edtui = { version = "0.11", default-features = false }`. `edtui`'s core modal-editing functionality (motions, modes, buffer mutation) is not gated behind any feature flag — only clipboard (`arboard`), `mouse-support`, and `syntax-highlighting` are optional extras, and none are needed for this sub-project's goals.

**`QueryEditorComponent`** (`src/components/query_editor.rs`): `query_input: String` is replaced by:
- `state: edtui::EditorState` — the buffer, cursor, and mode.
- `event_handler: edtui::EditorEventHandler` — persists across keystrokes (tracks operator-pending sequences like `dd`).

New/changed methods:
- `text(&self) -> String` — flattens `state.lines` (a `Jagged<char>`, edtui/edtui-jagged's row-based buffer) into a single `\n`-joined `String`. Replaces every current read of `query_input` (used by `SubmitQuery` and `ExportCurl`). The exact row-join call is an edtui-jagged API detail to confirm during implementation; the flattening behavior is what's specified here.
- `insert_at_cursor(&mut self, text: &str)` — inserts each character of `text` at the current cursor via edtui's `InsertChar` action, then sets `state.mode = EditorMode::Insert`. Replaces the old `query_input.push_str(...)` used by schema insertion.
- `forward_key(&mut self, key: KeyEvent)` — `self.event_handler.on_key_event(key, &mut self.state)`. The single entry point `QueryScreenComponent` uses to hand off any key it doesn't intercept itself.
- `draw()` renders `EditorView::new(&mut self.state)` (wrapped, themed with the existing bordered/titled block) instead of a `Paragraph`.
- `push_char`/`backspace` are deleted; edtui owns all buffer mutation.

`Component::handle_key_event`'s signature (`(KeyCode, KeyModifiers) -> Option<Action>`) is unchanged. `edtui::EditorEventHandler::on_key_event` wants a full `crossterm::event::KeyEvent`; `QueryScreenComponent` reconstructs one locally (`KeyEvent::new(code, modifiers)`, kind defaults to `Press`) at the point it forwards to `forward_key`. This keeps the trait — and every other component's key handling and tests — untouched.

**`QueryScreenComponent::handle_key_event`** — same intercept-then-forward shape as today, re-ordered around editor mode:

1. `Esc`: if `query_editor.state.mode != EditorMode::Normal`, forward the key into `query_editor.forward_key(...)` (which transitions Insert→Normal) and consume it (return `None`); otherwise emit `Action::BackToPicker`, exactly as today.
2. `Tab` → `Action::ToggleFocus`, unconditionally — unchanged from today (Tab already steals focus rather than inserting a literal tab character even in the current plain-text editor, so this isn't a new regression).
3. `Ctrl+y` → `Action::ExportCurl`, unconditionally.
4. `is_submit` (`F5` / `Ctrl+Enter`) → `Action::SubmitQuery`, unconditionally — works mid-Insert, matching today.
5. If `focus == Focus::Sidebar`: unchanged (`Down`/`Up`/`Enter` → schema actions); the editor never sees keys while the sidebar has focus.
6. Otherwise (`focus == Focus::Editor`, nothing above matched): forward the raw key into `query_editor.forward_key(...)`, return `None`.

**`Action::InsertSchemaSelection`** handling in `update()`: calls `query_editor.insert_at_cursor(&name)` instead of `query_input.push_str(&name)`, then sets `focus = Focus::Editor`. Per this sub-project's scope, insertion happens at the cursor position (not always at the buffer's end), and the editor ends in Insert mode so the user can keep typing immediately.

## Error handling

No new fallible operations. `edtui`'s key handling and buffer mutation are infallible — `EditorState` methods don't return `Result`. The one existing fallible path (query execution failure, surfaced via `Action::QueryFailed`) is untouched; `text()` only changes how the query string is produced before `SubmitQuery` is dispatched.

## Testing

Per the project's TDD convention:

- Update `query_editor.rs`'s existing unit tests (currently drive `push_char`/`backspace` and assert on `query_input`) to build content via `EditorState::new(Lines::from(...))` and/or `forward_key`, asserting on `text()`. ~17 call sites across `query_editor.rs`/`query_screen.rs` reference `query_input` today and need updating to the new API — expected churn from the representation change, not a new design concern.
- New regression test: `Esc` while the editor is in Insert mode transitions to Normal mode and does **not** emit `Action::BackToPicker`; a second `Esc`, now in Normal mode, does. This is the specific arm-ordering regression the backlog flagged.
- New regression test: `Ctrl+y` (curl export) still fires while the sidebar has keyboard focus — closing the gap the backlog separately flagged as untested, since this work is already touching the same match arm.
- New test: schema selection inserts the selected name at the current cursor position (not just appended at the buffer's end) and leaves the editor in Insert mode.
- Not retested here: `edtui`'s own motions/operators/registers — that's covered by its own test suite. Tradar's tests cover only the integration seams listed above.

## Risks

- `ratatui` `0.29` → `0.30` is a version bump for a crate depended on directly throughout the codebase (`Frame`, widgets, `TestBackend` in every component's tests). Needs a changelog check during implementation to catch breaking API changes; not expected to affect the design above, but budget time for fallout.
- The exact `edtui-jagged` `Lines`/`Jagged<char>` → `String` row-join call wasn't pinned down during this design (no `Display` impl found by source inspection); implementation needs to confirm it against the crate source. The `text()` method's behavior (flatten to a `\n`-joined `String`) is fixed regardless of which exact API produces it.
