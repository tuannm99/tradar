# CLAUDE.md

Guidance for Claude Code (claude.ai/code) in this repository.

## Project state

**Primary interface: Neovim + `tradar-server`** (`nvim/`, `crates/tradar-server`) — not the TUI. `tradar-server` is a headless JSON-RPC server (unix socket) that holds every open connection; Neovim is the editor/navigation/vim-motion layer, and several Neovim sessions share one server. Shape, traits and protocol: `docs/architecture.md`. What's open: `docs/roadmap.md`.

**The TUI (`tradar-app`, `tradar-query-workbench`'s `components/`) is deprecated** — no new features and no proactive fixes; touch it only if the user asks (someone requests it). It stays in the tree and must keep building and passing `make check`. The connector crates and `tradar-query` are shared by both interfaces and are *not* deprecated. `tradar-server` must not depend on anything TUI: it uses `tradar-core` without `ui` and connectors without `tui` (check with `cargo tree -p tradar-server`). New user-facing features go into the Neovim plugin/server.

## Workspace

Cargo workspace; `cargo run`/`build`/`test` need no `-p` (`default-members = ["crates/tradar-app"]`, binary `tradar`). Crates under `crates/`:

- `tradar-query` (`QueryDriver`, result/schema types, statement splitting, completion, edit helpers, `DriverOpener`; no UI, no workspace deps), `tradar-core` (storage; with its default `ui` feature also theme, keymap, shared widgets), `tradar-connector-spi` (TUI `Connector`/`Session` traits), `tradar-query-workbench` (TUI: `QueryEngine`, query screens; `query_driver` re-exports `tradar-query`).
- `tradar-connector-<name>` ×12: `postgres mysql sqlite mongo elasticsearch redis cassandra clickhouse` implement `QueryDriver`; `kafka rabbitmq http socket` have a TUI-free shared module (`api`/`request`/`preview`/`tail`, used by the server's own modules) and implement `Connector`/`Session` in `session.rs` behind the default `tui` feature. gRPC lives only in `tradar-server` (no TUI screen).
- `tradar-app` (TUI) and `tradar-server` (headless server).

Large crates are split by topic (e.g. `QueryDriver` in `query_driver/*`, Mongo in `shell|run|driver|snippets|...`, server in `grpc/` and `server/`, Neovim plugin in `nvim/lua/tradar/*` sharing `ctx.lua`); keep new code in the matching module and tests beside it (`tests.rs` / `tests/`) rather than growing `lib.rs`.

Connectors are gated by Cargo features named after `ConnectorDescriptor::id`; `default` enables all. Slim build: `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch` (or `make build-slim FEATURES=…`) — skips e.g. `rdkafka`, which needs `cmake`/`gcc`/`libcurl-dev`.

## Rules that matter

- A query connector crate implements `QueryDriver` and exports `opener()` (server); its TUI `Connector` lives in `tui.rs` behind the default `tui` feature. It depends on `tradar-query` (plus the TUI crates only under `tui`), never on another connector; nothing shared depends on a concrete connector. This is what lets new databases be added without touching core.
- Prefer interfaces over concrete types; reuse shared widgets. Keyboard-first; fast startup; large results must paginate rather than load everything.
- Keep each backend's native query language (SQL, Mongo shell JS, ES Query DSL) — no invented unified language.
- Additive protocol changes bump the minor in `protocol::PROTOCOL_VERSION`; mirror the number in `docs/architecture.md`.
- Tests: unit tests for core logic, integration tests per driver, and an `nvim/tests/*_spec.lua` for user-visible plugin behavior (don't leave plugin checks as ad-hoc scripts). Update docs when behavior changes — concisely; history and "why" go to `docs/backlog/` (index: `docs/backlog/README.md`), not into `docs/architecture.md`.

## Commands

- `cargo build` / `cargo run` / `cargo test [name]` / `cargo clippy --all-targets --workspace -- -D warnings` / `cargo fmt`
- `make test-nvim` (or `nvim/tests/run.sh [filter]`): the Neovim plugin's tests -- each `nvim/tests/*_spec.lua` runs in its own headless Neovim against its own `tradar-server` and an empty SQLite database (`nvim/tests/helpers.lua`). Needs `nvim`, `python3` and (first run) network for telescope/plenary, cloned pinned into `.deps/` by `nvim/tests/deps.sh`; no Docker. CI (`.github/workflows/ci.yml`) runs `make check` + `make test-nvim` on every push.
- Ten connectors' tests (everything but SQLite/Socket) start a real backend via `testcontainers` and need a Docker daemon. Without Docker: `make test-unit`. With: `make test-docker` (includes `make test-kafka-server`; set `TRADAR_DOCKER_HOST_NETWORK=1` on Docker without a bridge network).

## Documentation

Docs (`docs/*.md`, `README.md`, `docs/wiki/`) are in Vietnamese; keep technical terms (crate/trait/function names, commands, paths, code) untranslated. Reply to the user in Vietnamese with the same rule. This file stays in English.

- User guide: `docs/wiki/` (laid out for GitHub Wiki) — update the relevant page on any user-visible change; `nvim/README.md` is the plugin reference.

## Non-obvious dependency notes

- `tradar-connector-sqlite` uses `libsqlite3-sys` for `cancel_query` (`sqlite3_interrupt` on a raw handle cached at `connect()`; sqlx's safe API is unreachable while its worker thread is blocked).
- `tradar-connector-mongo` enables `serde_json`'s `preserve_order` so BSON field order survives (Cargo feature unification makes it workspace-wide).
- `reqwest` is shared by Elasticsearch, RabbitMQ (Management API), HTTP and ClickHouse (`FORMAT JSON`).
- `tradar-app/Cargo.toml` lists `thiserror`, `tracing`, `tracing-subscriber` but no code uses them yet.
