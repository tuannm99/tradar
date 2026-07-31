# Tradar: v1 Design

Status: Approved
Date: 2026-08-01
Amended: 2026-08-01 — see "Naming" below; an earlier revision of this spec briefly renamed the project to "Rowdy". That name turned out to already be in use by another project, so the project reverts to its original name, `tradar`.

## Context

The project lives in this repo under the Cargo package name `tradar` — a portmanteau of the author's name (tuannm) and "radar", not related to trading despite the surface reading. The current `init.md` product brief (now retired, see Documentation deliverables) describes a terminal-first database exploration and query tool (TUI), in the spirit of LazyGit or k9s but for databases — a unified, keyboard-driven interface for querying, browsing, analyzing, and managing multiple databases (PostgreSQL, MySQL, MariaDB, SQLite, MongoDB, Elasticsearch, with Redis/ClickHouse planned) without switching between native CLI clients, while preserving each database's native query language rather than inventing a custom one.

The codebase itself is still just Cargo scaffolding (`src/main.rs` prints "Hello, world!") at the time this spec was originally written. This spec defines the scope and architecture for the project's first real (v1) milestone.

## Goals

1. Define a v1 ("walking skeleton") scope: connect to a database, browse its schema, run a query, see results — end to end, keyboard-only — for exactly two drivers (PostgreSQL and SQLite).
2. Establish an architecture with strict isolation between core application logic and per-database driver code, so additional drivers can be added later with minimal change to the core.
3. Produce documentation (`README.md`, `docs/architecture.md`) that captures the product pitch, v1 scope, and architecture, replacing `init.md` as the primary reference.

## Non-goals (explicitly deferred past v1)

- MongoDB, Elasticsearch, Redis, ClickHouse drivers.
- SSH tunnel support, TLS support, connection groups.
- Multi-tab query editor, autocomplete, query history persistence beyond in-session.
- Transaction control, explain plan, stored procedures, view definitions.
- Any workspace split into multiple crates (see Architecture below — deferred until there's a concrete second reason for it, e.g. a third driver or an out-of-tree plugin).

## Naming

The project keeps the name `tradar` (Cargo package and binary). It is a portmanteau of the author's name (tuannm) + "radar" — a deliberate, personal name, not a leftover from a different project concept.

An earlier revision of this spec renamed the project to "Rowdy" based on a (mistaken) assumption that `tradar` was leftover naming from an unrelated trading-platform idea. That name is already in use by another project, so the rename was reverted; no further name-collision search is needed since the original name is being kept.

- `init.md` is retired as a standalone root-level file once its content is absorbed into `README.md` and `docs/architecture.md` — contributors and future agents should not need to find and parse a separate scratch brief.
- `CLAUDE.md` is updated to reflect this spec's scope.

## v1 Scope ("walking skeleton")

Goal: `tradar` launches, the user picks a saved connection (Postgres or SQLite), browses its schema, runs a query, and sees results — fully keyboard-driven, no mouse required.

In scope:

- **Connections**: saved/named connections, persisted locally via the `directories` crate's config path. No connection groups, no SSH tunnel, no TLS in v1.
- **Schema explorer**:
  - Postgres: databases, schemas, tables, views, columns, indexes, constraints.
  - SQLite: databases (single file), tables, columns, indexes.
- **Query editor**: single-tab, syntax highlighting, execute query, in-session query history. No autocomplete, no multi-tab.
- **Result viewer**: paginated table view, JSON tree view, export to CSV and JSON.
- Both drivers implement one shared `Driver` trait; no Postgres- or SQLite-specific code exists outside `drivers/postgres` and `drivers/sqlite`.

## Architecture

Single crate, layered modules with workspace-shaped boundaries — i.e., driver isolation is enforced by convention and trait boundaries now, so a future split into a Cargo workspace (separate crates per driver) is a mechanical move rather than a redesign. This was chosen over splitting into a workspace immediately: with only two drivers in v1, workspace machinery (crate boundaries, versioning, inter-crate APIs) is overhead without a second consumer to justify it yet.

```
src/
  tui/            — ratatui views/widgets, input handling
  app/            — application state, event loop, command dispatch
  query_engine/   — takes a query string, hands it to the active driver, normalizes results
  drivers/
    mod.rs        — the `Driver` trait (connect, list_schema, execute, ...)
    postgres/
    sqlite/
  storage/        — local config/connection persistence (via `directories`)
  config/         — app config loading
```

Isolation rule (enforced throughout, not just at the top level): code under `drivers/*` only implements the `Driver` trait and depends on nothing else in the app. Code in `app`, `tui`, and `query_engine` only ever depends on the `Driver` trait — never on `drivers::postgres` or `drivers::sqlite` directly.

## Testing approach

- Unit tests for core logic in `query_engine` and `app`.
- Integration tests per driver: SQLite against a local temp file; Postgres against a test instance (exact mechanism — e.g. testcontainers vs. a docker-compose fixture vs. a CI service container — is an implementation-plan decision, not fixed by this spec).

## Documentation deliverables

- `README.md`: project pitch (LazyGit/k9s-for-databases framing), supported databases (v1: Postgres + SQLite; planned: MongoDB, Elasticsearch, Redis, ClickHouse), build/run instructions, current status (pre-alpha, v1 in progress), keyboard-first philosophy.
- `docs/architecture.md`: the module layout above, the `Driver` trait contract, and the isolation rule.

## Open questions for the implementation plan

- Exact `Driver` trait method signatures and error types.
- Exact Postgres integration-test fixture mechanism (see Testing approach above).
- Local connection-storage file format (e.g. TOML via existing `serde`/`directories` deps) and whether credentials need encryption at rest — deferred to the implementation plan since it affects the `storage` module directly.
