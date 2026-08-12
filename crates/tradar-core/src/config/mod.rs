//! Application configuration: `~/.config/tradar/config.toml`, holding the
//! theme overrides and key bindings. Both are optional -- a missing file
//! means "use the built-in defaults", which is the common case.
//!
//! ```toml
//! [theme]
//! border-focused = "#89b4fa"
//! error = "red"
//!
//! [keymap.global]
//! new-tab = "ctrl-n"          # one binding
//!
//! [keymap.list]
//! move-down = ["j", "down"]   # or several
//! move-top = "gg"             # or a two-key sequence
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::keymap::{Keymap, set_keymap};
use crate::theme::{Theme, set_theme};

/// A command can be bound to one key or a list of them; TOML users
/// shouldn't have to write `["j"]` for the common single-binding case.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(key) => vec![key],
            Self::Many(keys) => keys,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    theme: HashMap<String, String>,
    #[serde(default)]
    keymap: HashMap<String, HashMap<String, Keys>>,
}

pub fn default_config_path() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tradar").ok_or_else(|| {
        anyhow::anyhow!("could not determine a config directory for this platform")
    })?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Loads `path` (if it exists) and installs the resulting theme and keymap
/// process-wide. Call once, before the first frame is drawn.
///
/// A malformed config is returned as an error rather than being ignored --
/// the caller decides whether to warn and continue with defaults or bail,
/// but silently dropping a user's config is never right.
pub fn init(path: &std::path::Path) -> anyhow::Result<()> {
    let (theme, keymap) = load(path)?;
    set_theme(theme);
    set_keymap(keymap);
    Ok(())
}

fn load(path: &std::path::Path) -> anyhow::Result<(Theme, Keymap)> {
    let mut theme = Theme::default();
    let mut keymap = Keymap::default();

    if !path.exists() {
        return Ok((theme, keymap));
    }

    let contents = std::fs::read_to_string(path)?;
    let file: ConfigFile = toml::from_str(&contents)?;

    theme.apply_overrides(&file.theme)?;

    let keymap_overrides: HashMap<String, HashMap<String, Vec<String>>> = file
        .keymap
        .into_iter()
        .map(|(context, commands)| {
            let commands = commands
                .into_iter()
                .map(|(command, keys)| (command, keys.into_vec()))
                .collect();
            (context, commands)
        })
        .collect();
    keymap.apply_overrides(&keymap_overrides)?;

    Ok((theme, keymap))
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::Color;

    use super::*;
    use crate::keymap::{Command, Context, KeyPress, Resolution};

    fn write_config(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn a_missing_config_file_yields_the_defaults() {
        let dir = tempfile::tempdir().unwrap();

        let (theme, keymap) = load(&dir.path().join("nope.toml")).unwrap();

        assert_eq!(theme, Theme::default());
        assert_eq!(keymap, Keymap::default());
    }

    #[test]
    fn an_empty_config_file_yields_the_defaults() {
        let (_dir, path) = write_config("");

        let (theme, keymap) = load(&path).unwrap();

        assert_eq!(theme, Theme::default());
        assert_eq!(keymap, Keymap::default());
    }

    #[test]
    fn theme_overrides_are_applied() {
        let (_dir, path) = write_config("[theme]\nerror = \"#ff0000\"\n");

        let (theme, _) = load(&path).unwrap();

        assert_eq!(theme.error, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn a_command_can_be_bound_to_one_key_or_a_list() {
        let (_dir, path) = write_config(
            "[keymap.global]\nnew-tab = \"ctrl-n\"\n\n[keymap.list]\nmove-down = [\"n\", \"down\"]\n",
        );

        let (_, keymap) = load(&path).unwrap();

        let mut pending = None;
        assert_eq!(
            keymap.resolve(
                Context::Global,
                &mut pending,
                KeyPress::new(KeyCode::Char('n'), KeyModifiers::CONTROL)
            ),
            Resolution::Command(Command::NewTab)
        );
        assert_eq!(
            keymap.resolve(
                Context::List,
                &mut pending,
                KeyPress::new(KeyCode::Char('n'), KeyModifiers::NONE)
            ),
            Resolution::Command(Command::MoveDown)
        );
        assert_eq!(
            keymap.resolve(
                Context::List,
                &mut pending,
                KeyPress::new(KeyCode::Down, KeyModifiers::NONE)
            ),
            Resolution::Command(Command::MoveDown)
        );
    }

    #[test]
    fn a_broken_config_is_an_error_rather_than_being_ignored() {
        let (_dir, path) = write_config("[theme]\nnot-a-role = \"red\"\n");

        let err = load(&path).unwrap_err();

        assert!(err.to_string().contains("unknown theme key"), "{err}");
    }

    #[test]
    fn default_config_path_ends_with_config_toml() {
        let path = default_config_path().unwrap();

        assert_eq!(path.file_name().unwrap(), "config.toml");
    }
}
