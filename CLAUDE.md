# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Tradar's name is a portmanteau of the author's name (tuannm) + "radar" — not related to trading, despite the surface reading. The project briefly went through a rename to "Rowdy" before reverting back to `tradar`; don't propose renaming it again.

The crate is a runnable walking skeleton: `cargo run` connects to a real PostgreSQL or SQLite database (via `Driver` implementations in `src/drivers/postgres` and `src/drivers/sqlite`), runs queries through `query_engine`, and renders a connection picker + query/results screen via `tui`, driven by `app`'s state machine and wired together in `main.rs`. There's no interactive "add connection" UI yet — saved connections are read from a TOML file (`src/storage/mod.rs`) that has to be edited by hand. Schema browsing (`SchemaInfo`/`list_schema` exist on `Driver` but aren't wired into the TUI), multi-tab editing, syntax highlighting, and export are not built yet. The full v1 scope and architecture rationale live in `docs/superpowers/specs/2026-08-01-tradar-v1-design.md` — read it before adding new modules.

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

The Postgres driver's tests (`src/drivers/postgres/mod.rs`) spin up a real Postgres via `testcontainers-modules`, which requires a working Docker daemon. If Docker isn't available, run `cargo test --lib -- --skip drivers::postgres` to exercise everything else.

## Architecture

See `docs/architecture.md` for the full module layout and the `Driver` trait contract. The rule that matters most for any change: code under `drivers/*` only implements `Driver` and depends on nothing else in the app; code in `app`, `tui`, and `query_engine` depends only on the `Driver` trait, never on a concrete driver module (`drivers::postgres`, `drivers::sqlite`, etc.). This is what lets new databases be added without touching core logic.

Other standing principles from the design spec:

- Prefer interfaces over concrete implementations; reuse shared UI components whenever possible.
- Keyboard-first, terminal-first — every feature must work without a mouse; keep startup fast and memory usage low.
- Support large result sets efficiently — virtual scrolling/pagination, avoid loading unnecessary data into memory.
- Unit tests for core logic (`app`, `query_engine`), integration tests per driver.
- Update documentation whenever behavior changes.

## Current dependencies

- `tokio` (async runtime, full features) — async I/O for DB connections/queries
- `ratatui` + `crossterm` — the terminal UI layer
- `async-trait` — enables the async `Driver` trait
- `sqlx` (sqlite + postgres features) — the two v1 drivers are built on this
- `toml` — saved-connections file format
- `directories` — platform-appropriate config path for the saved-connections file
- `serde` / `serde_json` — serialization
- `anyhow` — error handling
- Dev-only: `tempfile` (driver/storage tests use real temp files, not mocks), `testcontainers-modules` (spins up a real Postgres for driver tests)

Present but not yet used by any code: `reqwest` (planned for Elasticsearch's REST API), `clap`, `thiserror`, `tracing`/`tracing-subscriber`.
