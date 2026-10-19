//! Local persistence for saved connections and app state, via the
//! `directories` crate's platform-appropriate config path.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedConnection {
    pub name: String,
    /// A connector id (e.g. `"postgres"`, `"sqlite"`), matched at runtime
    /// against `ConnectorDescriptor::id` -- see the "Registry" section in
    /// docs/architecture.md. Not a closed enum: `tradar-core` has no
    /// business knowing the full set of connectors compiled into a given
    /// build.
    pub driver: String,
    pub target: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    connections: Vec<SavedConnection>,
}

pub fn default_connections_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("connections.toml"))
}

/// Which tabs were open (and connected) when `tradar` last quit, so the
/// next run can reconnect them instead of starting back at a bare picker.
/// Only tabs that had actually connected are worth remembering -- a tab
/// still sitting on the picker has nothing to reconnect to.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default)]
    pub active_tab: usize,
    /// The connected tabs, in order.
    #[serde(default)]
    pub tabs: Vec<TabState>,
}

/// One remembered tab. The connection is matched by name against
/// `connections.toml` on the next run -- a name that no longer exists there
/// is silently skipped, since it may have been renamed or removed.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    pub connection: String,
    /// What was in the query editor. Kept so quitting doesn't throw away
    /// work in progress; empty when there was nothing typed.
    #[serde(default)]
    pub query: String,
}

impl TabState {
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            query: String::new(),
        }
    }
}

/// Where saved queries live by default. Same directory as the other state
/// files, so everything tradar owns is in one place.
pub fn default_queries_dir() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("queries"))
}

/// Where one connection's migration files live: `migrations/<connection
/// name>` under the same config directory as everything else tradar owns.
/// Per connection, unlike `default_queries_dir()` -- a migration set is
/// inherently tied to one database, not shared editing material the way
/// saved queries are. The name is sanitized (anything that isn't
/// alphanumeric, `-`, `_`, or `.` becomes `_`) since it's typed freely into
/// the connection form and would otherwise let a name like `a/../../etc`
/// escape the migrations directory or collide across platforms.
pub fn default_migrations_dir(connection_name: &str) -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    let sanitized: String = connection_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(dirs.config_dir().join("migrations").join(sanitized))
}

/// Moves a connection's migrations directory after its name changes, so
/// editing a connection (the normal "fix a typo" path) doesn't silently
/// orphan the whole directory -- the panel would otherwise start reading
/// an empty/different path and report "up to date" with nothing pending,
/// while the real files sit untouched at the old, now-unreachable name.
/// Best-effort and silent either way: nothing to move if the old
/// directory never existed (no migrations run yet), and the old directory
/// is left in place rather than overwritten or merged if something is
/// already at the new path (e.g. left over from an unrelated, since-
/// deleted connection that once had the same name) -- losing migration
/// history to a rename would be worse than requiring a manual cleanup
/// here.
pub fn rename_migrations_dir(
    old_connection_name: &str,
    new_connection_name: &str,
) -> anyhow::Result<()> {
    let old = default_migrations_dir(old_connection_name)?;
    let new = default_migrations_dir(new_connection_name)?;
    move_dir_if_safe(&old, &new);
    Ok(())
}

fn move_dir_if_safe(old: &std::path::Path, new: &std::path::Path) {
    if old == new || !old.exists() || new.exists() {
        return;
    }
    let _ = std::fs::rename(old, new);
}

/// Turns what the user typed in a save/open prompt into a path. A bare
/// name, or a relative path with subfolders (e.g. `reports/first`), lands
/// inside the queries directory and gains a `.sql` extension if it doesn't
/// already have one -- this is what lets the folder browser's own
/// subfolder prefills (`reports/`) round-trip correctly. Only an explicit
/// escape -- absolute (`/...`), home (`~/...`), or CWD-relative (`./...`,
/// `../...`) -- is taken as a path and used as-is, so anyone who wants to
/// save next to their project still can.
pub fn resolve_query_path(input: &str, queries_dir: &std::path::Path) -> PathBuf {
    let input = input.trim();
    let is_escape = input.starts_with('/')
        || input.starts_with('\\')
        || input.starts_with('~')
        || input.starts_with("./")
        || input.starts_with("../")
        || input.starts_with(".\\")
        || input.starts_with("..\\");
    if is_escape {
        return PathBuf::from(input);
    }
    let name = if std::path::Path::new(input).extension().is_some() {
        input.to_string()
    } else {
        format!("{input}.sql")
    };
    queries_dir.join(name)
}

/// The files most recently saved or opened, most recent first. Global
/// rather than per session: which queries you were last editing isn't tied
/// to which connections happened to be open.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentFiles {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Longest the recent list gets. Past this it stops being "recent" and
/// starts being a second, worse file browser.
const MAX_RECENT: usize = 20;

impl RecentFiles {
    /// Moves `path` to the front, de-duplicating so re-opening a file
    /// promotes it rather than listing it twice.
    pub fn record(&mut self, path: &std::path::Path) {
        let path = path.to_string_lossy().to_string();
        self.paths.retain(|existing| *existing != path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX_RECENT);
    }
}

pub fn default_recent_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("recent.toml"))
}

#[derive(Debug, Clone)]
pub struct RecentStore {
    path: PathBuf,
}

impl RecentStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> anyhow::Result<RecentFiles> {
        if !self.path.exists() {
            return Ok(RecentFiles::default());
        }
        Ok(toml::from_str(&std::fs::read_to_string(&self.path)?)?)
    }

    pub fn save(&self, recent: &RecentFiles) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, toml::to_string_pretty(recent)?)?;
        Ok(())
    }
}

/// Where saved queries live and which ones were opened lately.
///
/// Process-global like [`crate::theme`] and [`crate::keymap`], and for the
/// same reason: every query screen wants it, screens are built deep inside
/// connectors, and threading it down would mean putting "where files live"
/// into the connector SPI -- which has nothing to do with connecting to a
/// database.
pub struct QueryFiles {
    dir: PathBuf,
    store: RecentStore,
    recent: std::sync::RwLock<Vec<String>>,
}

static QUERY_FILES: std::sync::OnceLock<QueryFiles> = std::sync::OnceLock::new();

/// Called once at startup. Before this -- and in tests, which never call it
/// -- [`query_files`] returns `None` and callers fall back to plain paths.
pub fn init_query_files(dir: PathBuf, store: RecentStore) {
    let recent = store.load().unwrap_or_default().paths;
    let _ = QUERY_FILES.set(QueryFiles {
        dir,
        store,
        recent: std::sync::RwLock::new(recent),
    });
}

pub fn query_files() -> Option<&'static QueryFiles> {
    QUERY_FILES.get()
}

impl QueryFiles {
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Most recent first.
    pub fn recent(&self) -> Vec<String> {
        self.recent.read().map(|r| r.clone()).unwrap_or_default()
    }

    /// Promotes `path` to the front and persists the list. A write failure
    /// is swallowed: losing a recent entry must not fail the save or open
    /// that just succeeded.
    pub fn record(&self, path: &std::path::Path) {
        let Ok(mut guard) = self.recent.write() else {
            return;
        };
        let mut recent = RecentFiles {
            paths: std::mem::take(&mut *guard),
        };
        recent.record(path);
        let _ = self.store.save(&recent);
        *guard = recent.paths;
    }
}

/// A user-named, user-saved query, kept separate from the auto-generated
/// CRUD snippets (`Component::crud_snippet`) -- this one is arbitrary text
/// the user chose to keep, not derived from a schema entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSnippet {
    pub name: String,
    /// A connector id (`"postgres"`, `"redis"`, ...), same meaning as
    /// `SavedConnection::driver`. `name` only has to be unique within one
    /// driver -- a Postgres snippet and a Redis snippet can share a name
    /// without conflict, since the library overlay only ever shows one
    /// driver's snippets at a time (the currently open connection's).
    pub driver: String,
    pub text: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SnippetsFile {
    #[serde(default)]
    snippets: Vec<SavedSnippet>,
}

pub fn default_snippets_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("snippets.toml"))
}

#[derive(Debug, Clone)]
pub struct SnippetStore {
    path: PathBuf,
}

impl SnippetStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> anyhow::Result<Vec<SavedSnippet>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        let file: SnippetsFile = toml::from_str(&contents)?;
        Ok(file.snippets)
    }

    pub fn save(&self, snippets: &[SavedSnippet]) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = SnippetsFile {
            snippets: snippets.to_vec(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }
}

/// The saved-snippet library. Process-global for the same reason
/// [`QueryFiles`] is: `QueryScreenComponent` (deep inside a connector's
/// `Screen`) is the only thing that wants this, and threading "where
/// snippets live" through the connector SPI would put a UI-management
/// concern into the trait every connector has to implement.
pub struct Snippets {
    store: SnippetStore,
    list: std::sync::RwLock<Vec<SavedSnippet>>,
}

static SNIPPETS: std::sync::OnceLock<Snippets> = std::sync::OnceLock::new();

/// Called once at startup, mirroring [`init_query_files`]. Before this --
/// and in tests, which never call it -- [`snippets`] returns `None`.
pub fn init_snippets(store: SnippetStore) {
    let list = store.load().unwrap_or_default();
    let _ = SNIPPETS.set(Snippets {
        store,
        list: std::sync::RwLock::new(list),
    });
}

pub fn snippets() -> Option<&'static Snippets> {
    SNIPPETS.get()
}

impl Snippets {
    /// This driver's snippets, in save order -- what the library overlay
    /// lists. Filtered here rather than by the caller so nothing outside
    /// this module needs to know the `(name, driver)` uniqueness rule.
    pub fn for_driver(&self, driver: &str) -> Vec<SavedSnippet> {
        self.list
            .read()
            .map(|list| {
                list.iter()
                    .filter(|s| s.driver == driver)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Adds a new snippet, or overwrites the existing one with the same
    /// name for this driver -- saving under a name already in use updates
    /// it in place rather than erroring or duplicating it.
    pub fn save(&self, name: String, driver: String, text: String) {
        let Ok(mut guard) = self.list.write() else {
            return;
        };
        guard.retain(|s| !(s.name == name && s.driver == driver));
        guard.push(SavedSnippet { name, driver, text });
        let _ = self.store.save(&guard);
    }

    pub fn delete(&self, driver: &str, name: &str) {
        let Ok(mut guard) = self.list.write() else {
            return;
        };
        guard.retain(|s| !(s.name == name && s.driver == driver));
        let _ = self.store.save(&guard);
    }

    /// A no-op if `old_name` isn't found for `driver` -- the entry may
    /// already have been deleted by a concurrent action.
    pub fn rename(&self, driver: &str, old_name: &str, new_name: String) {
        let Ok(mut guard) = self.list.write() else {
            return;
        };
        if let Some(entry) = guard
            .iter_mut()
            .find(|s| s.name == old_name && s.driver == driver)
        {
            entry.name = new_name;
        }
        let _ = self.store.save(&guard);
    }
}

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
struct HttpRequestsFile {
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
    store: HttpRequestStore,
    list: std::sync::RwLock<Vec<SavedHttpRequest>>,
}

static HTTP_REQUESTS: std::sync::OnceLock<HttpRequests> = std::sync::OnceLock::new();

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

pub fn default_session_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("session.toml"))
}

pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> anyhow::Result<SessionState> {
        if !self.path.exists() {
            return Ok(SessionState::default());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        Ok(toml::from_str(&contents)?)
    }

    pub fn save(&self, state: &SessionState) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(state)?;
        std::fs::write(&self.path, contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionStore {
    path: PathBuf,
}

impl ConnectionStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The `connections.toml` this store reads and writes.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> anyhow::Result<Vec<SavedConnection>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        let file: ConnectionsFile = toml::from_str(&contents)?;
        Ok(file.connections)
    }

    pub fn save(&self, connections: &[SavedConnection]) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = ConnectionsFile {
            connections: connections.to_vec(),
        };
        let contents = toml::to_string_pretty(&file)?;
        std::fs::write(&self.path, contents)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
