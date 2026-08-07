# Architecture

This document has two parts: the architecture as implemented today (single Cargo crate, one `Driver` trait), and the target architecture the project is migrating to (a Cargo workspace with a `Connector → Session → Screen` pipeline) so that non-query-shaped systems — message brokers, watch-based systems, etc. — can be added without reshaping core code. The migration itself has not started; nothing under "Current implementation" is stale yet.

## Current implementation

Tradar is a single Cargo crate with layered modules, structured so that the boundaries between layers are already crate-boundary-shaped.

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

### The `Driver` trait

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

### Per-driver query language scope

Postgres and SQLite accept arbitrary SQL. The other three drivers accept a deliberately narrow subset, not their full native query language:

- **Elasticsearch**: modeled on Kibana's Dev Tools console, not a fixed Search-only client. First line is `METHOD /path` (e.g. `GET my-index/_search`); remaining lines, if any, are the JSON request body sent as-is. No auth/TLS client-cert config, and one request per execution (no multi-request scripts). The full JSON response is wrapped as a single-element `Documents`, not unwrapped into per-hit documents. `Ctrl+Y` on an Elasticsearch connection exports the current request as a `curl` command to `./tradar-query.sh` (fixed path, overwritten each export).
- **Redis**: a single command line, split on whitespace (no quoting/escaping support), sent via `redis::cmd`. Result conversion is type-aware only for `HGETALL` (→ JSON object) and `ZRANGE`/`ZREVRANGE ... WITHSCORES` (→ array of `{member, score}` objects); every other command uses the generic RESP-to-JSON conversion. No pipelining, transactions (`MULTI`/`EXEC`), pub/sub, or stream-specific (`XADD`/`XRANGE`) handling.
- **MongoDB**: a minimal shell-subset parser for the literal shape `db.<collection>.<method>(<json-args>)` — not a JS engine. Supports `find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`. No method chaining (`.sort()`, `.limit()`), no `$where`, no bulk operations or transactions; anything outside this shape returns an "unsupported query" error.

### Isolation rule

This is the rule that keeps drivers pluggable, and it's enforced everywhere, not just at the top level:

- Code under `drivers/*` implements `Driver` and depends on nothing else in the app.
- Code in `components/`, `action.rs`, and `query_engine` depends only on the `Driver` trait — never on `drivers::postgres`, `drivers::sqlite`, or any other concrete driver module.
- `main.rs` is the only place that constructs a concrete driver (in `Action::ConnectRequested`) or calls a concrete driver helper (in `Action::ExportCurl`).

Adding a new database means adding a new module under `drivers/` that implements `Driver`. It should never require changes to `components/`, `action.rs`, or `query_engine`.

### Current state

The v1 walking skeleton works end to end: `tradar` loads saved connections from `storage`, connects via the selected `Driver` (Postgres, SQLite, Elasticsearch, Redis, or MongoDB, all fully implemented against real backends), and runs queries typed into `QueryScreenComponent`'s query editor through `query_engine`, rendering real results or errors.

Notably thin/missing pieces:

- No interactive "add connection" screen — connections are added by hand-editing the TOML file.
- `Driver::list_schema` is implemented and tested for all five drivers, and wired into the TUI as a schema sidebar on the query screen (loads automatically on connect; `Tab` to focus it, `Enter` to insert the selected name into the query).
- `config/` is an empty placeholder module; app configuration beyond the connections file doesn't exist yet.

## Target architecture: pluggable connectors

All five current drivers share one shape: `connect → list_schema → execute(query) -> Table | Documents`, enforced by the single `Driver` trait and single `QueryScreenComponent` UI above. That shape doesn't fit systems Tradar plans to support next — message brokers (Kafka, RabbitMQ), watch/inspect-live-state systems (Kubernetes, Docker, Prometheus), and remote-shell-shaped tools (SSH). Kafka/RabbitMQ aren't "query a string, get rows back" — they're browse-topic/queue, tail messages live, publish a message. Cassandra (CQL) is the exception: it fits the query shape and can reuse the existing UI.

The following defines the target shape these will be built into. **It is architecture only — no code has moved yet.** The migration is a cross-cutting refactor (touches all five existing drivers, `RootComponent`, `main.rs`, and `storage`) meant to be executed incrementally — stand up the workspace and core crates first, then migrate one driver at a time — not as a single large change.

### Decisions

- **Pluggable = static/compile-time, not dynamic loading.** No `.so`/`.wasm` plugin loading, no third-party plugin ecosystem. Every connector is a Rust crate compiled into the `tradar` binary.
- **Split into a Cargo workspace, one crate per module.** The v1 spec deferred this until "a concrete second reason"; adding fundamentally different connector shapes (message queues, watch-based systems) alongside existing query-shaped ones is that reason. A workspace makes the isolation rule a Cargo dependency-graph fact, not a comment-enforced convention.
- **Each connector owns its own Screen**, not a shared query-editor shape. Connectors that *are* query-shaped (SQL, Mongo, ES, Redis, future Cassandra) still share one UI crate so they don't each reimplement it.
- **"Backend"/"Driver" is renamed to Connector** throughout (`PostgresConnector`, `KafkaConnector`, ...) — a Postgres database, a Kafka cluster, and an SSH host aren't "backends"/"drivers" in any shared sense the old name captured.
- **Connecting and building UI are two different jobs, done by two different types:** a `Connector` produces a `Session`; a `Session` produces a `Screen`.

### Workspace layout

```
Cargo.toml                    [workspace]
crates/
  tradar-core/                — Action, Component trait, Connector/Session traits, Capability,
                                 storage (SavedConnection, ConnectionStore), config
  tradar-query-workbench/     — QueryScreenComponent, ResultsComponent, SchemaSidebarComponent, QueryEditorComponent,
                                 QueryEngine (implements Session), QueryDriver trait, SchemaInfo/QueryResult
                                 (today's components/query_screen.rs et al., moved wholesale — not new code)
  connectors/
    tradar-postgres/  tradar-sqlite/  tradar-mongo/  tradar-elasticsearch/  tradar-redis/
    (future) tradar-kafka/, tradar-rabbitmq/, tradar-cassandra/
  tradar-app/ (binary crate)  — main.rs, RootComponent, ConnectionPickerComponent, the connector registry
```

`tradar-query-workbench` is named "workbench", not "ui" — it bundles the editor, execution, and history for query-shaped connectors, not just widgets.

Dependency direction, enforced by Cargo (not just convention):

- `tradar-core` depends on nothing internal to the workspace.
- `tradar-query-workbench` depends only on `tradar-core`.
- Each connector crate depends on `tradar-core` always, and on `tradar-query-workbench` only if it is query-shaped (Postgres, SQLite, Mongo, Elasticsearch, Redis today; Cassandra later). Kafka, RabbitMQ, and other non-query connectors depend on `tradar-core` only and build their own `Session`/`Component` implementations directly.
- `tradar-app` is the only crate that depends on every connector crate. No connector crate depends on another, and no connector crate depends on `tradar-app`.

Non-binding guidance: as a connector crate grows past connection setup + execution + schema listing (completion, formatting, explain plans), split it internally into `client/`, `executor/`, `metadata/` submodules. Reach for this when a crate actually outgrows one file's worth of clarity — not scaffolding to pre-create now.

### Connector, Session, and Screen

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;
    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>>;
}

pub trait Session: Send + Sync {
    /// Drains whatever internal channel this session's background tasks report
    /// through, updating its own state. Bounded per call — see "Screen never
    /// does IO" below.
    fn tick(&mut self);

    fn build_screen(self: Box<Self>, action_tx: UnboundedSender<Action>) -> Box<dyn Component>;
}
```

- **Connector**: stateless-ish factory. Given a `SavedConnection`, produces a `Session`. The only stage that does the initial handshake (open a TCP connection, authenticate, ping).
- **Session**: the long-lived actor. Owns everything that touches IO or outlives a single render frame — the connection/client handle, any background tasks it spawns, an internal channel those tasks report back through, and caches (schema, topic metadata, mapping info). A `KafkaSession` owns its consumer, producer, and offset tracking; a `MongoSession` owns its client and collection cache. **A `Session` is the only thing in this pipeline allowed to spawn tasks or own a channel.**
- **Screen**: what `RootComponent` actually holds and routes keys/draws to — a value implementing `Component`. It reads its `Session`'s state to render and turns key events into synchronous command calls on the `Session` (e.g. `session.submit_query(text)`, `session.publish(topic, payload)`). It never touches a socket, a file, or `tokio::spawn` directly.

Query-shaped connectors: `QueryEngine` (in `tradar-query-workbench`) implements `Session` — its `tick()` drains query-completion replies, its `build_screen()` returns a `QueryScreenComponent`. This is a rename-free fit; `QueryEngine` already plays this role today, it just gets a formal trait.

No new trait is needed for the Screen/Component/widget distinction — it names a pattern the code already uses. `QueryScreenComponent` is a Screen (implements `Component`), but internally composes `query_editor.rs`, `results.rs`, and `schema_sidebar.rs`, none of which implement `Component` themselves — plain state+draw structs the screen owns and calls directly. Every future connector follows the same pattern: `KafkaScreen` implements `Component` and composes plain `TopicList`/`MessageTable`/`Header` structs as it sees fit.

### Screen never does IO — Session is the actor

A Screen must never call `tokio::spawn` or own a channel directly. If it did, every connector's UI code would end up interleaved with its IO/business logic — the same coupling the driver-isolation rule exists to prevent, just moved up a layer.

1. A key press or `update()` call on a Screen turns into a **synchronous** method call on its Session (e.g. `self.session.submit_query(text)`), returning immediately.
2. That Session method calls `tokio::spawn` for whatever IO the command requires, handing the spawned task a `Sender` half of a channel the Session owns.
3. The event loop calls `Component::tick()` on the active screen every iteration (new trait method, default no-op); a Screen's `tick()` forwards to `self.session.tick()`.
4. `Session::tick()` drains its own internal channel — **with a budget** (e.g. at most 64 messages per call) — updating its own state. It never blocks.
5. The Screen's next `draw()` call renders whatever state the Session now holds.

```rust
pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn tick(&mut self) {}
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
```

The budget matters once a connector is a genuine firehose — a Kafka consumer at thousands of messages/second, an Elasticsearch tail, a Prometheus scrape loop. An unbounded `while let Ok(msg) = rx.try_recv()` would starve rendering; draining a fixed number per tick and picking up the rest next tick keeps the UI responsive regardless of producer throughput — the same technique game engines and GUI frameworks (iced, egui) use for their event queues.

This is why no `Action::Plugin(Box<dyn Any>)` (or equivalent type-erased variant) exists: it would still require `RootComponent` to route a type-erased payload to the active screen, losing type safety and debuggability exactly where they matter most — inside a connector's own message handling. With each Session owning a private channel of its own concrete message type, no connector-internal message ever crosses a type-erased boundary, and `tradar-core`'s `Action` enum never needs a catch-all variant or a new variant when a connector is added.

One exception: the **initial** connect, since a Session doesn't exist yet to spawn it. `tradar-app`'s `main.rs` spawns the `Connector::connect(...).await` call itself (descendant of today's `spawn_connect`); once that resolves to a Session, all further task-spawning for that connection is the Session's job.

### ConnectorDescriptor and Capability

```rust
pub enum Capability {
    Query,
    Schema,
    Streaming,
    Publish,
    Tail,
    Explain,
    Export,
}

pub struct ConnectorDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub capabilities: &'static [Capability],
}
```

Lets a connection picker or a screen reason about what a connector *can do* without hardcoding its identity. Examples: Postgres/SQLite declare `[Query, Schema, Explain, Export]`; Redis declares `[Query, Streaming]`; a future Kafka would declare `[Streaming, Publish, Tail]`. Nothing consumes `Capability` yet — no UI branches on it — it's defined now because retrofitting it after several connectors exist is far more disruptive than defining the shape up front, and it costs nothing to leave unused.

### Registry

- `SavedConnection.driver` changes from the closed `DriverKind` enum to a `String` connector id (e.g. `"postgres"`, `"kafka"`), matched against `ConnectorDescriptor::id`. Neither `tradar-core` nor any connector crate needs to enumerate the full connector list.
- Each connector crate exports a constructor, e.g. `pub fn connector() -> Box<dyn Connector>`.
- `tradar-app`'s `main.rs` is the only place that knows the full set of connectors: it builds a `HashMap<String, Box<dyn Connector>>` at startup by calling each connector crate's constructor. Adding a connector means one `Cargo.toml` dependency line and one registration line in `main.rs` — no change to `tradar-core`, `tradar-query-workbench`, or any other connector crate.
- An unmatched `connection.driver` id is a runtime error surfaced to the user (e.g. `"unknown connector 'kafka': not compiled into this build"`), not a compile-time enum-exhaustiveness error.

### RootComponent and Action

`RootComponent`'s hardcoded `query_screen: QueryScreenComponent` field becomes:

```rust
enum ScreenSlot {
    ConnectionPicker,
    Active(Box<dyn Component>),
}
```

`Action` shrinks to exactly the application-level events core needs, renamed `Connect*` → `Open*` (not everything is a "connection" the way Postgres/Kafka are — an SSH host or Docker daemon fit "open a screen for this target" better):

```rust
pub enum Action {
    Quit,
    OpenRequested { connection: SavedConnection, epoch: u64 },
    Opened { connection: SavedConnection, screen: Box<dyn Component>, epoch: u64 },
    OpenFailed { error: String, epoch: u64 },
    BackToPicker,
}
```

This enum is closed and stays closed — no connector ever adds a variant, since connector-internal messages never cross this boundary. `tradar-core` no longer needs to know what `QueryEngine`, `SchemaInfo`, or any connector-specific type is; `Opened` only carries the already-built Screen. `SavedConnection`/`ConnectionStore` keep their current names — "a saved way to reach a target" is still accurate even for SSH/Docker/Kubernetes.

**Side effect:** today's `Action::ExportCurl` is a shared-enum variant only Elasticsearch implements, forcing `main.rs` to special-case it. Under the new model, curl export becomes `session.export_curl(query)` — a synchronous call handled entirely inside `tradar-elasticsearch`'s own Session/Screen — removing a pre-existing leak of connector-specific logic into shared code.

### Big picture

```
App
 ├── Registry        (Connector id -> Connector, built once at startup in tradar-app)
 ├── Navigation      (RootComponent: ConnectionPicker <-> active Screen)
 └── Screen                             — renders, dispatches commands
      │
      ▼
    Session                             — the actor; owns IO
      ├── Connection / client handle
      ├── Background tasks it spawned
      ├── Internal channel (bounded, budgeted drain in tick())
      └── Cache (schema, topics, mappings, ...)
```

Example flow (submit a query):

```
key press → QueryScreen.handle_key_event → QueryScreen.update() calls
session.submit_query(text) directly (sync call, returns immediately)
  → Session spawns a task → task awaits the driver → task sends the result into Session's channel
  → next event-loop tick: RootComponent.tick() → QueryScreen.tick() → Session.tick() drains (budgeted)
  → Session's state now holds the result → QueryScreen.draw() renders it
```

### Considered and deferred

Raised during design review, deliberately left out of the target shape above until a concrete connector actually needs them — each has a trigger condition for revisiting, the same way the v1 spec deferred the workspace split itself:

- **Session split into Runtime/Store/Client sub-components.** Revisit if a single connector's `Session` impl (e.g. Kafka's) grows large enough that its responsibilities (connection, cache, task/channel plumbing) become hard to navigate in one file.
- **`Arc<dyn Session>` / `SessionHandle` instead of `build_screen(self: Box<Self>)`.** The current design gives the Screen sole ownership of its Session. Revisit only when something needs to share one Session across multiple Screens — e.g. two tabs against the same connection, or reconnect-and-reuse.
- **`tick(cx: &mut Context)` carrying delta/now/frame instead of `tick(&mut self)`.** Revisit when a connector actually needs timers, animation, or debounce logic that depends on wall-clock/frame info.
- **Explicit lifecycle hooks (`on_open`, `on_close`, `suspend`, `resume`, `dispose`).** Revisit when a connector holds a resource that must be shut down deterministically rather than on `Drop` — e.g. a Kafka consumer that should leave its group cleanly.
- **`Capability` as bitflags instead of a plain enum.** Revisit if the variant count grows large enough (a few dozen) that a `&'static [Capability]` slice becomes unwieldy compared to a flags type.
- **Command pattern (`enum Command` sent from Screen to Session) instead of direct synchronous method calls.** Revisit if a concrete feature needs to intercept/replay commands — undo/redo, macros, session recording — none of which are planned today.
- **`Workspace → Tab → Screen` instead of a single `ScreenSlot::Active(Box<dyn Component>)`.** Revisit when multi-tab or split-view (e.g. two connections open side by side) becomes an actual planned feature — it's on the backlog as "Sessions/workspace state" but not yet scoped.
- **`ConnectorFactory` instead of a `Connector` trait object directly in the registry.** Revisit if a connector ever needs non-singleton construction (e.g. per-tab instances with different config), which nothing today requires.

## Non-goals of the target architecture

- Implementing Kafka, RabbitMQ, Cassandra, or any other new connector — this defines the shape they'll be built into.
- Dynamic plugin loading (`.so`/`.wasm`, third-party plugin distribution).
- An interactive "add connection" UI (still hand-edited TOML).
- Any UI actually branching on `Capability` — the enum and descriptor shape are defined now; consuming them is future work.
- Any change to the vim-modal query editor work beyond `QueryScreenComponent` moving into `tradar-query-workbench`.
