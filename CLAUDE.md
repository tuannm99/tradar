# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

**Primary interface: Neovim + `tradar-server`** (`nvim/`, `crates/tradar-server`) — not the TUI. `tradar-server` is a headless JSON-RPC server (over a unix socket) that holds every open connection; Neovim is the editor/navigation/vim-motion layer on top, and multiple Neovim sessions share one server process. See `docs/architecture.md` for the full shape (crate layout, trait contracts, protocol) and `docs/roadmap.md` for what's open.

**TUI (`tradar-app`, and `tradar-query-workbench`'s `components/`) is frozen**: bug fixes only, no new features. It must keep building and passing `make check`. The connector crates and `QueryDriver` trait are shared by both interfaces and are *not* frozen — a fix there reaches both. New user-facing features go into the Neovim plugin/server. The decision is made: the TUI stays frozen (it is not being removed).

The repo is a Cargo workspace. `cargo run`/`cargo build`/`cargo test` all work with no `-p` flag (`default-members = ["crates/tradar-app"]`; the binary is still named `tradar`). Crate layout:

- `tradar-core` — `Action`/`Component`, `Capability`, storage (saved connections, session state, config, snippets, saved HTTP requests), theme, keymap, shared UI widgets.
- `tradar-connector-spi` — the `Connector`/`Session` trait contracts every connector implements (named `-spi`, not `-api`, so it doesn't read like just another connector).
- `tradar-query-workbench` — the `QueryDriver` trait, `QueryEngine`, and the query editor/results UI shared by every SQL-shaped connector.
- Twelve connector crates, each `tradar-connector-<name>`, living directly under `crates/` (not nested under a `connectors/` subdirectory): `postgres`, `mysql`, `sqlite`, `mongo`, `elasticsearch`, `redis`, `cassandra`, `clickhouse` implement `QueryDriver`; `kafka`, `rabbitmq`, `http`, `socket` have no query language and implement `Connector`/`Session` directly, each with its own bespoke TUI `Screen`.
- `tradar-app` — the TUI binary (`[[bin]] name = "tradar"`).
- `tradar-server` — the headless JSON-RPC server that serves the query-shaped connectors (and, separately, gRPC/Kafka/Socket/RabbitMQ/HTTP through their own non-`QueryDriver` methods) to Neovim. See `docs/architecture.md`'s "Server headless" section for the protocol.

`tradar` connects to a real PostgreSQL, MySQL/MariaDB, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, ClickHouse, or RabbitMQ instance, a Kafka cluster, an arbitrary HTTP endpoint, or a raw TCP socket. gRPC is server/plugin-only (no TUI screen — see `docs/architecture.md`).

Both `tradar-app` and `tradar-server` gate each connector behind a Cargo feature named after `ConnectorDescriptor::id` (`postgres`, `mysql`, `sqlite`, `mongo`, `elasticsearch`, `redis`, `cassandra`, `clickhouse`, `rabbitmq`, `kafka`, `http`, `socket`). `default` includes everything, so a plain `cargo build`/`cargo run` is unchanged. Build a binary with only the connectors you want via `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch` (or `make build-slim FEATURES=mongo,elasticsearch`) — this skips every other connector's dependencies, most usefully `kafka`'s (`rdkafka`, which needs `cmake`/`gcc`/`libcurl-dev` to build `librdkafka` from vendored source).

## What Tradar is

A terminal-first database exploration and query tool, in the spirit of LazyGit or k9s but for databases. It gives a unified, keyboard-driven interface for querying, browsing, analyzing, and managing different databases without switching between multiple native CLI clients — while still preserving each database's native query language (SQL for SQL databases, Mongo Shell JS for MongoDB, Query DSL for Elasticsearch) rather than inventing a custom one.

Databases: PostgreSQL, MySQL/MariaDB, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, ClickHouse. Non-database connectors (same `Connector`/`Session`/`Screen` architecture): Kafka, RabbitMQ, HTTP, Socket (raw TCP), gRPC (server/plugin only, no TUI).

## Commands

This is a Rust project built with Cargo (edition 2024).

- Build: `cargo build`
- Run: `cargo run`
- Test: `cargo test` (run a single test: `cargo test <test_name>`)
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Check without building: `cargo check`

Ten of the twelve connectors' tests (Postgres, MySQL, Redis, Mongo, Elasticsearch, Cassandra, ClickHouse, RabbitMQ, Kafka, HTTP — everything but SQLite/Socket) spin up a real backend via `testcontainers`/`testcontainers-modules`, which requires a working Docker daemon. If Docker isn't available, run `make test-unit` (excludes those ten connector crates plus `tradar-app`, then runs `tradar-app`'s own tests separately with a trimmed feature set so building it doesn't pull in `kafka`'s native toolchain requirement) to exercise everything else; `make test-docker` runs just the Docker-dependent ones (including `make test-kafka-server`, `tradar-server`'s own Kafka end-to-end test; on a Docker without a bridge network set `TRADAR_DOCKER_HOST_NETWORK=1`). Most cargo invocations need `--workspace` to cover every crate (e.g. `cargo clippy --all-targets --workspace -- -D warnings`).

## Documentation language

Project docs (`docs/*.md`, `README.md`) are written in Vietnamese — keep technical/domain terms untranslated (crate names, trait/type/function names, cargo commands, code blocks, file paths) rather than translating everything, which loses precision. Respond to the user in Vietnamese in this project too, with the same term-preservation rule. This file (`CLAUDE.md`) itself stays in English, since it's operating instructions for Claude Code rather than project documentation.

## Architecture

See `docs/architecture.md` for the current module layout and the `QueryDriver`/`Connector`/`Session` trait contracts, kept short and current — it describes the shape as it stands today, not the history of how it got there. The rule that matters most for any change: code in each connector crate (`tradar-connector-*`, directly under `crates/`) only implements those traits and depends on nothing else in the workspace; code in `tradar-query-workbench` (components, `query_engine`) depends only on the `QueryDriver` trait, never on a concrete connector crate. This is what lets new databases be added without touching core logic.

Finished sub-project history (what each feature was, why, and how it was verified) lives in `docs/backlog/` — one file per sub-project, indexed by `docs/backlog/README.md`. Open/unscoped roadmap items live in `docs/roadmap.md`. The user-facing usage guide lives in `docs/wiki/` (Markdown pages, laid out for GitHub Wiki: `Home.md`, `_Sidebar.md`, one page per topic) — update the relevant page when user-visible behavior changes; `nvim/README.md` is the plugin's own reference.

Other standing principles:

- Prefer interfaces over concrete implementations; reuse shared UI components whenever possible.
- Keyboard-first, terminal-first — every feature must work without a mouse; keep startup fast and memory usage low.
- Support large result sets efficiently — virtual scrolling/pagination, avoid loading unnecessary data into memory.
- Unit tests for core logic (`components`, `query_engine`), integration tests per driver.
- Update documentation whenever behavior changes — concisely; put the why/history in `docs/backlog/`, not inline in `docs/architecture.md`.

## Current dependencies

- `tokio` (async runtime, full features) — async I/O for DB connections/queries
- `ratatui` + `crossterm` — the TUI layer (also a direct dependency of the four non-`QueryDriver` connector crates, which draw their own `Screen`)
- `async-trait` — enables the async `QueryDriver`/`Connector`/`Session` traits
- `sqlx` (sqlite + postgres + mysql features) — Postgres/MySQL/SQLite connectors
- `libsqlite3-sys` — `tradar-connector-sqlite` only, for `cancel_query` (`sqlite3_interrupt` on a raw handle cached at `connect()` time — sqlx's own safe API can't be reached while the connection's worker thread is itself blocked)
- `mongodb`, `redis` — the Mongo/Redis connectors
- `reqwest` — Elasticsearch's REST client, reused by `tradar-connector-rabbitmq` (Management HTTP API), `tradar-connector-http`, and `tradar-connector-clickhouse` (its HTTP interface, `FORMAT JSON`)
- `scylla` — the Cassandra connector (CQL over the native binary protocol, pure Rust)
- `rdkafka` (feature `cmake-build`) — the Kafka connector; builds `librdkafka` from vendored source, needs `cmake`/`gcc`/`libcurl-dev`
- `tree-sitter-highlight` + `tree-sitter-sequel` — SQL syntax highlighting in the query editor, Postgres/MySQL/SQLite/ClickHouse/Cassandra
- `toml` / `directories` — saved-connections/session/config/snippets file format and platform config path (`tradar-core` only)
- `serde` / `serde_json` — serialization; `tradar-connector-mongo` enables `preserve_order` so a BSON document's field order survives into JSON (Cargo feature unification means this applies workspace-wide)
- `anyhow` — error handling
- `base64` / `arboard` — clipboard: OSC52 yank (`tradar-core::ui::yank_to_clipboard`) and real OS-clipboard paste (`default-features = false`, text only)
- `clap` (derive) — `tradar-app`'s `--help`/`--version`
- Dev-only: `tempfile`, `testcontainers-modules`, `testcontainers` (`GenericImage` for Cassandra/RabbitMQ/Kafka/HTTP, no `testcontainers-modules` support for those four)

Present in `tradar-app/Cargo.toml` but not yet used by any code: `thiserror`, `tracing`/`tracing-subscriber`.
