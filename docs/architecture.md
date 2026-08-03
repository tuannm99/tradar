# Architecture

Tradar is a single Cargo crate with layered modules, structured so that the boundaries between layers are already crate-boundary-shaped — a future split into a Cargo workspace (one crate per driver) would be mechanical rather than a redesign. See the [design spec](superpowers/specs/2026-08-01-tradar-v1-design.md) for why a workspace wasn't adopted immediately.

```
src/
  main.rs                 — the event loop: crossterm input -> Component actions -> query_engine/driver calls
  action.rs               — the Action enum defining all possible state transitions, and the Component trait
  components/             — ratatui components; RootComponent, ConnectionPickerComponent, and QueryScreenComponent implement the Component trait from action.rs, while query_editor.rs, results.rs, and schema_sidebar.rs are plain state+draw structs composed by QueryScreenComponent without implementing it
    mod.rs                — RootComponent composes ConnectionPickerComponent and QueryScreenComponent
    connection_picker.rs  — ConnectionPickerComponent
    query_screen.rs       — QueryScreenComponent (composes QueryEditorComponent, ResultsComponent, SchemaSidebarComponent)
    query_editor.rs       — QueryEditorComponent
    results.rs            — ResultsComponent
    schema_sidebar.rs     — SchemaSidebarComponent
  query_engine/           — takes a query string, hands it to the active driver, tracks history
  drivers/
    mod.rs                — the Driver trait (connect, list_schema, execute, ...)
    postgres/
    sqlite/
    elasticsearch/
    redis/
    mongo/
  storage/                — saved connections as TOML (via the `directories` crate for the config path)
  config/                 — reserved for app config loading; not used yet
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

`SchemaInfo` and `QueryResult` are the normalized shapes the rest of the app renders — a driver is responsible for translating its database's native results into these types. `QueryResult` is an enum, not a single struct, because SQL results and document-shaped results (MongoDB, Elasticsearch, Redis) don't fit the same shape:

```rust
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

`Table` is what Postgres and SQLite return. `Documents` is shared by the other three drivers: each Elasticsearch hit/response, MongoDB document, or Redis reply becomes one `serde_json::Value` in the vec. `ResultsComponent` renders `Table` as a text table and `Documents` as pretty-printed JSON blocks.

## Isolation rule

This is the rule that keeps drivers pluggable, and it's enforced everywhere, not just at the top level:

- Code under `drivers/*` implements `Driver` and depends on nothing else in the app.
- Code in `components/`, `action.rs`, and `query_engine` depends only on the `Driver` trait — never on `drivers::postgres`, `drivers::sqlite`, or any other concrete driver module.
- `main.rs` is the only place that constructs a concrete driver (in `Action::ConnectRequested`) or calls a concrete driver helper (in `Action::ExportCurl`).

Adding a new database means adding a new module under `drivers/` that implements `Driver`. It should never require changes to `components/`, `action.rs`, or `query_engine`.

## Current state

The v1 walking skeleton works end to end: `tradar` loads saved connections from `storage`, connects via the selected `Driver` (Postgres, SQLite, Elasticsearch, Redis, or MongoDB, all fully implemented against real backends), and runs queries typed into `QueryScreenComponent`'s query editor through `query_engine`, rendering real results or errors.

Notably thin/missing pieces:

- No interactive "add connection" screen — connections are added by hand-editing the TOML file.
- `Driver::list_schema` is implemented and tested for all five drivers, and wired into the TUI as a schema sidebar on the query screen (loads automatically on connect; `Tab` to focus it, `Enter` to insert the selected name into the query).
- `config/` is an empty placeholder module; app configuration beyond the connections file doesn't exist yet.
