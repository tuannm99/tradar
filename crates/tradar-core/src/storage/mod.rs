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
mod tests {
    use super::*;

    #[test]
    fn loading_from_a_missing_file_returns_no_connections() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::at(dir.path().join("connections.toml"));

        let connections = store.load().unwrap();

        assert!(connections.is_empty());
    }

    #[test]
    fn default_connections_path_ends_with_connections_toml() {
        let path = default_connections_path().unwrap();

        assert_eq!(path.file_name().unwrap(), "connections.toml");
    }

    #[test]
    fn saving_then_loading_round_trips_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::at(dir.path().join("connections.toml"));
        let connection = SavedConnection {
            name: "local".to_string(),
            driver: "sqlite".to_string(),
            target: "test.db".to_string(),
        };

        store.save(std::slice::from_ref(&connection)).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, vec![connection]);
    }

    #[test]
    fn default_snippets_path_ends_with_snippets_toml() {
        let path = default_snippets_path().unwrap();

        assert_eq!(path.file_name().unwrap(), "snippets.toml");
    }

    #[test]
    fn loading_a_missing_snippets_file_returns_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();

        let snippets = SnippetStore::at(dir.path().join("nope.toml"))
            .load()
            .unwrap();

        assert!(snippets.is_empty());
    }

    #[test]
    fn saving_a_snippet_then_loading_round_trips_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnippetStore::at(dir.path().join("snippets.toml"));
        let snippet = SavedSnippet {
            name: "active-users".to_string(),
            driver: "postgres".to_string(),
            text: "SELECT * FROM users WHERE active;".to_string(),
        };

        store.save(std::slice::from_ref(&snippet)).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, vec![snippet]);
    }

    /// A `Snippets` backed by a temp-dir store, bypassing the process-global
    /// singleton (`init_snippets`/`snippets()`) entirely -- same reasoning
    /// as testing `ConnectionStore`/`RecentStore` directly rather than
    /// through `QueryFiles`.
    fn test_snippets(dir: &std::path::Path) -> Snippets {
        Snippets {
            store: SnippetStore::at(dir.join("snippets.toml")),
            list: std::sync::RwLock::new(Vec::new()),
        }
    }

    #[test]
    fn saving_under_an_existing_name_and_driver_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());
        snippets.save(
            "q".to_string(),
            "postgres".to_string(),
            "SELECT 1".to_string(),
        );

        snippets.save(
            "q".to_string(),
            "postgres".to_string(),
            "SELECT 2".to_string(),
        );

        assert_eq!(
            snippets.for_driver("postgres"),
            vec![SavedSnippet {
                name: "q".to_string(),
                driver: "postgres".to_string(),
                text: "SELECT 2".to_string(),
            }],
            "saving under a name already in use must update it, not duplicate it"
        );
    }

    #[test]
    fn the_same_name_on_two_different_drivers_does_not_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());

        snippets.save(
            "q".to_string(),
            "postgres".to_string(),
            "SELECT 1".to_string(),
        );
        snippets.save("q".to_string(), "redis".to_string(), "GET k".to_string());

        assert_eq!(snippets.for_driver("postgres").len(), 1);
        assert_eq!(snippets.for_driver("redis").len(), 1);
    }

    #[test]
    fn for_driver_only_returns_that_driver_s_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());
        snippets.save(
            "a".to_string(),
            "postgres".to_string(),
            "SELECT 1".to_string(),
        );
        snippets.save("b".to_string(), "redis".to_string(), "GET k".to_string());

        let entries = snippets.for_driver("postgres");
        let names: Vec<&str> = entries.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn deleting_removes_only_the_matching_driver_s_entry() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());
        snippets.save(
            "q".to_string(),
            "postgres".to_string(),
            "SELECT 1".to_string(),
        );
        snippets.save("q".to_string(), "redis".to_string(), "GET k".to_string());

        snippets.delete("postgres", "q");

        assert!(snippets.for_driver("postgres").is_empty());
        assert_eq!(snippets.for_driver("redis").len(), 1);
    }

    #[test]
    fn renaming_updates_the_name_and_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());
        snippets.save(
            "old".to_string(),
            "postgres".to_string(),
            "SELECT 1".to_string(),
        );

        snippets.rename("postgres", "old", "new".to_string());

        assert_eq!(
            snippets.for_driver("postgres"),
            vec![SavedSnippet {
                name: "new".to_string(),
                driver: "postgres".to_string(),
                text: "SELECT 1".to_string(),
            }]
        );
        let reloaded = SnippetStore::at(dir.path().join("snippets.toml"))
            .load()
            .unwrap();
        assert_eq!(reloaded[0].name, "new");
    }

    #[test]
    fn renaming_a_snippet_that_does_not_exist_is_a_harmless_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = test_snippets(dir.path());

        snippets.rename("postgres", "ghost", "new".to_string());

        assert!(snippets.for_driver("postgres").is_empty());
    }

    #[test]
    fn a_session_file_from_before_queries_were_saved_is_ignored_not_fatal() {
        // The old format stored tabs as plain strings. Rather than
        // migrating, loading fails and the caller falls back to a fresh
        // session -- losing a restore is a smaller cost than the migration
        // code for a file that is rewritten on every quit anyway.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "active_tab = 0\ntabs = [\"local sqlite\"]\n").unwrap();

        assert!(SessionStore::at(path).load().is_err());
    }

    #[test]
    fn a_bare_name_is_saved_into_the_queries_directory_as_sql() {
        let dir = std::path::Path::new("/home/u/.config/tradar/queries");

        assert_eq!(
            resolve_query_path("report", dir),
            dir.join("report.sql"),
            "a bare name gets the extension so files are readable elsewhere"
        );
        assert_eq!(
            resolve_query_path("report.txt", dir),
            dir.join("report.txt"),
            "an explicit extension is respected"
        );
    }

    #[test]
    fn an_explicit_escape_is_used_as_typed() {
        let dir = std::path::Path::new("/home/u/.config/tradar/queries");

        assert_eq!(
            resolve_query_path("/tmp/one.sql", dir),
            PathBuf::from("/tmp/one.sql")
        );
        assert_eq!(
            resolve_query_path("./one.sql", dir),
            PathBuf::from("./one.sql")
        );
        assert_eq!(
            resolve_query_path("../one.sql", dir),
            PathBuf::from("../one.sql")
        );
        assert_eq!(
            resolve_query_path("~/one.sql", dir),
            PathBuf::from("~/one.sql")
        );
    }

    #[test]
    fn a_relative_subfolder_is_joined_into_the_queries_directory() {
        let dir = std::path::Path::new("/home/u/.config/tradar/queries");

        assert_eq!(
            resolve_query_path("reports/first", dir),
            dir.join("reports/first.sql"),
            "no leading escape marker, so this is a subfolder inside the queries dir, not a cwd-relative path"
        );
        assert_eq!(
            resolve_query_path("reports/first.sql", dir),
            dir.join("reports/first.sql")
        );
    }

    #[test]
    fn recording_a_file_moves_it_to_the_front_without_duplicating() {
        let mut recent = RecentFiles::default();

        recent.record(std::path::Path::new("/a.sql"));
        recent.record(std::path::Path::new("/b.sql"));
        recent.record(std::path::Path::new("/a.sql"));

        assert_eq!(recent.paths, vec!["/a.sql", "/b.sql"]);
    }

    #[test]
    fn the_recent_list_is_capped() {
        let mut recent = RecentFiles::default();

        for i in 0..(MAX_RECENT + 5) {
            recent.record(std::path::Path::new(&format!("/{i}.sql")));
        }

        assert_eq!(recent.paths.len(), MAX_RECENT);
        assert_eq!(recent.paths[0], format!("/{}.sql", MAX_RECENT + 4));
    }

    #[test]
    fn saving_then_loading_round_trips_the_recent_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = RecentStore::at(dir.path().join("recent.toml"));
        let mut recent = RecentFiles::default();
        recent.record(std::path::Path::new("/a.sql"));

        store.save(&recent).unwrap();

        assert_eq!(store.load().unwrap(), recent);
    }

    #[test]
    fn loading_a_missing_recent_file_returns_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();

        let recent = RecentStore::at(dir.path().join("nope.toml"))
            .load()
            .unwrap();

        assert!(recent.paths.is_empty());
    }

    #[test]
    fn loading_a_missing_session_file_returns_the_default_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::at(dir.path().join("session.toml"));

        let state = store.load().unwrap();

        assert_eq!(state, SessionState::default());
    }

    #[test]
    fn default_session_path_ends_with_session_toml() {
        let path = default_session_path().unwrap();

        assert_eq!(path.file_name().unwrap(), "session.toml");
    }

    #[test]
    fn saving_then_loading_round_trips_a_session_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::at(dir.path().join("session.toml"));
        let state = SessionState {
            active_tab: 1,
            tabs: vec![
                TabState {
                    connection: "local sqlite".to_string(),
                    query: "select 1;\nselect 2;".to_string(),
                },
                TabState::new("local postgres"),
            ],
        };

        store.save(&state).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, state);
    }

    #[test]
    fn default_http_requests_path_ends_with_http_requests_toml() {
        let path = default_http_requests_path().unwrap();

        assert_eq!(path.file_name().unwrap(), "http_requests.toml");
    }

    #[test]
    fn loading_a_missing_http_requests_file_returns_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();

        let requests = HttpRequestStore::at(dir.path().join("nope.toml"))
            .load()
            .unwrap();

        assert!(requests.is_empty());
    }

    #[test]
    fn saving_an_http_request_then_loading_round_trips_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = HttpRequestStore::at(dir.path().join("http_requests.toml"));
        let request = SavedHttpRequest {
            name: "list users".to_string(),
            method: "GET".to_string(),
            url: "https://api.example.com/users".to_string(),
            headers: "Accept: application/json".to_string(),
            body: String::new(),
        };

        store.save(std::slice::from_ref(&request)).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, vec![request]);
    }

    fn test_http_requests(dir: &std::path::Path) -> HttpRequests {
        HttpRequests {
            store: HttpRequestStore::at(dir.join("http_requests.toml")),
            list: std::sync::RwLock::new(Vec::new()),
        }
    }

    fn sample_request(name: &str) -> SavedHttpRequest {
        SavedHttpRequest {
            name: name.to_string(),
            method: "GET".to_string(),
            url: "https://api.example.com".to_string(),
            headers: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn saving_under_an_existing_name_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let requests = test_http_requests(dir.path());
        requests.save(sample_request("q"));

        let mut updated = sample_request("q");
        updated.method = "POST".to_string();
        requests.save(updated);

        let all = requests.all();
        assert_eq!(
            all.len(),
            1,
            "saving under a used name must update, not duplicate"
        );
        assert_eq!(all[0].method, "POST");
    }

    #[test]
    fn deleting_removes_the_matching_entry() {
        let dir = tempfile::tempdir().unwrap();
        let requests = test_http_requests(dir.path());
        requests.save(sample_request("a"));
        requests.save(sample_request("b"));

        requests.delete("a");

        let names: Vec<String> = requests.all().into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["b".to_string()]);
    }
}
