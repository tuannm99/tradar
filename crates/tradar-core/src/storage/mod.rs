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
    /// `SavedConnection::name` values, in tab order. Matched by name against
    /// whatever's in `connections.toml` on the next run -- a name that no
    /// longer exists there is silently skipped, since the connection it
    /// pointed to may have been renamed or removed.
    #[serde(default)]
    pub tabs: Vec<String>,
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
            tabs: vec!["local sqlite".to_string(), "local postgres".to_string()],
        };

        store.save(&state).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, state);
    }
}
