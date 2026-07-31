# MongoDB, Elasticsearch, and Redis Drivers

Status: Approved
Date: 2026-08-01

## Context

The v1 walking skeleton (`docs/superpowers/specs/2026-08-01-tradar-v1-design.md`) shipped with two `Driver` implementations — PostgreSQL and SQLite — both SQL, both naturally tabular. That spec explicitly deferred MongoDB, Elasticsearch, and Redis as non-goals.

This spec covers adding those three. They break an assumption baked into the current code: `QueryResult` is `{ columns: Vec<String>, rows: Vec<Vec<String>> }`, which fits SQL but not MongoDB documents, Elasticsearch hits, or Redis replies (strings, integers, arrays, nil). Accommodating that is the main design work here, not just writing three more driver modules.

## Goals

1. Redesign `QueryResult` as an enum with a tabular variant (existing SQL drivers) and a document variant (the three new drivers).
2. Upgrade the query input to multi-line editing, and change the run keybinding to Ctrl+Enter (plain Enter now inserts a newline) — for every driver, not just the new ones.
3. Implement `ElasticsearchDriver` as a Kibana Dev-Tools-style console: type any `METHOD /path` + JSON body, executed against the cluster as-is (not limited to Search API against a fixed index).
4. Add curl export: a keybinding on the query screen (Elasticsearch only) writes the current request as a `curl` command to `./tradar-query.sh`.
5. Implement `RedisDriver`: single command-line execution, naive whitespace parsing, with type-aware result formatting for common commands (hashes render as objects, sorted-set-with-scores renders as `{member, score}` pairs) rather than a flat RESP-to-JSON dump.
6. Implement `MongoDriver`: a minimal shell-subset parser for `db.<collection>.<method>(<json-args>)`, not a real JS engine — supporting the standard CRUD method set.
7. Update every existing call site that constructs or matches the old `QueryResult` struct shape (`drivers::{postgres,sqlite}`, `app`, `query_engine`'s test fake, `tui`'s result renderer) so the whole crate compiles and passes against the new enum.

## Non-goals (explicitly deferred)

- TUI wiring for the new drivers beyond the query/results screen (no connection-picker changes, no schema explorer for any driver — schema browsing isn't wired into the TUI for any driver yet, per the v1 spec).
- Elasticsearch: authentication (basic auth/API keys), TLS client cert config, multi-request scripts (Kibana console supports multiple requests separated by blank lines — this pass is one request per execution).
- Curl export: Elasticsearch only, and only a fixed output path (`./tradar-query.sh`, overwritten each export) — no filename prompt, since that needs a text-input UI element that doesn't exist yet.
- Redis: pipelining, transactions (`MULTI`/`EXEC`), quoted/escaped arguments in the command line, pub/sub, streams (`XADD`/`XRANGE` get the generic conversion, not type-aware formatting — see Redis driver section for exactly which commands get special handling).
- MongoDB: any JS beyond the literal `db.<collection>.<method>(<args>)` shape — no `$where`, no chained methods (`.sort()`, `.limit()`), no arbitrary expressions as arguments. Bulk operations and transactions (explicitly listed as future work in the original product brief) are out of scope here too.

## `QueryResult` redesign

```rust
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

- `Table` is unchanged in spirit from today's struct; Postgres and SQLite switch from constructing `QueryResult { columns, rows }` to `QueryResult::Table { columns, rows }`.
- `Documents` is a single shape shared by all three new drivers rather than three bespoke ones:
  - Elasticsearch: each hit in the response becomes one `Value`.
  - MongoDB: each returned document becomes one `Value` (already JSON-like via BSON-to-JSON conversion).
  - Redis: the single reply converts to one `Value` in a one-element vec (RESP maps onto JSON naturally: bulk string → string, integer → number, array → array, nil → null, simple string → string).
- `tui`'s result renderer gets a second render path: `Documents` renders each value pretty-printed (via `serde_json::to_string_pretty`), one block per line group, instead of the table widget.
- This is a breaking change to existing tests, not just new code — `drivers::sqlite`/`drivers::postgres` tests, `app`'s `set_result`/`set_error` tests, `query_engine`'s `FakeDriver`, and `tui`'s rendering tests all construct the old shape and need updating to `QueryResult::Table { .. }`.

## Multi-line query input and the run keybinding

Currently `App::push_char`/`backspace` already operate on an arbitrary `String` — pushing `'\n'` and popping it back off already works with no App-layer changes needed. What changes:

- `main.rs`'s key handling: plain `KeyCode::Enter` now calls `app.push_char('\n')` instead of submitting; a new case, `KeyCode::Enter` with `KeyModifiers::CONTROL`, submits (calls the existing `run_query`). This applies uniformly to every driver's query screen, including SQL — SQL queries typically still fit one line, so this is a minor behavior change there (Enter no longer submits) rather than a new capability.
- `tui`'s query input `Paragraph` already renders embedded `\n` as multiple lines with no code change (ratatui's `Paragraph` splits on newlines natively) — this was verified against the ratatui 0.29 source during brainstorming.
- No change to `App`'s public API is needed for multi-line support itself.

## Elasticsearch driver

Modeled on Kibana's Dev Tools console rather than a fixed Search-only client:

- Connection target: the cluster base URL only, e.g. `http://localhost:9200` (no index baked in — a request's path carries that).
- Query input format: first line is `METHOD /path` (e.g. `GET my-index/_search`, `GET _cat/indices?v`, `PUT my-index/_doc/1`); remaining lines (if any) are the JSON request body, sent as-is. A missing body is valid for methods like `GET`/`DELETE` without one.
- `connect()`: pings the cluster (`GET {target}/`) to fail fast on a bad host — exact endpoint confirmed during implementation (cluster root vs. `/_cluster/health`).
- `execute(query)`: parses the method+path line and body per above, issues the HTTP request via `reqwest`, and wraps the full JSON response body as a single-element `Documents(vec![response_json])` — not unwrapped into per-hit documents, since arbitrary endpoints (not just `_search`) return arbitrary JSON shapes and a single "here's the response" document generalizes better than special-casing `_search`'s hit array.
- `list_schema()`: lists indices via `GET {target}/_cat/indices?format=json`.
- New dependency: none (`reqwest` with the `json` feature is already present).

### Curl export (Elasticsearch only)

- A new keybinding on the query screen, `y`, active only when the connected driver is Elasticsearch (no-op otherwise).
- Parses the current query input the same way `execute()` does (method + path + body) and builds a curl command: `curl -X {METHOD} "{base_url}{path}" -H 'Content-Type: application/json' -d '{body}'` (the `-H`/`-d` pair omitted when there's no body).
- This parsing-and-formatting step is a pure function in `drivers::elasticsearch` (e.g. `pub fn to_curl(base_url: &str, query: &str) -> Option<String>`), independently unit-testable without touching `main.rs` or the filesystem.
- `main.rs` calls it and writes the result (with a `#!/usr/bin/env bash` shebang line) to `./tradar-query.sh` in the current working directory, overwriting any existing file — this file write itself isn't unit-tested, consistent with how `main.rs`'s other I/O glue is handled.
- **`App` needs to know more about the active connection to support this.** Today `App::active_connection` is just `Option<String>` (the connection's name). To gate the `y` keybinding by driver kind and to know the Elasticsearch base URL for curl export, `active_connection` becomes `Option<SavedConnection>` (the whole saved connection, which already carries `driver: DriverKind` and `target: String`). This touches the existing `connect_to_selected`/`back_to_picker` tests in `app`, which currently assert on `active_connection.as_deref() == Some("name")` — they'll assert on the connection's `.name` field instead.

## Redis driver

- Connection target: a `redis://host:port[/db]` URL.
- `connect()`: opens a connection via the `redis` crate's async multiplexed connection.
- `execute(query)`: splits `query` on whitespace into command + args (no quoting support), sends via `redis::cmd`.
- Result conversion is type-aware for a bounded set of commands (matched case-insensitively on the command name), rather than one generic RESP-to-JSON dump:
  - `HGETALL` → the flat `[field1, value1, field2, value2, ...]` reply becomes a JSON object `{field1: value1, ...}`.
  - `ZRANGE`/`ZREVRANGE` with a trailing `WITHSCORES` argument → the flat `[member1, score1, member2, score2, ...]` reply becomes an array of `{"member": ..., "score": ...}` objects.
  - Every other command (including `LRANGE`, `SMEMBERS`, `GET`, `HGET`, `SADD`, plain `ZRANGE` without `WITHSCORES`, streams, etc.) uses the existing generic RESP-to-`serde_json::Value` conversion, which already renders arrays/strings/integers/nil sensibly — it's only the two flat-array-that-actually-means-pairs cases above that need special handling to avoid losing shape.
  - All of the above returns `Documents(vec![value])`.
- `list_schema()`: returns key names matching `SCAN` (bounded, e.g. first batch) as `SchemaInfo` entries — Redis has no fixed schema, so this is a best-effort key listing, not a structural schema.
- New dependency: `redis` crate with `tokio-comp` (async) feature.

## MongoDB driver

- Connection target: a `mongodb://host:port/db` URL with a default database.
- `connect()`: opens a connection via the official `mongodb` crate's async client.
- `execute(query)`: parses the literal text shape `db.<collection>.<method>(<json-args>)`:
  - Split off `db.`, read the collection name up to the next `.`.
  - Read the method name up to `(`.
  - Extract the balanced-parenthesis argument text, split on top-level commas (bracket-depth-aware, since arguments are JSON objects that may themselves contain commas) into individual JSON arguments.
  - Dispatch on method name to the matching `mongodb` crate driver call: `find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`.
  - Any other method name, or text that doesn't match the `db.<collection>.<method>(...)` shape, returns a clear "unsupported query" error rather than attempting to interpret it as JS.
- `list_schema()`: returns collection names via `list_collection_names`.
- New dependency: `mongodb` crate (official async driver, uses `tokio` runtime already in the project).

## Testing approach

Consistent with the existing drivers: real backends via `testcontainers-modules`, not mocks.

- Elasticsearch: `testcontainers-modules`' `elasticsearch` module (or `GenericImage` if no dedicated module exists at the pinned version — confirmed during implementation).
- Redis: `testcontainers-modules`' `redis` module.
- MongoDB: `testcontainers-modules`' `mongo` module.
- `QueryResult` refactor: existing driver/app/query_engine/tui tests are updated in place to construct `QueryResult::Table { .. }`; no new tests needed purely for the enum change beyond what already covers `Table`.
- Each new driver gets the same test shape used for Postgres/SQLite: connect succeeds, `list_schema` returns expected entries, `execute` returns expected `Documents` for a real read and, where applicable, a real write.
- MongoDB's shell-subset parser gets its own unit tests independent of a real database: valid `db.x.find({...})` parses into the right collection/method/args; malformed input and unsupported methods return errors — these don't need `testcontainers` since they test parsing, not execution.
- Elasticsearch's method+path+body parsing and `to_curl()` formatting get unit tests independent of a real cluster (e.g. `to_curl` with/without a body produces the right `curl` invocation); `execute`/`list_schema`/`connect` are tested against a real cluster as with the other drivers.
- Redis's type-aware conversion (`HGETALL` → object, `ZRANGE ... WITHSCORES` → array of pairs) is tested against a real Redis via `testcontainers-modules`, same as the rest of the driver — these aren't pure functions since they need a real reply to convert, but the assertions specifically check the shaped-JSON output, not just "didn't error."

## Documentation deliverables

- `README.md`: move MongoDB/Elasticsearch/Redis from "Planned" to "v1 target" (alongside Postgres/SQLite), with a one-line note on each driver's execution model (native JS-shell subset, Kibana-style console, Redis command line); document the new Ctrl+Enter-to-run / Enter-for-newline keybinding change (applies to all drivers); document curl export (`y` on the Elasticsearch query screen → `./tradar-query.sh`).
- `docs/architecture.md`: document the `QueryResult` enum (replacing the old struct description), and add each new driver to the module layout list.

## Open questions / risks for the implementation plan

- **Ctrl+Enter detection is a real risk, not just a detail.** Many terminal emulators don't send a distinguishable code for Ctrl+Enter versus plain Enter without an enhanced keyboard protocol (e.g. the Kitty keyboard protocol) enabled — crossterm supports enabling this (`PushKeyboardEnhancementFlags`), but not every terminal supports it, and behavior needs to be verified in the actual terminals this is used in (confirmed during implementation: query `crossterm::terminal::supports_keyboard_enhancement()` and pick a fallback key such as F5 or Ctrl+X if Ctrl+Enter isn't reliably available).
- Exact `testcontainers-modules` module names/versions for Elasticsearch and MongoDB (Redis's existence was not verified during brainstorming) — confirm during implementation, fall back to `GenericImage` if a dedicated module isn't available at the pinned `testcontainers-modules` version.
- Exact BSON-to-`serde_json::Value` conversion path for MongoDB documents (likely `bson::Bson::into_relaxed_extjson()` or similar — confirm against the pinned `mongodb`/`bson` crate versions during implementation).
- Elasticsearch `connect()`'s fail-fast ping: exact endpoint (e.g. cluster root `/` vs. `/_cluster/health`) — confirm during implementation.
