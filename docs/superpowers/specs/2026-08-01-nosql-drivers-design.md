# MongoDB, Elasticsearch, and Redis Drivers

Status: Approved
Date: 2026-08-01

## Context

The v1 walking skeleton (`docs/superpowers/specs/2026-08-01-tradar-v1-design.md`) shipped with two `Driver` implementations — PostgreSQL and SQLite — both SQL, both naturally tabular. That spec explicitly deferred MongoDB, Elasticsearch, and Redis as non-goals.

This spec covers adding those three. They break an assumption baked into the current code: `QueryResult` is `{ columns: Vec<String>, rows: Vec<Vec<String>> }`, which fits SQL but not MongoDB documents, Elasticsearch hits, or Redis replies (strings, integers, arrays, nil). Accommodating that is the main design work here, not just writing three more driver modules.

## Goals

1. Redesign `QueryResult` as an enum with a tabular variant (existing SQL drivers) and a document variant (the three new drivers).
2. Implement `ElasticsearchDriver`: Search API only, against a fixed index encoded in the connection target.
3. Implement `RedisDriver`: single command-line execution, naive whitespace parsing.
4. Implement `MongoDriver`: a minimal shell-subset parser for `db.<collection>.<method>(<json-args>)`, not a real JS engine — supporting the standard CRUD method set.
5. Update every existing call site that constructs or matches the old `QueryResult` struct shape (`drivers::{postgres,sqlite}`, `app`, `query_engine`'s test fake, `tui`'s result renderer) so the whole crate compiles and passes against the new enum.

## Non-goals (explicitly deferred)

- TUI wiring for the new drivers beyond a results renderer (no connection-picker changes, no schema explorer for any driver — schema browsing isn't wired into the TUI for any driver yet, per the v1 spec).
- Elasticsearch: `_count`, `_explain`, bulk, update/delete-by-query, aliases/templates/mappings/settings browsing.
- Redis: pipelining, transactions (`MULTI`/`EXEC`), quoted/escaped arguments in the command line, pub/sub.
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

## Elasticsearch driver

- Connection target: a base URL including the index, e.g. `http://localhost:9200/my-index` (mirrors how Postgres's target is a full connection string).
- `connect()`: no persistent connection needed (HTTP is stateless) — a no-op that optionally pings `{target}` to fail fast on a bad host, consistent with the other drivers' "connect must succeed before execute" contract.
- `execute(query)`: POSTs `query` as the JSON body to `{target}/_search`, returns hits as `Documents`.
- `list_schema()`: returns index-level info only for this pass (e.g. a single `SchemaInfo` for the configured index) — full aliases/templates/mappings browsing is a non-goal here.
- New dependency: none (`reqwest` with the `json` feature is already present).

## Redis driver

- Connection target: a `redis://host:port[/db]` URL.
- `connect()`: opens a connection via the `redis` crate's async multiplexed connection.
- `execute(query)`: splits `query` on whitespace into command + args (no quoting support), sends via `redis::cmd`, converts the `redis::Value` reply to `serde_json::Value`, returns `Documents(vec![value])`.
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

## Documentation deliverables

- `README.md`: move MongoDB/Elasticsearch/Redis from "Planned" to "v1 target" (alongside Postgres/SQLite), with a one-line note on each driver's execution model (native JS-shell subset, Query DSL against Search API, Redis command line).
- `docs/architecture.md`: document the `QueryResult` enum (replacing the old struct description), and add each new driver to the module layout list.

## Open questions for the implementation plan

- Exact `testcontainers-modules` module names/versions for Elasticsearch and MongoDB (Redis's existence was not verified during brainstorming) — confirm during implementation, fall back to `GenericImage` if a dedicated module isn't available at the pinned `testcontainers-modules` version.
- Exact BSON-to-`serde_json::Value` conversion path for MongoDB documents (likely `bson::Bson::into_relaxed_extjson()` or similar — confirm against the pinned `mongodb`/`bson` crate versions during implementation).
- Elasticsearch `connect()`'s fail-fast ping: exact endpoint (e.g. cluster health `/_cluster/health` vs. a simple `HEAD {target}`) — confirm during implementation.
