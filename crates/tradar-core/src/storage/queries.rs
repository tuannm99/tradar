use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub(crate) fn move_dir_if_safe(old: &std::path::Path, new: &std::path::Path) {
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
    pub(super) dir: PathBuf,
    pub(super) store: RecentStore,
    pub(super) recent: std::sync::RwLock<Vec<String>>,
}

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

/// Longest the recent list gets. Past this it stops being "recent" and
/// starts being a second, worse file browser.
pub(crate) const MAX_RECENT: usize = 20;

pub(crate) static QUERY_FILES: std::sync::OnceLock<QueryFiles> = std::sync::OnceLock::new();
