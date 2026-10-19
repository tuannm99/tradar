use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
