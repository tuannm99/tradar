# CLAUDE.md

Guidance for Claude Code (claude.ai/code) in this repository.

## Project state

**Primary interface: Neovim + `tradar-server`** (`nvim/`, `crates/tradar-server`) — not the TUI. `tradar-server` is a headless JSON-RPC server (unix socket) that holds every open connection; Neovim is the editor/navigation/vim-motion layer, and several Neovim sessions share one server. Shape, traits and protocol: `docs/architecture.md`. What's open: `docs/roadmap.md`.

**The TUI (`tradar-app`, `tradar-query-workbench`'s `components/`) is deprecated** — no new features and no proactive fixes; touch it only if the user asks (someone requests it). It stays in the tree and must keep building and passing `make check`. The connector crates and the `QueryDriver` trait are shared by both interfaces and are *not* deprecated. New user-facing features go into the Neovim plugin/server.

## Workspace

Cargo workspace; `cargo run`/`build`/`test` need no `-p` (`default-members = ["crates/tradar-app"]`, binary `tradar`). Crates under `crates/`:

- `tradar-core` (storage, theme, keymap, shared widgets), `tradar-connector-spi` (the `Connector`/`Session` traits), `tradar-query-workbench` (`QueryDriver`, `QueryEngine`, shared query UI).
- `tradar-connector-<name>` ×12: `postgres mysql sqlite mongo elasticsearch redis cassandra clickhouse` implement `QueryDriver`; `kafka rabbitmq http socket` implement `Connector`/`Session` directly. gRPC lives only in `tradar-server` (no TUI screen).
- `tradar-app` (TUI) and `tradar-server` (headless server).

Large crates are split by topic (e.g. `QueryDriver` in `query_driver/*`, Mongo in `shell|run|driver|snippets|...`, server in `grpc/` and `server/`, Neovim plugin in `nvim/lua/tradar/*` sharing `ctx.lua`); keep new code in the matching module and tests beside it (`tests.rs` / `tests/`) rather than growing `lib.rs`.

Connectors are gated by Cargo features named after `ConnectorDescriptor::id`; `default` enables all. Slim build: `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch` (or `make build-slim FEATURES=…`) — skips e.g. `rdkafka`, which needs `cmake`/`gcc`/`libcurl-dev`.

## Rules that matter

- A connector crate only implements the SPI traits and depends on nothing else in the workspace; `tradar-query-workbench` depends only on `QueryDriver`, never on a concrete connector. This is what lets new databases be added without touching core.
- Prefer interfaces over concrete types; reuse shared widgets. Keyboard-first; fast startup; large results must paginate rather than load everything.
- Keep each backend's native query language (SQL, Mongo shell JS, ES Query DSL) — no invented unified language.
- Additive protocol changes bump the minor in `protocol::PROTOCOL_VERSION`; mirror the number in `docs/architecture.md`.
- Tests: unit tests for core logic, integration tests per driver. Update docs when behavior changes — concisely; history and "why" go to `docs/backlog/` (index: `docs/backlog/README.md`), not into `docs/architecture.md`.

## Commands

- `cargo build` / `cargo run` / `cargo test [name]` / `cargo clippy --all-targets --workspace -- -D warnings` / `cargo fmt`
- Ten connectors' tests (everything but SQLite/Socket) start a real backend via `testcontainers` and need a Docker daemon. Without Docker: `make test-unit`. With: `make test-docker` (includes `make test-kafka-server`; set `TRADAR_DOCKER_HOST_NETWORK=1` on Docker without a bridge network).

## Documentation

Docs (`docs/*.md`, `README.md`, `docs/wiki/`) are in Vietnamese; keep technical terms (crate/trait/function names, commands, paths, code) untranslated. Reply to the user in Vietnamese with the same rule. This file stays in English.

- User guide: `docs/wiki/` (laid out for GitHub Wiki) — update the relevant page on any user-visible change; `nvim/README.md` is the plugin reference.

## Non-obvious dependency notes

- `tradar-connector-sqlite` uses `libsqlite3-sys` for `cancel_query` (`sqlite3_interrupt` on a raw handle cached at `connect()`; sqlx's safe API is unreachable while its worker thread is blocked).
- `tradar-connector-mongo` enables `serde_json`'s `preserve_order` so BSON field order survives (Cargo feature unification makes it workspace-wide).
- `reqwest` is shared by Elasticsearch, RabbitMQ (Management API), HTTP and ClickHouse (`FORMAT JSON`).
- `tradar-app/Cargo.toml` lists `thiserror`, `tracing`, `tracing-subscriber` but no code uses them yet.
