use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A user-named, user-saved HTTP request (`tradar-connector-http`'s Postman-style
/// screen, `Ctrl+K`/`Ctrl+L`) -- a separate shape from [`SavedSnippet`]
/// because a request has four structured fields, not one blob of text. See
/// "Thiết kế UI: HTTP, gRPC, Socket" in docs/architecture.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedHttpRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    /// Raw `Key: Value` lines, one header per line -- parsed at send time,
    /// not structured here (see the design doc for why).
    pub headers: String,
    pub body: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct HttpRequestsFile {
    #[serde(default)]
    requests: Vec<SavedHttpRequest>,
}

pub fn default_http_requests_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("http_requests.toml"))
}

#[derive(Debug, Clone)]
pub struct HttpRequestStore {
    path: PathBuf,
}

impl HttpRequestStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> anyhow::Result<Vec<SavedHttpRequest>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        let file: HttpRequestsFile = toml::from_str(&contents)?;
        Ok(file.requests)
    }

    pub fn save(&self, requests: &[SavedHttpRequest]) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = HttpRequestsFile {
            requests: requests.to_vec(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }
}

/// The saved-request library. Process-global for the same reason
/// [`Snippets`] is: `HttpScreen` (deep inside `tradar-connector-http`'s `Session`) is
/// the only thing that wants this, and threading "where requests live"
/// through the connector SPI would put a UI-management concern into a trait
/// every connector implements.
pub struct HttpRequests {
    pub(super) store: HttpRequestStore,
    pub(super) list: std::sync::RwLock<Vec<SavedHttpRequest>>,
}

/// Called once at startup, mirroring [`init_snippets`]. Before this -- and
/// in tests, which never call it -- [`http_requests`] returns `None`.
pub fn init_http_requests(store: HttpRequestStore) {
    let list = store.load().unwrap_or_default();
    let _ = HTTP_REQUESTS.set(HttpRequests {
        store,
        list: std::sync::RwLock::new(list),
    });
}

pub fn http_requests() -> Option<&'static HttpRequests> {
    HTTP_REQUESTS.get()
}

impl HttpRequests {
    /// Every saved request, in save order -- unlike [`Snippets`], not
    /// scoped per-connector: one HTTP connection's saved requests are just
    /// as useful to send against another (there's no schema/driver mismatch
    /// concern the way there is with SQL).
    pub fn all(&self) -> Vec<SavedHttpRequest> {
        self.list
            .read()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    /// Adds a new request, or overwrites the existing one with the same
    /// name -- saving under a name already in use updates it in place.
    pub fn save(&self, request: SavedHttpRequest) {
        let Ok(mut guard) = self.list.write() else {
            return;
        };
        guard.retain(|r| r.name != request.name);
        guard.push(request);
        let _ = self.store.save(&guard);
    }

    pub fn delete(&self, name: &str) {
        let Ok(mut guard) = self.list.write() else {
            return;
        };
        guard.retain(|r| r.name != name);
        let _ = self.store.save(&guard);
    }
}

pub(crate) static HTTP_REQUESTS: std::sync::OnceLock<HttpRequests> = std::sync::OnceLock::new();
