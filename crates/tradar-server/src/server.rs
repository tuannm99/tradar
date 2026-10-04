//! The headless host: owns live connections and result cursors, and answers
//! JSON-RPC requests. `Server::handle` is transport-free (a `Value` in, a
//! `Value` out) so every method is testable without a socket; `transport`
//! is the thin unix-socket shell around it.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use tradar_connector_spi::Connector;
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
}

pub struct Server {
    registry: HashMap<String, Box<dyn Connector>>,
    store: Option<ConnectionStore>,
    state: Mutex<State>,
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
        }
    }

    /// One request object in, one response object out (JSON-RPC 2.0, minus
    /// batches and notifications -- every call here wants an answer).
    pub async fn handle(&self, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let outcome = match request.get("method").and_then(Value::as_str) {
            None => Err(RpcError::invalid("missing `method`")),
            Some(method) => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                self.dispatch(method, &params).await
            }
        };
        match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": error.code, "message": error.message},
            }),
        }
    }

    async fn dispatch(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "connectors.list" => Ok(self.connectors()),
            "connections.list" => self.connections(),
            "connect" => self.connect(params).await,
            "disconnect" => self.disconnect(params),
            "status" => self.status(params).await,
            "schema" => self.schema(params).await,
            "execute" => self.execute(params).await,
            "fetch" => self.fetch(params),
            "cursor.close" => self.close_cursor(params),
            "split" => self.split(params),
            "keywords" => self.keywords(params),
            "complete" => self.complete(params),
            "edit.source" => self.edit_source(params),
            "edit.sql" => self.edit_sql(params),
            _ => Err(RpcError {
                code: METHOD_NOT_FOUND,
                message: format!("unknown method `{method}`"),
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
                    "connected": connected.drivers.contains_key(&c.name),
                    "supported": self.registry.contains_key(&c.driver),
                })
            })
            .collect();
        Ok(json!(list))
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
        // connection must not block another client's `fetch`.
        let result = driver.execute(query).await?;
        let stored = match StoredResult::from_query(result) {
            Ok(stored) => stored,
            Err(rows) => return Ok(json!({"kind": "affected", "rows": rows})),
        };

        let first_page = stored.page(0, page_size);
        let response = json!({
            "kind": stored.kind(),
            "columns": stored.columns(),
            "total": stored.total(),
            "truncated": stored.truncated(),
            "rows": first_page,
        });
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
        let mut response = response;
        response["cursor"] = json!(cursor);
        Ok(response)
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
        Ok(json!({"offset": offset, "total": stored.total(), "rows": stored.page(offset, limit)}))
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
        Ok(json!({"table": source, "key_columns": keys}))
    }

    /// Only *builds* the statement; running it is the client's explicit
    /// follow-up `execute`, which keeps the TUI's "show it, run it only
    /// after a `y`" rule instead of hiding a write inside this call.
    fn edit_sql(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let edit = parse_row_edit(params)?;
        Ok(json!({"sql": driver.edit_sql(&edit)}))
    }
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
