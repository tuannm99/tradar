//! The server's log file. The server outlives the editor and is usually
//! started detached by the plugin, so stderr goes nowhere -- everything
//! worth knowing (requests, failures, clients coming and going) is written
//! to a file instead, and `:TradarLog` opens it.
//!
//! `TRADAR_LOG_FILE` overrides the path, `TRADAR_LOG` the level
//! (`off|error|warn|info|debug|trace`, default `info`). Query text and
//! passwords are never logged; a failure's message is, cut to its first
//! line -- a database may quote a fragment of the statement in it.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::filter::LevelFilter;

/// Past this size the file is moved to `<name>.1` at startup (one
/// generation kept), so a long-lived install cannot grow it without bound.
const MAX_BYTES: u64 = 5 << 20;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// `$TRADAR_LOG_FILE`, else `$XDG_STATE_HOME/tradar/server.log`, else
/// `~/.local/state/tradar/server.log`.
pub fn default_log_path() -> PathBuf {
    if let Some(path) = std::env::var_os("TRADAR_LOG_FILE").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(std::env::temp_dir);
    state.join("tradar").join("server.log")
}

/// Where this process logs, once `init` succeeded; `None` when logging is
/// off or the file could not be opened.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().and_then(|p| p.as_deref())
}

fn level() -> LevelFilter {
    match std::env::var("TRADAR_LOG")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "off" | "none" => LevelFilter::OFF,
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    }
}

/// Starts logging to the log file. Never fatal: a server that cannot write
/// its log still serves queries.
pub fn init() -> Option<PathBuf> {
    let path = default_log_path();
    let level = level();
    let opened = (level != LevelFilter::OFF)
        .then(|| open(&path))
        .and_then(Result::ok);
    let Some(file) = opened else {
        let _ = LOG_PATH.set(None);
        return None;
    };
    let installed = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_target(false)
        .with_writer(Mutex::new(file))
        .try_init()
        .is_ok();
    let _ = LOG_PATH.set(installed.then(|| path.clone()));
    installed.then_some(path)
}

fn open(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let _ = std::fs::rename(path, rotated);
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// A failure message for the log: first line only, bounded.
pub fn brief(message: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(300) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_string(),
    }
}
