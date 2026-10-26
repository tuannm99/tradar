//! The cheat sheet a connection offers: the driver's own built-in queries
//! (`QueryDriver::cheatsheet`) plus the user's, read from `cheatsheet.toml`
//! next to `connections.toml` on every call -- so an edit needs no restart.
use crate::protocol::{RpcError, str_param};
use serde_json::{Value, json};
use std::path::PathBuf;

use super::*;

/// One `[[entry]]` of the user's file.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UserEntry {
    pub driver: Option<String>,
    pub connection: Option<String>,
    pub category: String,
    pub title: String,
    pub text: String,
}

impl UserEntry {
    /// Whether this entry is meant for `connection`, which runs `driver`.
    fn applies_to(&self, connection: &str, driver: Option<&str>) -> bool {
        self.connection.as_deref().is_none_or(|c| c == connection)
            && self
                .driver
                .as_deref()
                .is_none_or(|d| Some(d.to_lowercase()).as_deref() == driver)
    }
}

/// The entries of a `cheatsheet.toml`, and one warning per thing wrong with
/// it (bad TOML, an entry with no title or text). A broken entry is skipped,
/// never fatal: the built-in queries still show.
pub(super) fn parse_user_entries(text: &str) -> (Vec<UserEntry>, Vec<String>) {
    let mut warnings = Vec::new();
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(e) => return (Vec::new(), vec![format!("cheatsheet.toml: {e}")]),
    };
    let Some(items) = table.get("entry") else {
        return (Vec::new(), warnings);
    };
    let Some(items) = items.as_array() else {
        return (
            Vec::new(),
            vec!["cheatsheet.toml: `entry` must be written as [[entry]] tables".to_string()],
        );
    };
    let mut entries = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let field = |name: &str| {
            item.get(name)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let (Some(title), Some(body)) = (field("title"), field("text")) else {
            warnings.push(format!(
                "cheatsheet.toml: entry {} needs a `title` and a `text`",
                i + 1
            ));
            continue;
        };
        entries.push(UserEntry {
            driver: field("driver"),
            connection: field("connection"),
            category: field("category").unwrap_or_else(|| "Mine".to_string()),
            title,
            text: body,
        });
    }
    (entries, warnings)
}

impl Server {
    /// `cheatsheet.toml`, beside the connections file.
    fn cheatsheet_path(&self) -> Option<PathBuf> {
        self.store
            .as_ref()
            .map(|s| s.path().with_file_name("cheatsheet.toml"))
    }

    /// `{entries: [{category, title, text, user}], warnings, path}` for a
    /// connected connection: the user's entries that apply to it first, then
    /// the driver's built-in ones. Nothing is sent to the database.
    pub(super) async fn cheatsheet(&self, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "connection")?;
        let driver = self.driver(params)?;
        let driver_id = self
            .saved()?
            .into_iter()
            .find(|c| c.name == name)
            .map(|c| c.driver.to_lowercase());

        let mut entries: Vec<Value> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let path = self.cheatsheet_path();
        if let Some(path) = &path {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let (user, problems) = parse_user_entries(&text);
                    warnings.extend(problems);
                    entries.extend(
                        user.iter()
                            .filter(|e| e.applies_to(name, driver_id.as_deref()))
                            .map(|e| {
                                json!({"category": e.category, "title": e.title, "text": e.text, "user": true})
                            }),
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warnings.push(format!("cheatsheet.toml: {e}")),
            }
        }
        entries.extend(driver.cheatsheet().iter().map(
            |e| json!({"category": e.category, "title": e.title, "text": e.text, "user": false}),
        ));
        Ok(json!({
            "entries": entries,
            "warnings": warnings,
            "path": path.map(|p| p.display().to_string()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_read_with_their_filters_and_a_default_category() {
        let (entries, warnings) = parse_user_entries(
            r#"
[[entry]]
driver = "Postgres"
connection = "prod"
category = "Orders"
title = " Last hour "
text = """
select 1
"""

[[entry]]
title = "Anything"
text = "select 2"
"#,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Last hour");
        assert_eq!(entries[0].text, "select 1");
        assert_eq!(entries[0].category, "Orders");
        assert_eq!(entries[1].category, "Mine");
        assert_eq!(entries[1].driver, None);
    }

    #[test]
    fn a_filter_limits_an_entry_to_its_connection_and_driver() {
        let (entries, _) = parse_user_entries(
            "[[entry]]\ndriver = \"Postgres\"\nconnection = \"prod\"\ntitle = \"t\"\ntext = \"x\"",
        );
        let e = &entries[0];
        assert!(e.applies_to("prod", Some("postgres")));
        assert!(!e.applies_to("dev", Some("postgres")));
        assert!(!e.applies_to("prod", Some("mysql")));
        let (open, _) = parse_user_entries("[[entry]]\ntitle = \"t\"\ntext = \"x\"");
        assert!(open[0].applies_to("any", Some("mysql")));
    }

    #[test]
    fn a_broken_entry_is_skipped_with_a_warning_and_the_rest_stay() {
        let (entries, warnings) = parse_user_entries(
            "[[entry]]\ntitle = \"no text\"\n\n[[entry]]\ntitle = \"ok\"\ntext = \"select 1\"",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("entry 1"), "{warnings:?}");
    }

    #[test]
    fn invalid_toml_is_one_warning_not_an_error() {
        let (entries, warnings) = parse_user_entries("[[entry]\ntitle =");
        assert!(entries.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("cheatsheet.toml:"), "{warnings:?}");
    }
}
