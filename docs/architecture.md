# Architecture

Tradar is a single Cargo crate with layered modules, structured so that the boundaries between layers are already crate-boundary-shaped — a future split into a Cargo workspace (one crate per driver) would be mechanical rather than a redesign. See the [design spec](superpowers/specs/2026-08-01-tradar-v1-design.md) for why a workspace wasn't adopted immediately.

```
src/
  tui/            — ratatui views/widgets, input handling
  app/            — application state, event loop, command dispatch
  query_engine/   — takes a query string, hands it to the active driver, normalizes results
  drivers/
    mod.rs        — the Driver trait (connect, list_schema, execute, ...)
    postgres/
    sqlite/
  storage/        — local config/connection persistence (via the `directories` crate)
  config/         — app config loading
```

## The `Driver` trait

Every database backend implements one shared trait, defined in `src/drivers/mod.rs`:

```rust
#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;
}
```

`QueryResult` and `SchemaInfo` are the normalized shapes the rest of the app renders — a driver is responsible for translating its database's native results into these types.

## Isolation rule

This is the rule that keeps drivers pluggable, and it's enforced everywhere, not just at the top level:

- Code under `drivers/*` implements `Driver` and depends on nothing else in the app.
- Code in `app`, `tui`, and `query_engine` depends only on the `Driver` trait — never on `drivers::postgres`, `drivers::sqlite`, or any other concrete driver module.

Adding a new database means adding a new module under `drivers/` that implements `Driver`. It should never require changes to `app`, `tui`, or `query_engine`.

## Current state

Only the module skeleton exists today: the `Driver` trait is defined, and `drivers::postgres` / `drivers::sqlite` are stub implementations (`todo!()` bodies). `app`, `tui`, `query_engine`, `storage`, and `config` are empty modules awaiting the v1 implementation plan described in the design spec.
