//! Local persistence for saved connections and app state, via the
//! `directories` crate's platform-appropriate config path.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedConnection {
    pub name: String,
    pub driver: DriverKind,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Sqlite,
    Elasticsearch,
    Redis,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    connections: Vec<SavedConnection>,
}

pub fn default_connections_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar")
        .ok_or_else(|| anyhow::anyhow!("could not determine a config directory for this platform"))?;
    Ok(dirs.config_dir().join("connections.toml"))
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
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        };

        store.save(std::slice::from_ref(&connection)).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded, vec![connection]);
    }
}
