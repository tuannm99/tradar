# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Tradar's name is a portmanteau of the author's name (tuannm) + "radar" — not related to trading, despite the surface reading. The project briefly went through a rename to "Rowdy" before reverting back to `tradar`; don't propose renaming it again.

The repo is a Cargo workspace (ten crates: `tradar-core`, `tradar-connector-api`, `tradar-query-workbench`, five connector crates under `crates/connectors/`, and the `tradar-app` binary — see `docs/architecture.md`'s "Triển khai hiện tại" section for the exact layout and why it's split this way). `cargo run`/`cargo build`/`cargo test` all work with no `-p` flag (`default-members = ["crates/tradar-app"]`; the binary is still named `tradar`). It's a runnable walking skeleton: `tradar` connects to a real PostgreSQL, SQLite, MongoDB, Elasticsearch, or Redis instance (via `QueryDriver` implementations, one per crate under `crates/connectors/`), runs queries through `query_engine`, and renders a connection picker + query/results screen, driven by the `Component`/`Action` architecture (`RootComponent` in `tradar-app` holding `Vec<Tab>`, each independently on the picker or an active connector screen, wired together via the `mpsc` event loop in `main.rs`). Connections are added/edited/deleted from the picker itself (`a`/`e`/`d`, form in `crates/tradar-app/src/components/connection_form.rs`), persisted to a TOML file (`crates/tradar-core/src/storage/mod.rs`) that can still be hand-edited. Schema browsing is wired into the TUI as a sidebar on the query screen (loads on connect, `Tab` to focus, `Enter` to insert a name into the query). The query editor is a hand-rolled vim-modal text editor (`crates/tradar-query-workbench/src/components/query_editor.rs`, no longer backed by `edtui`) with tree-sitter SQL syntax highlighting on Postgres/SQLite connections. Keys are resolved through `tradar-core`'s `keymap` module (per-`Context` command lookup, remappable from `~/.config/tradar/config.toml`) rather than matched inline in components, colors come from `tradar-core`'s `theme` module, and shared widgets (bordered panel, selection style, status hint bar, `?` help overlay) live in `tradar-core`'s `ui` module — components must use these rather than hardcoding a `KeyCode`, a `Color`, or their own `Block`. Multiple tabs/connections can be open at once (`Ctrl+T`/`Ctrl+W`/`Ctrl+Left`/`Ctrl+Right`), and the app remembers which tabs were connected across restarts. General export (beyond Elasticsearch's curl export) is not built yet. The full architecture — current implementation and the target pluggable-connector design it's migrating to — lives in `docs/architecture.md`; read it before adding new modules. Roadmap/status lives in `docs/backlog.md`.

## What Tradar is

A terminal-first database exploration and query tool (TUI), in the spirit of LazyGit or k9s but for databases. It gives a unified, keyboard-driven interface for querying, browsing, analyzing, and managing different databases without switching between multiple native CLI clients — while still preserving each database's native query language (SQL for SQL databases, Mongo Shell JS for MongoDB, Query DSL for Elasticsearch) rather than inventing a custom query language.

v1 target databases: PostgreSQL, SQLite. Planned: MySQL, MariaDB, MongoDB, Elasticsearch, Redis, ClickHouse.

## Commands

This is a Rust project built with Cargo (edition 2024).

- Build: `cargo build`
- Run: `cargo run`
- Test: `cargo test` (run a single test: `cargo test <test_name>`)
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Check without building: `cargo check`

The Postgres driver's tests (`crates/tradar-app/src/drivers/postgres/mod.rs`) spin up a real Postgres via `testcontainers-modules`, which requires a working Docker daemon. If Docker isn't available, run `cargo test --workspace --lib -- --skip drivers::postgres` to exercise everything else. Since the split, most cargo invocations need `--workspace` to cover both crates (e.g. `cargo clippy --all-targets --workspace -- -D warnings`).

## Documentation language

Project docs (`docs/*.md`, `README.md`) are written in Vietnamese — keep technical/domain terms untranslated (crate names, trait/type/function names, cargo commands, code blocks, file paths) rather than translating everything, which loses precision. Respond to the user in Vietnamese in this project too, with the same term-preservation rule. This file (`CLAUDE.md`) itself stays in English, since it's operating instructions for Claude Code rather than project documentation.

## Architecture

See `docs/architecture.md` for the full module layout and the `QueryDriver`/`Connector`/`Session` trait contracts. The rule that matters most for any change: code in each connector crate under `crates/connectors/` only implements those traits and depends on nothing else in the workspace; code in `tradar-query-workbench` (components, `query_engine`) depends only on the `QueryDriver` trait, never on a concrete connector crate. This is what lets new databases be added without touching core logic — see the "Registry" section in `docs/architecture.md`.

Other standing principles from the design spec:

- Prefer interfaces over concrete implementations; reuse shared UI components whenever possible.
- Keyboard-first, terminal-first — every feature must work without a mouse; keep startup fast and memory usage low.
- Support large result sets efficiently — virtual scrolling/pagination, avoid loading unnecessary data into memory.
- Unit tests for core logic (`components`, `query_engine`), integration tests per driver.
- Update documentation whenever behavior changes.

## Current dependencies

- `tokio` (async runtime, full features) — async I/O for DB connections/queries
- `ratatui` + `crossterm` — the terminal UI layer
- `async-trait` — enables the async `QueryDriver`/`Connector`/`Session` traits
- `sqlx` (sqlite + postgres features) — the Postgres/SQLite connector crates are built on this
- `mongodb`, `reqwest` (Elasticsearch's REST API), `redis` — the other three connector crates
- `tree-sitter-highlight` + `tree-sitter-sequel` — SQL syntax highlighting in the query editor (`tradar-query-workbench`'s `sql_highlight.rs`), Postgres/SQLite only
- `toml` — saved-connections/session/config file format (`tradar-core` only)
- `directories` — platform-appropriate config path for the saved-connections/session/config files (`tradar-core` only)
- `serde` / `serde_json` — serialization
- `anyhow` — error handling
- `base64` — encodes yanked results text for the OSC52 clipboard escape sequence (`crates/tradar-query-workbench/src/components/query_screen.rs`'s `yank_to_clipboard`)
- Dev-only: `tempfile` (driver/storage tests use real temp files, not mocks), `testcontainers-modules` (spins up real Postgres/MongoDB/Elasticsearch/Redis instances for connector tests)

Present in `tradar-app/Cargo.toml` but not yet used by any code: `clap`, `thiserror`, `tracing`/`tracing-subscriber`.
