use crate::protocol::{PROTOCOL_VERSION, RpcError, schema_json, str_param};
use serde_json::{Value, json};
use std::sync::Arc;
use tradar_core::storage::SavedConnection;
use tradar_query::CompletionSource;
use tradar_query::QueryDriver;

use super::*;

/// An ad-hoc connection written straight into the name: `@driver:target`
/// (`@elasticsearch:http://localhost:9201`, `@postgres:postgres://u:p@h/db`,
/// `@sqlite:/tmp/x.db`). It needs no `connections.toml` entry; the driver id
/// is the connector that opens `target`.
pub(crate) fn adhoc_connection(name: &str) -> Option<SavedConnection> {
    let (driver, target) = name.strip_prefix('@')?.split_once(':')?;
    let driver_ok = !driver.is_empty()
        && driver
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !driver_ok || target.is_empty() {
        return None;
    }
    Some(SavedConnection {
        name: name.to_string(),
        driver: driver.to_ascii_lowercase(),
        target: target.to_string(),
    })
}

/// `name` for the log: a password in `scheme://user:password@host` is hidden.
pub(crate) fn redact(name: &str) -> String {
    let Some(scheme_end) = name.find("://") else {
        return name.to_string();
    };
    let start = scheme_end + 3;
    let rest = &name[start..];
    let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
    let Some(at) = authority.rfind('@') else {
        return name.to_string();
    };
    let Some(colon) = authority[..at].find(':') else {
        return name.to_string();
    };
    format!("{}:***{}", &name[..start + colon], &name[start + at..])
}

impl Server {
    /// The connection called `name`: one saved in `connections.toml`, else an
    /// ad-hoc `@driver:target` (see `adhoc_connection`).
    pub(super) fn find_connection(&self, name: &str) -> Result<Option<SavedConnection>, RpcError> {
        if let Some(saved) = self.saved()?.into_iter().find(|c| c.name == name) {
            return Ok(Some(saved));
        }
        Ok(adhoc_connection(name))
    }

    /// `{major, minor}` of `protocol::PROTOCOL_VERSION` -- never behind a
    /// connector feature gate (unlike almost every other method here), so
    /// even the most trimmed build still answers it. See that constant's
    /// own doc comment for what a mismatch means and why this never blocks
    /// a call.
    pub(super) fn version(&self) -> Value {
        let (exe, stale) = self.binary_state();
        json!({
            "major": PROTOCOL_VERSION.0,
            "minor": PROTOCOL_VERSION.1,
            "exe": exe,
            "stale": stale,
            "log_path": crate::logging::log_path().map(|p| p.display().to_string()),
            "idle_ttl_secs": self.idle_ttl.map(|d| d.as_secs()),
        })
    }

    /// Which binary this process is, and whether it has been rebuilt since
    /// it started -- the cause of "I fixed it and rebuilt but it still
    /// behaves the old way", since this server outlives the editor session.
    /// A replaced binary shows up either as a newer mtime or, on Linux, as
    /// the `/proc/self/exe` target gaining a ` (deleted)` suffix.
    pub(super) fn binary_state(&self) -> (Option<String>, bool) {
        let Ok(path) = std::env::current_exe() else {
            return (None, false);
        };
        let shown = path.display().to_string();
        if let Some(original) = shown.strip_suffix(" (deleted)") {
            return (Some(original.to_string()), true);
        }
        let rebuilt = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|mtime| mtime > self.started);
        (Some(shown), rebuilt)
    }

    pub(super) fn connectors(&self) -> Value {
        let mut ids: Vec<&str> = self.registry.keys().map(String::as_str).collect();
        ids.sort_unstable();
        json!(ids)
    }

    pub(super) fn saved(&self) -> Result<Vec<SavedConnection>, RpcError> {
        match &self.store {
            Some(store) => Ok(store.load()?),
            None => Ok(Vec::new()),
        }
    }

    pub(super) fn connections(&self) -> Result<Value, RpcError> {
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
                        || self.extensions.iter().any(|e| e.is_connected(&c.name)),
                    "supported": self.is_supported(&c.driver),
                })
            })
            .collect();
        Ok(json!(list))
    }

    /// Whether this server build can serve `driver` at all -- through the
    /// `Connector`/`QueryDriver` registry, or through an extension's own
    /// `<driver>.*` methods.
    pub(super) fn is_supported(&self, driver: &str) -> bool {
        self.registry.contains_key(driver) || self.extensions.iter().any(|e| e.driver() == driver)
    }

    pub(super) fn driver(&self, params: &Value) -> Result<Arc<dyn QueryDriver>, RpcError> {
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

    pub(super) async fn connect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        if self.state.lock().unwrap().drivers.contains_key(name) {
            return Ok(json!({"connected": true, "already": true}));
        }
        let saved = self.find_connection(name)?.ok_or_else(|| {
            RpcError::app(format!("no saved connection named `{}`", redact(name)))
        })?;
        let opener = self.registry.get(&saved.driver).ok_or_else(|| {
            RpcError::app(format!(
                "this server build has no `{}` connector (or it has no query language to serve)",
                saved.driver
            ))
        })?;
        let driver = opener.open(&saved.target).await?;
        // A failed schema load still connects, as in the TUI: completion
        // just has no schema names to offer.
        let schema = driver.list_schema().await.unwrap_or_default();
        let source = Arc::new(CompletionSource::new(driver.keywords(), &schema));
        let mut state = self.state.lock().unwrap();
        state.drivers.insert(name.to_string(), driver);
        state
            .last_used
            .insert(name.to_string(), std::time::Instant::now());
        state.completions.insert(name.to_string(), source);
        Ok(json!({"connected": true, "already": false}))
    }

    /// Closes connection `name` whatever serves it -- a query driver or an
    /// extension (the same as `<driver>.disconnect`).
    pub(super) fn disconnect(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let mut removed = {
            let mut state = self.state.lock().unwrap();
            state.completions.remove(name);
            state.last_used.remove(name);
            state.drivers.remove(name).is_some()
        };
        for extension in &self.extensions {
            removed |= extension.close(name);
        }
        Ok(json!({"disconnected": removed}))
    }

    pub(super) async fn status(&self, params: &Value) -> Result<Value, RpcError> {
        let driver = self.driver(params)?;
        let in_transaction = driver.in_transaction();
        // A held transaction owns the connection, so a pool-based ping would
        // wait for it (and report "dead" after the acquire timeout): the open
        // transaction is itself proof the connection is up.
        let alive = in_transaction || driver.ping().await.is_ok();
        Ok(json!({"alive": alive, "in_transaction": in_transaction}))
    }

    pub(super) async fn schema(&self, params: &Value) -> Result<Value, RpcError> {
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
}
