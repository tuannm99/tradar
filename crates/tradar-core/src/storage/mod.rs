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
}
