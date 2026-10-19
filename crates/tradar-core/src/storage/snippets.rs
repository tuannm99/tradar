use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
pub(crate) struct SnippetsFile {
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
    pub(super) store: SnippetStore,
    pub(super) list: std::sync::RwLock<Vec<SavedSnippet>>,
}

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

pub(crate) static SNIPPETS: std::sync::OnceLock<Snippets> = std::sync::OnceLock::new();
