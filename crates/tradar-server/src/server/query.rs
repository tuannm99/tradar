use crate::protocol::{RpcError, StoredResult, str_param, u64_param, usize_param};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Notify;
use tradar_query::CompletionSource;
use tradar_query::QueryDriver;

use super::*;

impl Server {
    pub(super) async fn execute(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let query = str_param(params, "query")?;
        let page_size = usize_param(params, "page_size")?.unwrap_or(DEFAULT_PAGE);

        // The lock is not held across this await: a slow query on one
        // connection must not block another client's `fetch`. A `cancel`
        // for this `query_id` drops the in-flight future *and* fires the
        // driver's own `cancel_query` -- see `cancel` below.
        let max_rows = usize_param(params, "max_rows")?.unwrap_or(DEFAULT_FETCH_ROWS);
        let query_id = params.get("query_id").and_then(Value::as_str);
        let cancel = Arc::new(Notify::new());
        if let Some(id) = query_id {
            self.state
                .lock()
                .unwrap()
                .running
                .insert(id.to_string(), (Arc::clone(&cancel), Arc::clone(&driver)));
        }
        // `biased`, cancel first: `cancel` stores its permit *before* it asks
        // the driver to interrupt, so when the interrupted statement fails
        // (SQLite: "interrupted") both branches are ready at once -- and a
        // random pick would report the user's own cancel as a database error.
        let started = std::time::Instant::now();
        let outcome = tokio::select! {
            biased;
            () = cancel.notified() => None,
            result = tradar_query::with_max_rows(max_rows, driver.execute(query)) => Some(result),
        };
        // What the database took, without the socket round trip or the
        // client's rendering; shown next to the row count.
        let elapsed_ms = (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0;
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
            Err(rows) => {
                return Ok(json!({"kind": "affected", "rows": rows, "elapsed_ms": elapsed_ms}));
            }
        };

        let first_page = stored.page(0, page_size);
        let mut response = json!({
            "kind": stored.kind(),
            "columns": stored.columns(),
            "total": stored.total(),
            "truncated": stored.truncated(),
            "rows": first_page,
            "elapsed_ms": elapsed_ms,
        });
        if let Some(field_order) = stored.field_order(0, page_size) {
            response["field_order"] = field_order;
        }
        let mut state = self.state.lock().unwrap();
        state.next_cursor += 1;
        let cursor = state.next_cursor;
        state.cursor_bytes.insert(cursor, stored.approx_bytes());
        state.cursors.insert(cursor, stored);
        state.cursor_order.push_back(cursor);
        let mut held: usize = state.cursor_bytes.values().sum();
        while state.cursor_order.len() > MAX_CURSORS
            || (held > MAX_CURSOR_BYTES && state.cursor_order.len() > 1)
        {
            let Some(oldest) = state.cursor_order.pop_front() else {
                break;
            };
            state.cursors.remove(&oldest);
            held = held.saturating_sub(state.cursor_bytes.remove(&oldest).unwrap_or(0));
        }
        response["cursor"] = json!(cursor);
        Ok(response)
    }

    /// A skeleton statement in the driver's *own* language for a table --
    /// `SELECT ... LIMIT` for SQL, `find()` for Mongo, a `_search` for
    /// Elasticsearch -- so "open this table" never has to know which it is.
    /// `schema` disambiguates same-named tables (Postgres schemas).
    pub(super) async fn snippet(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let name = str_param(params, "name")?;
        let id = params.get("op").and_then(Value::as_str).unwrap_or("read");
        if !driver.snippet_options().iter().any(|o| o.id == id) {
            return Err(RpcError::invalid(format!("unknown op `{id}`")));
        }
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
        Ok(json!({"text": driver.option_snippet(entry, id, &columns)}))
    }

    /// The driver's cheat sheet (`QueryDriver::cheatsheet`), in order:
    /// `{entries: [{category, title, text}]}`. Static -- nothing is sent to
    /// the database.
    pub(super) async fn cheatsheet(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let entries: Vec<Value> = driver
            .cheatsheet()
            .iter()
            .map(|e| json!({"category": e.category, "title": e.title, "text": e.text}))
            .collect();
        Ok(json!({"entries": entries}))
    }

    /// The statements `snippet` can generate on this connection, in menu
    /// order: `{options: [{id, label, hint}]}`. Create / Read / Update /
    /// Delete for most drivers; more where a driver has more basic shapes.
    pub(super) async fn snippet_options(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let options: Vec<Value> = driver
            .snippet_options()
            .iter()
            .map(|o| json!({"id": o.id, "label": o.label, "hint": o.hint}))
            .collect();
        Ok(json!({"options": options}))
    }

    pub(super) async fn cancel(&self, params: &Value) -> Result<Value, RpcError> {
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
        if self.extensions.iter().any(|e| e.cancel(id)) {
            return Ok(json!({"cancelled": true}));
        }
        Ok(json!({"cancelled": false}))
    }

    /// Rebuilds a connection's completion source from the live schema. A
    /// failed schema read keeps the old source rather than blanking it.
    pub(super) async fn refresh_completions(&self, params: &Value, driver: &Arc<dyn QueryDriver>) {
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

    pub(super) fn fetch(&self, params: &Value) -> Result<Value, RpcError> {
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

    pub(super) fn close_cursor(&self, params: &Value) -> Result<Value, RpcError> {
        let cursor = u64_param(params, "cursor")?;
        let mut state = self.state.lock().unwrap();
        state.cursor_order.retain(|c| *c != cursor);
        state.cursor_bytes.remove(&cursor);
        Ok(json!({"closed": state.cursors.remove(&cursor).is_some()}))
    }
}
