# Tradar

Tradar is a terminal-first database exploration and query tool (TUI), in the spirit of [LazyGit](https://github.com/jesseduffield/lazygit) or [k9s](https://k9scli.io/) — but for databases. The name is a portmanteau of the author's name (tuannm) and "radar".

It gives you a single, keyboard-driven interface for connecting to, browsing, and querying different databases, so you stop context-switching between native CLI clients. Tradar doesn't invent its own query language: you write real SQL against SQL databases, real Mongo Shell JavaScript against MongoDB, and real Query DSL against Elasticsearch.

## Status

Pre-alpha, but runnable: `tradar` connects to a real PostgreSQL or SQLite database, runs queries, and shows results in the terminal — connection picker → query screen → results, all keyboard-driven. There's no interactive "add connection" screen yet, so saved connections must be added by hand to the TOML file at the path `tradar` prints when none exist (see `src/storage/mod.rs`). Schema browsing, multi-tab editing, and export are not built yet. See `docs/architecture.md` for the shape of the system and the [design spec](docs/superpowers/specs/2026-08-01-tradar-v1-design.md) for the full v1 plan.

## Databases

**v1 target:** PostgreSQL, SQLite

**Planned:** MySQL, MariaDB, MongoDB, Elasticsearch, Redis, ClickHouse

New database support is added as a `Driver` implementation without touching the rest of the application — see `docs/architecture.md`.

## Philosophy

- **Keyboard-first.** Every feature works without a mouse.
- **Terminal-first.** Fast startup, low memory usage, no browser or Electron shell.
- **Native query languages.** SQL for SQL databases, Mongo Shell JS for MongoDB, Query DSL for Elasticsearch — not a custom unified language.
- **Database-agnostic core.** Business logic never depends on a specific database; every driver is isolated behind one shared interface.

## Building

Requires Rust (edition 2024).

```bash
cargo build   # build
cargo run     # run
cargo test    # test
cargo clippy  # lint
cargo fmt     # format
```
