//! The headless host: owns live connections and result cursors, and answers
//! JSON-RPC requests. `Server::handle` is transport-free (a `Value` in, a
//! `Value` out) so every method is testable without a socket; `transport`
//! is the thin unix-socket shell around it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, mpsc};

use serde_json::{Value, json};

use tradar_connector_spi::Connector;
use tradar_core::action::CrudOp;
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_query_workbench::components::completion::{CandidateKind, CompletionSource};
use tradar_query_workbench::query_driver::{QueryDriver, completion_context};
use tradar_query_workbench::query_engine::QueryEngine;

use crate::protocol::{
    METHOD_NOT_FOUND, RpcError, StoredResult, parse_row_edit, schema_json, statement_json,
};

/// Rows returned inline by `execute` -- enough to fill a screen, so the
/// common case is one round trip; the rest comes through `fetch`.
const DEFAULT_PAGE: usize = 200;

/// Cursors kept per server. A result can hold `MAX_ROWS` rows, so an
/// unbounded map of them is the memory failure `MAX_ROWS` exists to prevent
/// -- oldest is evicted first.
const MAX_CURSORS: usize = 16;

#[derive(Default)]
struct State {
    drivers: HashMap<String, Arc<dyn QueryDriver>>,
    /// Built once per connection (keywords + schema) and rebuilt whenever
    /// `schema` is called, the same lifetime the TUI gives its own
    /// `CompletionSource` -- never per keystroke.
    completions: HashMap<String, Arc<CompletionSource>>,
    cursors: HashMap<u64, StoredResult>,
    cursor_order: VecDeque<u64>,
    next_cursor: u64,
    /// In-flight `execute`s that a client named with `query_id`, so a
    /// `cancel` from the same (or another) connection can reach them --
    /// the `Notify` to drop the client's own wait, plus the driver to fire
    /// a real DB-side `cancel_query` against (`cancel` needs both: which
    /// future to stop waiting on, and which connection to tell the
    /// database about).
    running: HashMap<String, (Arc<Notify>, Arc<dyn QueryDriver>)>,
    /// gRPC connections, kept apart from `drivers` -- gRPC has no
    /// `QueryDriver` to serve (see `grpc.rs`'s doc comment), so it needs its
    /// own channel + schema state instead of fitting the `Arc<dyn
    /// QueryDriver>` shape every other connection here uses.
    #[cfg(feature = "grpc")]
    grpc: HashMap<String, crate::grpc::GrpcConnection>,
    /// In-flight `grpc.invoke`s named with `query_id`, mirroring `running`
    /// above -- a separate map because gRPC has no `Arc<dyn QueryDriver>`
    /// to carry alongside the `Notify` (and no DB-side "stop processing"
    /// to call either; cancelling only drops the client's own wait, same
    /// ceiling `QueryDriver::cancel_query` already has for most drivers).
    #[cfg(feature = "grpc")]
    grpc_running: HashMap<String, Arc<Notify>>,
    /// Kafka tail subscriptions (`kafka.tail.start`/`kafka.tail.stop`),
    /// keyed by the subscription id the client named. See `kafka.rs`'s
    /// doc comment for why this doesn't go through `drivers` either.
    #[cfg(feature = "kafka")]
    kafka: HashMap<String, crate::kafka::KafkaConnection>,
    /// `(owning client id, the two tasks)` -- the id is what lets
    /// `kafka_tail_stop`/`kafka_forget_client` also remove this
    /// subscription from `kafka_tails_by_client` without a linear scan.
    #[cfg(feature = "kafka")]
    kafka_tails: HashMap<String, (u64, crate::kafka::TailHandle)>,
    /// Which subscription ids belong to which `ClientConnection` -- the
    /// only reason `ClientConnection::id` exists at all: when that socket
    /// closes, every tail it started (and forgot to `kafka.tail.stop`)
    /// must be aborted too, or an orphaned consumer runs forever for a
    /// client nobody is listening on anymore.
    #[cfg(feature = "kafka")]
    kafka_tails_by_client: HashMap<u64, Vec<String>>,
    #[cfg(feature = "kafka")]
    next_kafka_subscription: u64,
}

/// One live client socket (`transport.rs`'s `serve_connection`) -- *not* a
/// saved database connection (`connect`'s own `connection` param); the two
/// are unrelated concepts that happen to share the English word. Exists so
/// a background push subscription (Kafka tail, and whatever reuses the
/// same mechanism later -- Socket, see
/// `docs/backlog/push-channel-design-2026-10-14.md`) started by one client
/// can push notification lines onto *that* client's own socket, and be
/// torn down when *that* client disconnects, without touching another
/// client's subscriptions on the same server.
// No push-capable connector is compiled in without `kafka` (Socket tail
// is designed, per docs/backlog/push-channel-design-2026-10-14.md, but not
// yet built) -- `id`/`outbox`/`notify` go unused in that build, same as any
// other build with every push connector's feature off. Still part of the
// public shape: `new_client`/`client_disconnected` take/use `ClientConnection`
// unconditionally so the generic plumbing doesn't itself need feature gates.
#[cfg_attr(not(feature = "kafka"), allow(dead_code))]
#[derive(Clone)]
pub struct ClientConnection {
    id: u64,
    /// A notification is a JSON-RPC object with no `id` -- same framing,
    /// same per-connection channel `transport.rs` already had for replies,
    /// just sent without a request ever asking for it.
    outbox: mpsc::UnboundedSender<String>,
}

impl ClientConnection {
    /// A connection with nowhere to push notifications -- for a caller
    /// (every existing test, or anything else driving `Server::handle`
    /// without a real socket behind it) that never starts a subscription.
    /// Pushes sent to it are silently dropped (the receiver is dropped
    /// immediately); `id` is shared across every detached connection,
    /// which is fine since nothing here ever disconnects one by id.
    pub fn detached() -> Self {
        let (outbox, _unused) = mpsc::unbounded_channel();
        Self { id: 0, outbox }
    }

    #[cfg_attr(not(feature = "kafka"), allow(dead_code))]
    pub(crate) fn notify(&self, method: &str, params: Value) {
        let line = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        let _ = self.outbox.send(format!("{line}\n"));
    }
}

pub struct Server {
    registry: HashMap<String, Box<dyn Connector>>,
    store: Option<ConnectionStore>,
    state: Mutex<State>,
    /// Fired by the `shutdown` method; `main` waits on it next to Ctrl-C.
    shutdown: Arc<Notify>,
    next_client_id: AtomicU64,
}

impl Server {
    pub fn new(
        registry: HashMap<String, Box<dyn Connector>>,
        store: Option<ConnectionStore>,
    ) -> Self {
        Self {
            registry,
            store,
            state: Mutex::new(State::default()),
            shutdown: Arc::new(Notify::new()),
            next_client_id: AtomicU64::new(1),
        }
    }

    /// Completes when a client asks the server to stop (`shutdown`) -- how a
    /// rebuilt binary replaces a long-running old one without `kill`.
    pub fn shutdown_signal(&self) -> Arc<Notify> {
        Arc::clone(&self.shutdown)
    }

    /// One `ClientConnection` per accepted socket (`transport.rs`'s
    /// `serve_connection`) -- `outbox` is that connection's own reply
    /// channel, reused here so a push notification interleaves with
    /// ordinary replies on the same line stream.
    pub fn new_client(&self, outbox: mpsc::UnboundedSender<String>) -> ClientConnection {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        ClientConnection { id, outbox }
    }

    /// Call when a client's socket closes: aborts and forgets every
    /// background subscription it started and never explicitly stopped.
    #[cfg_attr(not(feature = "kafka"), allow(unused_variables))]
    pub fn client_disconnected(&self, client: &ClientConnection) {
        #[cfg(feature = "kafka")]
        self.kafka_forget_client(client.id);
    }

    /// One request object in, one response object out (JSON-RPC 2.0, minus
    /// batches and notifications -- every call here wants an answer).
    pub async fn handle(&self, request: Value, client: &ClientConnection) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let outcome = match request.get("method").and_then(Value::as_str) {
            None => Err(RpcError::invalid("missing `method`")),
            Some(method) => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                self.dispatch(method, &params, client).await
            }
        };
        match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": error.code, "message": error.message, "data": error.data},
            }),
        }
    }

    #[cfg_attr(not(feature = "kafka"), allow(unused_variables))]
    async fn dispatch(
        &self,
        method: &str,
        params: &Value,
        client: &ClientConnection,
    ) -> Result<Value, RpcError> {
        match method {
            "connectors.list" => Ok(self.connectors()),
            "connections.list" => self.connections(),
            "connect" => self.connect(params).await,
            "disconnect" => self.disconnect(params),
            "status" => self.status(params).await,
            "schema" => self.schema(params).await,
            "execute" => self.execute(params).await,
            "cancel" => self.cancel(params).await,
            "shutdown" => {
                self.shutdown.notify_one();
                Ok(json!({"ok": true}))
            }
            "fetch" => self.fetch(params),
            "cursor.close" => self.close_cursor(params),
            "split" => self.split(params),
            "keywords" => self.keywords(params),
            "snippet" => self.snippet(params).await,
            "complete" => self.complete(params),
            "edit.source" => self.edit_source(params),
            "edit.sql" => self.edit_sql(params),
            #[cfg(feature = "grpc")]
            "grpc.connect" => self.grpc_connect(params).await,
            #[cfg(feature = "grpc")]
            "grpc.disconnect" => self.grpc_disconnect(params),
            #[cfg(feature = "grpc")]
            "grpc.services" => self.grpc_services(params),
            #[cfg(feature = "grpc")]
            "grpc.load_proto" => self.grpc_load_proto(params),
            #[cfg(feature = "grpc")]
            "grpc.methods" => self.grpc_methods(params),
            #[cfg(feature = "grpc")]
            "grpc.invoke" => self.grpc_invoke(params).await,
            #[cfg(feature = "kafka")]
            "kafka.connect" => self.kafka_connect(params).await,
            #[cfg(feature = "kafka")]
            "kafka.disconnect" => self.kafka_disconnect(params),
            #[cfg(feature = "kafka")]
            "kafka.topics" => self.kafka_topics(params).await,
            #[cfg(feature = "kafka")]
            "kafka.tail.start" => self.kafka_tail_start(params, client),
            #[cfg(feature = "kafka")]
            "kafka.tail.stop" => self.kafka_tail_stop(params),
            _ => Err(RpcError {
                code: METHOD_NOT_FOUND,
                message: format!("unknown method `{method}`"),
                data: None,
            }),
        }
    }

    fn connectors(&self) -> Value {
        let mut ids: Vec<&str> = self.registry.keys().map(String::as_str).collect();
        ids.sort_unstable();
        json!(ids)
    }

    fn saved(&self) -> Result<Vec<SavedConnection>, RpcError> {
        match &self.store {
            Some(store) => Ok(store.load()?),
            None => Ok(Vec::new()),
        }
    }

    fn connections(&self) -> Result<Value, RpcError> {
        let connected = self.state.lock().unwrap();
        let list: Vec<Value> = self
            .saved()?
            .into_iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "driver": c.driver,
                    "target": c.target,
                    "connected": connected.drivers.contains_key(&c.name)
                        || connected_grpc(&connected, &c.name)
                        || connected_kafka(&connected, &c.name),
                    "supported": self.is_supported(&c.driver),
                })
            })
            .collect();
        Ok(json!(list))
    }

    /// Whether this server build can serve `driver` at all -- through the
    /// `Connector`/`QueryDriver` registry, or (gRPC/Kafka) through their
    /// own `grpc.*`/`kafka.*` methods instead.
    fn is_supported(&self, driver: &str) -> bool {
        self.registry.contains_key(driver)
            || (driver == "grpc" && cfg!(feature = "grpc"))
            || (driver == "kafka" && cfg!(feature = "kafka"))
    }

    fn driver(&self, params: &Value) -> Result<Arc<dyn QueryDriver>, RpcError> {
        let name = str_param(params, "connection")?;
        self.state
            .lock()
            .unwrap()
            .drivers
            .get(name)
            .cloned()
            .ok_or_else(|| {
                RpcError::app(format!("`{name}` is not connected -- call `connect` first"))
            })
    }

    async fn connect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        if self.state.lock().unwrap().drivers.contains_key(name) {
            return Ok(json!({"connected": true, "already": true}));
        }
        let saved = self
            .saved()?
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| RpcError::app(format!("no saved connection named `{name}`")))?;
        let connector = self.registry.get(&saved.driver).ok_or_else(|| {
            RpcError::app(format!(
                "this server build has no `{}` connector (or it has no query language to serve)",
                saved.driver
            ))
        })?;
        let session = connector.connect(saved).await?;
        let driver = session
            .as_any()
            .and_then(|any| any.downcast_ref::<QueryEngine>())
            .map(QueryEngine::driver)
            .ok_or_else(|| RpcError::app("this connector has no query driver to serve"))?;
        // A failed schema load still connects, as in the TUI: completion
        // just has no schema names to offer.
        let schema = driver.list_schema().await.unwrap_or_default();
        let source = Arc::new(CompletionSource::new(driver.keywords(), &schema));
        let mut state = self.state.lock().unwrap();
        state.drivers.insert(name.to_string(), driver);
        state.completions.insert(name.to_string(), source);
        Ok(json!({"connected": true, "already": false}))
    }

    fn disconnect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let mut state = self.state.lock().unwrap();
        state.completions.remove(name);
        let removed = state.drivers.remove(name).is_some();
        Ok(json!({"disconnected": removed}))
    }

    async fn status(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let alive = driver.ping().await.is_ok();
        Ok(json!({"alive": alive, "in_transaction": driver.in_transaction()}))
    }

    async fn schema(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let entries = driver.list_schema().await?;
        let source = Arc::new(CompletionSource::new(driver.keywords(), &entries));
        self.state
            .lock()
            .unwrap()
            .completions
            .insert(str_param(params, "connection")?.to_string(), source);
        Ok(json!(entries.iter().map(schema_json).collect::<Vec<_>>()))
    }

    async fn execute(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let query = str_param(params, "query")?;
        let page_size = usize_param(params, "page_size")?.unwrap_or(DEFAULT_PAGE);

        // The lock is not held across this await: a slow query on one
        // connection must not block another client's `fetch`. A `cancel`
        // for this `query_id` drops the in-flight future *and* fires the
        // driver's own `cancel_query` -- see `cancel` below.
        let query_id = params.get("query_id").and_then(Value::as_str);
        let cancel = Arc::new(Notify::new());
        if let Some(id) = query_id {
            self.state
                .lock()
                .unwrap()
                .running
                .insert(id.to_string(), (Arc::clone(&cancel), Arc::clone(&driver)));
        }
        let outcome = tokio::select! {
            result = driver.execute(query) => Some(result),
            () = cancel.notified() => None,
        };
        if let Some(id) = query_id {
            self.state.lock().unwrap().running.remove(id);
        }
        let result = match outcome {
            Some(result) => result.map_err(|e| RpcError::from_query_error(e.to_string()))?,
            None => return Err(RpcError::app("query cancelled")),
        };
        if is_ddl(query) {
            self.refresh_completions(params, &driver).await;
        }
        let stored = match StoredResult::from_query(result) {
            Ok(stored) => stored,
            Err(rows) => return Ok(json!({"kind": "affected", "rows": rows})),
        };

        let first_page = stored.page(0, page_size);
        let mut response = json!({
            "kind": stored.kind(),
            "columns": stored.columns(),
            "total": stored.total(),
            "truncated": stored.truncated(),
            "rows": first_page,
        });
        if let Some(field_order) = stored.field_order(0, page_size) {
            response["field_order"] = field_order;
        }
        let mut state = self.state.lock().unwrap();
        state.next_cursor += 1;
        let cursor = state.next_cursor;
        state.cursors.insert(cursor, stored);
        state.cursor_order.push_back(cursor);
        while state.cursor_order.len() > MAX_CURSORS {
            if let Some(oldest) = state.cursor_order.pop_front() {
                state.cursors.remove(&oldest);
            }
        }
        response["cursor"] = json!(cursor);
        Ok(response)
    }

    /// A skeleton statement in the driver's *own* language for a table --
    /// `SELECT ... LIMIT` for SQL, `find()` for Mongo, a `_search` for
    /// Elasticsearch -- so "open this table" never has to know which it is.
    /// `schema` disambiguates same-named tables (Postgres schemas).
    async fn snippet(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let name = str_param(params, "name")?;
        let op = match params.get("op").and_then(Value::as_str).unwrap_or("read") {
            "create" => CrudOp::Create,
            "read" => CrudOp::Read,
            "update" => CrudOp::Update,
            "delete" => CrudOp::Delete,
            other => return Err(RpcError::invalid(format!("unknown op `{other}`"))),
        };
        let schema = params.get("schema").and_then(Value::as_str);
        let columns: Vec<String> = params
            .get("columns")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let entries = driver.list_schema().await?;
        let entry = entries
            .iter()
            .find(|e| e.name == name && (schema.is_none() || e.schema.as_deref() == schema))
            .ok_or_else(|| RpcError::app(format!("no table `{name}` in the schema")))?;
        Ok(json!({"text": driver.crud_snippet(entry, op, &columns)}))
    }

    async fn cancel(&self, params: &Value) -> Result<Value, RpcError> {
        let id = str_param(params, "query_id")?;
        let target = self.state.lock().unwrap().running.get(id).cloned();
        if let Some((notify, driver)) = target {
            notify.notify_one();
            // Best-effort; the client already got `cancelled: true` either
            // way -- see `QueryDriver::cancel_query`'s own doc comment for
            // why this never promises the statement actually stopped.
            let _ = driver.cancel_query().await;
            return Ok(json!({"cancelled": true}));
        }
        #[cfg(feature = "grpc")]
        {
            let target = self.state.lock().unwrap().grpc_running.get(id).cloned();
            if let Some(notify) = target {
                notify.notify_one();
                return Ok(json!({"cancelled": true}));
            }
        }
        Ok(json!({"cancelled": false}))
    }

    /// Rebuilds a connection's completion source from the live schema. A
    /// failed schema read keeps the old source rather than blanking it.
    async fn refresh_completions(&self, params: &Value, driver: &Arc<dyn QueryDriver>) {
        let Ok(name) = str_param(params, "connection") else {
            return;
        };
        if let Ok(schema) = driver.list_schema().await {
            let source = Arc::new(CompletionSource::new(driver.keywords(), &schema));
            self.state
                .lock()
                .unwrap()
                .completions
                .insert(name.to_string(), source);
        }
    }

    fn fetch(&self, params: &Value) -> Result<Value, RpcError> {
        let cursor = u64_param(params, "cursor")?;
        let offset = usize_param(params, "offset")?.unwrap_or(0);
        let limit = usize_param(params, "limit")?.unwrap_or(DEFAULT_PAGE);
        let state = self.state.lock().unwrap();
        let stored = state
            .cursors
            .get(&cursor)
            .ok_or_else(|| RpcError::app(format!("cursor {cursor} is gone (closed or evicted)")))?;
        let mut response = json!({
            "offset": offset,
            "total": stored.total(),
            "rows": stored.page(offset, limit),
        });
        if let Some(field_order) = stored.field_order(offset, limit) {
            response["field_order"] = field_order;
        }
        Ok(response)
    }

    fn close_cursor(&self, params: &Value) -> Result<Value, RpcError> {
        let cursor = u64_param(params, "cursor")?;
        let mut state = self.state.lock().unwrap();
        state.cursor_order.retain(|c| *c != cursor);
        Ok(json!({"closed": state.cursors.remove(&cursor).is_some()}))
    }

    fn split(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let text = str_param(params, "text")?;
        let statements = driver.split_statements(text);
        Ok(json!(
            statements.iter().map(statement_json).collect::<Vec<_>>()
        ))
    }

    fn keywords(&self, params: &Value) -> Result<Value, RpcError> {
        Ok(json!(self.driver(params)?.keywords()))
    }

    /// `text` is everything from the start of the buffer up to the cursor
    /// (context -- aliases, `JOIN`, a Mongo `db.<coll>.find({` -- needs the
    /// earlier lines, not just the current one). The partial word being
    /// typed is derived here, with the same word characters the TUI editor
    /// uses, so both clients complete identically.
    fn complete(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let text = str_param(params, "text")?;
        let source = self
            .state
            .lock()
            .unwrap()
            .completions
            .get(name)
            .cloned()
            .ok_or_else(|| {
                RpcError::app(format!("`{name}` is not connected -- call `connect` first"))
            })?;
        let prefix_start = text
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
            .last()
            .map_or(text.len(), |(i, _)| i);
        let prefix = &text[prefix_start..];
        let context = completion_context(text);
        let items: Vec<Value> = source
            .matches_in_context(prefix, &context)
            .into_iter()
            .map(|c| {
                let kind = match c.kind {
                    CandidateKind::Keyword => "keyword",
                    CandidateKind::Table => "table",
                    CandidateKind::Column => "column",
                };
                json!({"text": c.text, "kind": kind})
            })
            .collect();
        Ok(json!({"prefix": prefix, "items": items}))
    }

    fn edit_source(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let query = str_param(params, "query")?;
        let source = driver.edit_source(query);
        let keys = source.as_deref().and_then(|s| driver.edit_key_columns(s));
        // Only worth asking once the query isn't already one table -- a
        // `JOIN`-only question, and the caller (the results grid) is the
        // one place that already knows the result's own column names,
        // which this doesn't otherwise have a reason to track server-side.
        let column_sources = source.is_none().then(|| {
            let columns: Vec<String> = params
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            driver.column_sources(query, &columns)
        });
        Ok(json!({
            "table": source,
            "key_columns": keys,
            "column_sources": column_sources.flatten(),
        }))
    }

    /// Only *builds* the statement; running it is the client's explicit
    /// follow-up `execute`, which keeps the TUI's "show it, run it only
    /// after a `y`" rule instead of hiding a write inside this call.
    fn edit_sql(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let edit = parse_row_edit(params)?;
        Ok(json!({"sql": driver.edit_sql(&edit)}))
    }

    /// Opens the channel and tries server reflection; a server that
    /// doesn't implement it still connects (`grpc.services` just comes
    /// back empty until `grpc.load_proto` fills the pool by hand).
    #[cfg(feature = "grpc")]
    async fn grpc_connect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        if self.state.lock().unwrap().grpc.contains_key(name) {
            return Ok(json!({"connected": true, "already": true}));
        }
        let saved = self
            .saved()?
            .into_iter()
            .find(|c| c.name == name && c.driver == "grpc")
            .ok_or_else(|| RpcError::app(format!("no saved gRPC connection named `{name}`")))?;
        let channel = crate::grpc::connect(&saved.target).await.map_err(|e| {
            RpcError::app(format!("không kết nối được tới `{}`: {e}", saved.target))
        })?;
        let pool = crate::grpc::discover_via_reflection(channel.clone())
            .await
            .unwrap_or_default();
        self.state.lock().unwrap().grpc.insert(
            name.to_string(),
            crate::grpc::GrpcConnection { channel, pool },
        );
        Ok(json!({"connected": true, "already": false}))
    }

    #[cfg(feature = "grpc")]
    fn grpc_disconnect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let removed = self.state.lock().unwrap().grpc.remove(name).is_some();
        Ok(json!({"disconnected": removed}))
    }

    #[cfg(feature = "grpc")]
    fn grpc_services(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let state = self.state.lock().unwrap();
        let conn = state.grpc.get(name).ok_or_else(|| {
            RpcError::app(format!(
                "`{name}` is not connected -- call `grpc.connect` first"
            ))
        })?;
        Ok(json!(crate::grpc::list_services(&conn.pool)))
    }

    /// Fallback discovery for a server without reflection: parse a
    /// `.proto` file on disk and merge it into the connection's pool.
    #[cfg(feature = "grpc")]
    fn grpc_load_proto(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let path = str_param(params, "path")?;
        let mut state = self.state.lock().unwrap();
        let conn = state.grpc.get_mut(name).ok_or_else(|| {
            RpcError::app(format!(
                "`{name}` is not connected -- call `grpc.connect` first"
            ))
        })?;
        let added = crate::grpc::load_proto(&mut conn.pool, path)
            .map_err(|e| RpcError::app(format!("không parse được `{path}`: {e}")))?;
        Ok(json!({"added": added}))
    }

    #[cfg(feature = "grpc")]
    fn grpc_methods(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let service = str_param(params, "service")?;
        let state = self.state.lock().unwrap();
        let conn = state.grpc.get(name).ok_or_else(|| {
            RpcError::app(format!(
                "`{name}` is not connected -- call `grpc.connect` first"
            ))
        })?;
        let methods = crate::grpc::list_methods(&conn.pool, service)
            .map_err(|e| RpcError::app(e.to_string()))?;
        Ok(json!(
            methods
                .iter()
                .map(|m| json!({"name": m.name, "input": m.input, "output": m.output}))
                .collect::<Vec<_>>()
        ))
    }

    /// Only *invokes*; unlike `edit.sql` there is nothing to show-then-run
    /// here (a gRPC call isn't a statement the client could choose to run
    /// later) -- the "show it, run it only after `y`" rule is specific to
    /// row-edit's generated write statements, not to RPCs in general.
    ///
    /// An optional `query_id` (same idea as `execute`'s) lets a client
    /// `cancel` a slow invoke -- best-effort, same ceiling `cancel` already
    /// has for most `QueryDriver`s: it only drops this future's own wait,
    /// never anything server-side (there is no gRPC-level "stop processing
    /// this call", unlike a DB's own cancel).
    #[cfg(feature = "grpc")]
    async fn grpc_invoke(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let service = str_param(params, "service")?;
        let method = str_param(params, "method")?;
        let request = params.get("request").cloned().unwrap_or(json!({}));
        let (channel, pool) = {
            let state = self.state.lock().unwrap();
            let conn = state.grpc.get(name).ok_or_else(|| {
                RpcError::app(format!(
                    "`{name}` is not connected -- call `grpc.connect` first"
                ))
            })?;
            (conn.channel.clone(), conn.pool.clone())
        };

        let query_id = params.get("query_id").and_then(Value::as_str);
        let cancel = Arc::new(Notify::new());
        if let Some(id) = query_id {
            self.state
                .lock()
                .unwrap()
                .grpc_running
                .insert(id.to_string(), Arc::clone(&cancel));
        }
        let outcome = tokio::select! {
            result = crate::grpc::invoke(channel, &pool, service, method, request) => Some(result),
            () = cancel.notified() => None,
        };
        if let Some(id) = query_id {
            self.state.lock().unwrap().grpc_running.remove(id);
        }
        match outcome {
            Some(Ok(Ok(response))) => Ok(json!({"response": response})),
            Some(Ok(Err(status))) => Err(status.into()),
            Some(Err(e)) => Err(e.into()),
            None => Err(RpcError::app("invoke cancelled")),
        }
    }

    /// Opens the Kafka client and probes the broker's metadata once (fails
    /// loudly here, like `grpc.connect`'s channel dial, rather than
    /// silently connecting then only finding out on the first
    /// `kafka.topics`/`kafka.tail.start`).
    #[cfg(feature = "kafka")]
    async fn kafka_connect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        if self.state.lock().unwrap().kafka.contains_key(name) {
            return Ok(json!({"connected": true, "already": true}));
        }
        let saved = self
            .saved()?
            .into_iter()
            .find(|c| c.name == name && c.driver == "kafka")
            .ok_or_else(|| RpcError::app(format!("no saved kafka connection named `{name}`")))?;
        let conn = crate::kafka::connect(&saved.target).await.map_err(|e| {
            RpcError::app(format!("không kết nối được tới `{}`: {e}", saved.target))
        })?;
        self.state
            .lock()
            .unwrap()
            .kafka
            .insert(name.to_string(), conn);
        Ok(json!({"connected": true, "already": false}))
    }

    #[cfg(feature = "kafka")]
    fn kafka_disconnect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let removed = self.state.lock().unwrap().kafka.remove(name).is_some();
        Ok(json!({"disconnected": removed}))
    }

    #[cfg(feature = "kafka")]
    async fn kafka_topics(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let brokers = {
            let state = self.state.lock().unwrap();
            state
                .kafka
                .get(name)
                .ok_or_else(|| {
                    RpcError::app(format!(
                        "`{name}` is not connected -- call `kafka.connect` first"
                    ))
                })?
                .brokers
                .clone()
        };
        let topics = crate::kafka::list_topics(&brokers)
            .await
            .map_err(|e| RpcError::app(e.to_string()))?;
        Ok(json!(topics))
    }

    /// Starts a tail: one background subscription per call, never shared
    /// with another client tailing the same topic (see
    /// `docs/backlog/push-channel-design-2026-10-14.md`'s "mỗi subscription
    /// một task nền độc lập"). Returns immediately with a `subscription`
    /// id; messages arrive later as `kafka.tail.message` notifications on
    /// `client`'s own socket, not as part of this reply.
    #[cfg(feature = "kafka")]
    fn kafka_tail_start(
        &self,
        params: &Value,
        client: &ClientConnection,
    ) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let topic = str_param(params, "topic")?.to_string();
        let from_beginning = params.get("from").and_then(Value::as_str) == Some("earliest");

        let brokers = {
            let state = self.state.lock().unwrap();
            state
                .kafka
                .get(name)
                .ok_or_else(|| {
                    RpcError::app(format!(
                        "`{name}` is not connected -- call `kafka.connect` first"
                    ))
                })?
                .brokers
                .clone()
        };

        let subscription = {
            let mut state = self.state.lock().unwrap();
            state.next_kafka_subscription += 1;
            format!("kafka-{}", state.next_kafka_subscription)
        };

        let client_id = client.id;
        let push_client = client.clone();
        let handle = crate::kafka::start_tail(
            brokers,
            topic,
            from_beginning,
            subscription.clone(),
            move |item| push_client.notify("kafka.tail.message", item),
        );

        let mut state = self.state.lock().unwrap();
        state
            .kafka_tails
            .insert(subscription.clone(), (client_id, handle));
        state
            .kafka_tails_by_client
            .entry(client_id)
            .or_default()
            .push(subscription.clone());
        Ok(json!({"subscription": subscription}))
    }

    #[cfg(feature = "kafka")]
    fn kafka_tail_stop(&self, params: &Value) -> Result<Value, RpcError> {
        let subscription = str_param(params, "subscription")?;
        let mut state = self.state.lock().unwrap();
        let Some((client_id, handle)) = state.kafka_tails.remove(subscription) else {
            return Ok(json!({"stopped": false}));
        };
        if let Some(list) = state.kafka_tails_by_client.get_mut(&client_id) {
            list.retain(|s| s != subscription);
        }
        drop(state);
        handle.stop();
        Ok(json!({"stopped": true}))
    }

    /// Aborts and forgets every tail `client_id` started and never
    /// explicitly `kafka.tail.stop`ped -- called from
    /// `client_disconnected` when that socket closes.
    #[cfg(feature = "kafka")]
    fn kafka_forget_client(&self, client_id: u64) {
        let mut state = self.state.lock().unwrap();
        let Some(subs) = state.kafka_tails_by_client.remove(&client_id) else {
            return;
        };
        let handles: Vec<_> = subs
            .iter()
            .filter_map(|s| state.kafka_tails.remove(s))
            .collect();
        drop(state);
        for (_, handle) in handles {
            handle.stop();
        }
    }
}

/// `query` without the `--` line comments and `/* */` blocks that precede
/// its first real token -- a statement is usually sent with the comment
/// above it attached, and a leading-keyword check must not be fooled by one.
fn skip_leading_comments(query: &str) -> &str {
    let mut rest = query.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after
                .split_once('\n')
                .map_or("", |(_, tail)| tail)
                .trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after
                .split_once("*/")
                .map_or("", |(_, tail)| tail)
                .trim_start();
        } else {
            return rest;
        }
    }
}

/// Whether `query` changes the schema, so completion has to be rebuilt:
/// SQL's leading DDL verbs, plus the Mongo shell calls that add or drop a
/// collection (and, since an insert is what creates one, `insertOne/Many`). A leading-keyword heuristic like `returns_rows`, not a
/// parser -- a miss only means the next `schema` call picks the change up.
fn is_ddl(query: &str) -> bool {
    let lower = skip_leading_comments(query).to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or("");
    matches!(first, "create" | "alter" | "drop" | "rename")
        || lower.contains(".createcollection(")
        || lower.contains(".drop(")
        // An insert is how a Mongo collection (and new fields) come to exist.
        || lower.contains(".insertone(")
        || lower.contains(".insertmany(")
}

#[cfg(feature = "grpc")]
fn connected_grpc(state: &State, name: &str) -> bool {
    state.grpc.contains_key(name)
}

#[cfg(not(feature = "grpc"))]
fn connected_grpc(_state: &State, _name: &str) -> bool {
    false
}

#[cfg(feature = "kafka")]
fn connected_kafka(state: &State, name: &str) -> bool {
    state.kafka.contains_key(name)
}

#[cfg(not(feature = "kafka"))]
fn connected_kafka(_state: &State, _name: &str) -> bool {
    false
}

fn str_param<'a>(params: &'a Value, name: &str) -> Result<&'a str, RpcError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid(format!("`{name}` must be a string")))
}

fn u64_param(params: &Value, name: &str) -> Result<u64, RpcError> {
    params
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| RpcError::invalid(format!("`{name}` must be a non-negative integer")))
}

fn usize_param(params: &Value, name: &str) -> Result<Option<usize>, RpcError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| RpcError::invalid(format!("`{name}` must be a non-negative integer"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_is_recognised_behind_leading_comments() {
        assert!(is_ddl("CREATE TABLE t (id INT)"));
        assert!(is_ddl("  drop table t"));
        assert!(is_ddl("-- tradar: demo\nCREATE TABLE t (id INT)"));
        assert!(is_ddl(
            "/* make it */ -- and more\n  ALTER TABLE t ADD c INT"
        ));
        assert!(is_ddl("db.createCollection('x')"));
        assert!(is_ddl("db.users.insertMany([{a: 1}])"));
        assert!(!is_ddl("db.users.find({})"));
        assert!(!is_ddl("-- CREATE is only mentioned here\nSELECT 1"));
        assert!(!is_ddl("SELECT 1"));
        assert!(!is_ddl("-- only a comment"));
    }
}
