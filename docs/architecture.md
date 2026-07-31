# Architecture

Tradar is a single Cargo crate with layered modules, structured so that the boundaries between layers are already crate-boundary-shaped — a future split into a Cargo workspace (one crate per driver) would be mechanical rather than a redesign. See the [design spec](superpowers/specs/2026-08-01-tradar-v1-design.md) for why a workspace wasn't adopted immediately.

```
src/
  main.rs         — the event loop: crossterm input -> App transitions -> query_engine/driver calls
  tui/            — ratatui views/widgets: draw(frame, &App), pure rendering
  app/            — App/Screen: synchronous state machine (no I/O, no ratatui, fully unit-tested)
  query_engine/   — takes a query string, hands it to the active driver, tracks history
  drivers/
    mod.rs        — the Driver trait (connect, list_schema, execute, ...)
    postgres/
    sqlite/
  storage/        — saved connections as TOML (via the `directories` crate for the config path)
  config/         — reserved for app config loading; not used yet
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

The v1 walking skeleton works end to end: `tradar` loads saved connections from `storage`, connects via the selected `Driver` (Postgres or SQLite, both fully implemented against real databases), and runs queries typed into the `tui`'s query screen through `query_engine`, rendering real results or errors.

Notably thin/missing pieces:

- No interactive "add connection" screen — connections are added by hand-editing the TOML file.
- `Driver::list_schema` is implemented and tested for both drivers, but nothing in the TUI calls it yet — there's no schema explorer pane.
- `config/` is an empty placeholder module; app configuration beyond the connections file doesn't exist yet.
- Query editor is single-line with no syntax highlighting or autocomplete, matching v1 scope in the design spec but worth noting as intentionally minimal, not an oversight.
