//! The headless host: owns live connections and result cursors, and answers
//! JSON-RPC requests. `Server::handle` is transport-free (a `Value` in, a
//! `Value` out) so every method is testable without a socket; `transport`
//! is the thin unix-socket shell around it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, mpsc};

use serde_json::{Value, json};

use tradar_core::storage::ConnectionStore;
use tradar_query::CompletionSource;
use tradar_query::DriverOpener;
use tradar_query::QueryDriver;

use crate::extension::{Ctx, Extension};
use crate::protocol::{METHOD_NOT_FOUND, RpcError, StoredResult};

mod cheatsheet;
mod connections;
mod editing;
pub mod idle;
mod query;

/// Rows returned inline by `execute` -- enough to fill a screen, so the
/// common case is one round trip; the rest comes through `fetch`.
const DEFAULT_PAGE: usize = 200;

/// Cursors kept per server. A result can hold `MAX_ROWS` rows, so an
/// unbounded map of them is the memory failure `MAX_ROWS` exists to prevent
/// -- oldest is evicted first.
const MAX_CURSORS: usize = 16;

/// Text held by all cursors together; past it the oldest cursors go, whatever
/// `MAX_CURSORS` says (a few wide JSON/text columns make a single result
/// tens of MB). The newest cursor always stays.
const MAX_CURSOR_BYTES: usize = 256 * 1024 * 1024;

/// Rows an `execute` reads from the database when the client does not say
/// (`max_rows`): a grid shows tens of rows, and every row read is stringified,
/// held and (for the first page) sent. `tradar_query::MAX_ROWS` stays the
/// ceiling for a client that asks for more.
const DEFAULT_FETCH_ROWS: usize = 1_000;

#[derive(Default)]
struct State {
    drivers: HashMap<String, Arc<dyn QueryDriver>>,
    /// Built once per connection (keywords + schema) and rebuilt whenever
    /// `schema` is called, the same lifetime the TUI gives its own
    /// `CompletionSource` -- never per keystroke.
    completions: HashMap<String, Arc<CompletionSource>>,
    cursors: HashMap<u64, StoredResult>,
    cursor_order: VecDeque<u64>,
    /// `StoredResult::approx_bytes` per cursor, taken once at insert.
    cursor_bytes: HashMap<u64, usize>,
    next_cursor: u64,
    /// In-flight `execute`s that a client named with `query_id`, so a
    /// `cancel` from the same (or another) connection can reach them --
    /// the `Notify` to drop the client's own wait, plus the driver to fire
    /// a real DB-side `cancel_query` against (`cancel` needs both: which
    /// future to stop waiting on, and which connection to tell the
    /// database about).
    running: HashMap<String, (Arc<Notify>, Arc<dyn QueryDriver>)>,
    /// Last call that named each open connection; see `idle`.
    last_used: HashMap<String, std::time::Instant>,
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
// `id`/`outbox`/`notify` are only used by push-capable extensions (Kafka,
// Socket); a build with both off leaves them unused.
#[cfg_attr(
    not(any(feature = "kafka", feature = "socket", feature = "grpc")),
    allow(dead_code)
)]
#[derive(Clone)]
pub struct ClientConnection {
    pub(crate) id: u64,
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

    /// A connection whose pushes land in the returned receiver, one
    /// JSON-RPC line each -- for a test that drives a subscription.
    pub fn with_inbox(id: u64) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (outbox, inbox) = mpsc::unbounded_channel();
        (Self { id, outbox }, inbox)
    }

    #[cfg_attr(
        not(any(feature = "kafka", feature = "socket", feature = "grpc")),
        allow(dead_code)
    )]
    pub(crate) fn notify(&self, method: &str, params: Value) {
        let line = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        let _ = self.outbox.send(format!("{line}\n"));
    }
}

pub struct Server {
    registry: HashMap<String, Box<dyn DriverOpener>>,
    store: Option<ConnectionStore>,
    state: Mutex<State>,
    extensions: Vec<Box<dyn Extension>>,
    /// Fired by the `shutdown` method; `main` waits on it next to Ctrl-C.
    shutdown: Arc<Notify>,
    next_client_id: AtomicU64,
    /// Sockets currently open; see `connected_clients`.
    open_clients: AtomicUsize,
    /// When this process started -- `version` compares it with the binary on disk.
    started: std::time::SystemTime,
    /// Close connections unused for this long; `None` keeps them forever.
    idle_ttl: Option<std::time::Duration>,
}

impl Server {
    pub fn new(
        registry: HashMap<String, Box<dyn DriverOpener>>,
        store: Option<ConnectionStore>,
    ) -> Self {
        Self {
            registry,
            store,
            state: Mutex::new(State::default()),
            extensions: crate::extension::all(),
            shutdown: Arc::new(Notify::new()),
            next_client_id: AtomicU64::new(1),
            open_clients: AtomicUsize::new(0),
            started: std::time::SystemTime::now(),
            idle_ttl: idle::parse_ttl(std::env::var("TRADAR_IDLE_TTL").ok().as_deref()),
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
        self.open_clients.fetch_add(1, Ordering::Relaxed);
        ClientConnection { id, outbox }
    }

    /// How many client sockets are open right now.
    pub fn connected_clients(&self) -> usize {
        self.open_clients.load(Ordering::Relaxed)
    }

    /// Call when a client's socket closes: aborts and forgets every
    /// background subscription it started and never explicitly stopped.
    pub fn client_disconnected(&self, client: &ClientConnection) {
        self.open_clients.fetch_sub(1, Ordering::Relaxed);
        for extension in &self.extensions {
            extension.client_disconnected(client.id);
        }
    }

    /// One request object in, one response object out (JSON-RPC 2.0, minus
    /// batches and notifications -- every call here wants an answer).
    pub async fn handle(&self, request: Value, client: &ClientConnection) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let started = std::time::Instant::now();
        let mut label = String::new();
        let outcome = match request.get("method").and_then(Value::as_str) {
            None => Err(RpcError::invalid("missing `method`")),
            Some(method) => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                label = method.to_string();
                if let Some(connection) = params.get("connection").and_then(Value::as_str) {
                    label = format!("{method} [{connection}]");
                }
                // The background `status` poll must not count as use, or no
                // connection would ever go idle.
                let used = params
                    .get("connection")
                    .and_then(Value::as_str)
                    .filter(|_| !matches!(method, "status" | "version" | "disconnect"));
                if let Some(name) = used {
                    self.touch(name);
                }
                let outcome = self.dispatch(method, &params, client).await;
                if let Some(name) = used {
                    self.touch(name);
                }
                outcome
            }
        };
        let ms = started.elapsed().as_millis();
        match &outcome {
            // Cheap, frequent calls (the background `status` poll, every
            // completion request, paging) would drown the log at `info`.
            Ok(_) if is_chatty(&label) => tracing::debug!("{label} ok {ms}ms"),
            Ok(_) => tracing::info!("{label} ok {ms}ms"),
            Err(error) => tracing::warn!(
                "{label} failed {ms}ms: {}",
                crate::logging::brief(&error.message)
            ),
        }
        match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": error.code, "message": error.message, "data": error.data},
            }),
        }
    }

    async fn dispatch(
        &self,
        method: &str,
        params: &Value,
        client: &ClientConnection,
    ) -> Result<Value, RpcError> {
        match method {
            "version" => Ok(self.version()),
            "connectors.list" => Ok(self.connectors()),
            "connections.list" => self.connections(),
            "connections.path" => Ok(json!({
                "path": self.store.as_ref().map(|s| s.path().display().to_string()),
            })),
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
            "snippet.options" => self.snippet_options(params).await,
            "cheatsheet" => self.cheatsheet(params).await,
            "complete" => self.complete(params),
            "edit.source" => self.edit_source(params),
            "edit.sql" => self.edit_sql(params),
            _ => self.dispatch_extension(method, params, client).await,
        }
    }

    /// Routes `<driver>.<method>` to the extension serving `<driver>`.
    async fn dispatch_extension(
        &self,
        method: &str,
        params: &Value,
        client: &ClientConnection,
    ) -> Result<Value, RpcError> {
        let unknown = || RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("unknown method `{method}`"),
            data: None,
        };
        let (driver, rest) = method.split_once('.').ok_or_else(unknown)?;
        let extension = self
            .extensions
            .iter()
            .find(|e| e.driver() == driver)
            .ok_or_else(unknown)?;
        let ctx = Ctx {
            store: self.store.as_ref(),
            client,
        };
        extension.handle(rest, params, &ctx).await
    }
}

/// Methods that run constantly in the background -- logged at `debug` only.
fn is_chatty(label: &str) -> bool {
    let method = label.split_once(' ').map_or(label, |(m, _)| m);
    matches!(
        method,
        "status" | "version" | "complete" | "keywords" | "split" | "fetch" | "cursor.close"
    )
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

#[cfg(test)]
mod tests;
