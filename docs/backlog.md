# Backlog

Tracks the DB-IDE feature roadmap (decomposed into sub-projects during brainstorming — see `docs/superpowers/specs/2026-08-02-component-architecture-migration-design.md`'s Context section) and known issues deferred out of past work. Update this file directly when a sub-project starts/finishes or a new issue is deferred — it's the durable record now that per-plan SDD ledgers get deleted once a plan's final review is clean.

## Sub-project roadmap

Decomposed from a single broader request ("DataGrip/DBeaver-style DB IDE features: vim keymap, save/load, sessions, UI polish"). Each gets its own spec → plan → implementation cycle.

1. **Component Architecture migration** — done, merged to `master` (2026-08-02/03). Replaced the flat `App`/`tui` design with `src/action.rs` (`Action` enum + `Component` trait) + `src/components/`. Foundation for the items below.
2. **Vim-style modal query editor** (using `edtui`) — not started. Requires bumping `ratatui` 0.29 → `^0.30` (edtui's dependency). Design decisions already made during brainstorming: `edtui::EditorState` lives outside `App`/now outside the component tree proper — as a sibling owned by whichever component hosts the editor (`QueryScreenComponent`), not baked into `Action`/`Component` itself; `Esc` is context-sensitive (Insert→Normal mode inside the editor, only bubbling to `BackToPicker` when already in Normal mode) — this touches the same `handle_key_event` arm ordering that both the schema-sidebar and Component-migration final reviews fought to keep correct, so budget a regression test for it specifically.
3. **Save/load query to file** — not started.
4. **Sessions/workspace state** — not started; scope not yet clarified (query history? multiple simultaneous tabs/connections? state persisted across runs?). Needs its own brainstorming pass to scope before designing.
5. **General UI polish** — not started; intentionally last, since it needs the other features to have real shape first.

## Known issues (deferred, not blocking)

From the Component Architecture migration's task reviews and final whole-branch review (ledger deleted after merge per SDD process — recorded here so it isn't lost):

- ~~**Connect-vs-connect race.**~~ Fixed. `ConnectionPickerComponent` now owns a `connect_epoch: u64` counter, bumped and stamped onto `Action::ConnectRequested` on every connect attempt; `main.rs`'s `spawn_connect` threads that epoch through to the `Connected`/`ConnectFailed` it sends back. `RootComponent::update` drops any `Connected`/`ConnectFailed` whose epoch doesn't match `connection_picker.connect_epoch`, so a reply from a superseded connect attempt can no longer silently switch the UI back. Same pattern as the existing query-epoch mechanism in `QueryScreenComponent`. Covered by `components::tests::a_stale_connected_from_a_superseded_connect_attempt_is_ignored` and `...a_stale_connect_failed_from_a_superseded_connect_attempt_is_ignored` in `src/components/mod.rs`.
- `ConnectionPickerComponent.last_error` is only ever set, never cleared on a successful retry (`src/components/connection_picker.rs`). Low priority — cosmetic (a stale error sits behind the query screen until the next failed connect attempt).
- No test exercises `Ctrl+Y` (curl export) while the schema sidebar has keyboard focus in `QueryScreenComponent` — only `F5`/`Ctrl+Enter` are tested against the sidebar-focus guard. Same protection mechanism (checked before the guard in match-arm order), so low risk, but worth adding for completeness.
- `main.rs`'s `event::poll(Duration::from_millis(50))` is a synchronous call that can block its tokio worker thread up to 50ms per idle tick. Bounded and not correctness-affecting on the default multi-threaded runtime, but `crossterm::event::EventStream` + `tokio::select!` would be the non-blocking alternative if this ever becomes a real problem.
- Stylistic inconsistency: `ResultsComponent`, `QueryEditorComponent`, and `SchemaSidebarComponent` each implement `Default`/`new()` slightly differently (some derive `Default` and hand-write an identical `new()`, one hand-writes `Default` delegating to `new()`). Pick one convention and apply it across all three next time one of those files is touched.
