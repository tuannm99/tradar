//! Autocomplete for the query editor: the candidate list, how a typed
//! prefix is matched against it, and the popup that shows the matches.
//!
//! Candidates come from two places, neither of which this module knows the
//! shape of: the driver's own vocabulary (`QueryDriver::keywords`, so
//! Redis offers Redis commands and Postgres offers SQL) and the connected
//! schema (table and column names). That's what makes the suggestions
//! dialect-specific without `tradar-query-workbench` knowing which
//! databases exist.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState};

use tradar_core::theme::theme;
use tradar_core::ui;

use crate::query_driver::SchemaInfo;

/// Longest list shown at once. A popup taller than this is unreadable and
/// covers the query you're writing.
const MAX_VISIBLE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Keyword,
    Table,
    Column,
}

impl CandidateKind {
    /// The one-letter tag shown beside a suggestion, so "id" as a column
    /// is distinguishable from "id" as a table.
    fn tag(self) -> &'static str {
        match self {
            Self::Keyword => "k",
            Self::Table => "t",
            Self::Column => "c",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub kind: CandidateKind,
}

/// Everything completable for the current connection, built once when the
/// screen opens rather than per keystroke.
#[derive(Debug, Default, Clone)]
pub struct CompletionSource {
    candidates: Vec<Candidate>,
}

impl CompletionSource {
    pub fn new(keywords: &[&str], schema: &[SchemaInfo]) -> Self {
        let mut candidates: Vec<Candidate> = Vec::new();
        for name in keywords {
            candidates.push(Candidate {
                text: (*name).to_string(),
                kind: CandidateKind::Keyword,
            });
        }
        for entry in schema {
            candidates.push(Candidate {
                text: entry.name.clone(),
                kind: CandidateKind::Table,
            });
            for column in &entry.columns {
                // The same column name usually appears in several tables;
                // offering it once keeps the list short.
                if !candidates
                    .iter()
                    .any(|c| c.kind == CandidateKind::Column && c.text == column.name)
                {
                    candidates.push(Candidate {
                        text: column.name.clone(),
                        kind: CandidateKind::Column,
                    });
                }
            }
        }
        Self { candidates }
    }

    /// Matches for `prefix`, best first. Case-insensitive, because SQL is
    /// written in both cases and nobody wants to be told `select` doesn't
    /// match `SELECT`.
    ///
    /// Schema names rank above keywords: the keyword list is fixed and
    /// small, so the thing you can't guess -- your own table and column
    /// names -- is what's worth putting under the cursor.
    pub fn matches(&self, prefix: &str) -> Vec<Candidate> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let needle = prefix.to_ascii_lowercase();
        let mut matches: Vec<&Candidate> = self
            .candidates
            .iter()
            .filter(|c| {
                let text = c.text.to_ascii_lowercase();
                text.starts_with(&needle) && text != needle
            })
            .collect();
        matches.sort_by_key(|c| {
            let rank = match c.kind {
                CandidateKind::Table => 0,
                CandidateKind::Column => 1,
                CandidateKind::Keyword => 2,
            };
            (rank, c.text.to_ascii_lowercase())
        });
        matches.into_iter().cloned().collect()
    }
}

/// The visible suggestion list. Present only while there's something to
/// suggest -- `QueryScreenComponent` drops it as soon as `matches` is
/// empty, so "no popup" and "no matches" are the same state.
pub struct CompletionPopup {
    items: Vec<Candidate>,
    selected: usize,
    state: ListState,
}

impl CompletionPopup {
    pub fn new(items: Vec<Candidate>) -> Self {
        Self {
            items,
            selected: 0,
            state: ListState::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Keeps the same selection where possible as the list is refiltered on
    /// each keystroke, so typing another character doesn't silently move
    /// the highlight onto a different word.
    pub fn set_items(&mut self, items: Vec<Candidate>) {
        let previous = self.selected_text().map(str::to_string);
        self.items = items;
        self.selected = previous
            .and_then(|text| self.items.iter().position(|c| c.text == text))
            .unwrap_or(0);
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.items.get(self.selected).map(|c| c.text.as_str())
    }

    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.items.len();
    }

    pub fn prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
    }

    /// Draws below the cursor when there's room, above it otherwise, and
    /// never off the right edge -- a popup you can't read is worse than
    /// none.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, cursor: (u16, u16)) {
        if self.items.is_empty() {
            return;
        }
        let width = self
            .items
            .iter()
            .map(|c| c.text.chars().count() + 4)
            .max()
            .unwrap_or(10)
            .clamp(12, 40) as u16;
        let height = (self.items.len().min(MAX_VISIBLE) as u16) + 2;

        let (cursor_x, cursor_y) = cursor;
        let x = cursor_x.min(area.right().saturating_sub(width));
        let below = cursor_y.saturating_add(1);
        let y = if below + height <= area.bottom() {
            below
        } else {
            cursor_y.saturating_sub(height)
        };
        let popup = Rect {
            x,
            y,
            width: width.min(area.width),
            height: height.min(area.height),
        };

        let theme = theme();
        let items: Vec<ListItem> = self
            .items
            .iter()
            .map(|candidate| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {} ", candidate.kind.tag()),
                        Style::default().fg(theme.text_dim),
                    ),
                    Span::styled(candidate.text.clone(), Style::default().fg(theme.text)),
                ]))
            })
            .collect();

        self.state.select(Some(self.selected));
        frame.render_widget(Clear, popup);
        frame.render_stateful_widget(
            List::new(items)
                .block(ui::panel("", true))
                .highlight_style(ui::selection_style().add_modifier(Modifier::BOLD)),
            popup,
            &mut self.state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_driver::ColumnInfo;

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo {
                name: "users".to_string(),
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        type_name: "INTEGER".to_string(),
                    },
                    ColumnInfo {
                        name: "user_email".to_string(),
                        type_name: "TEXT".to_string(),
                    },
                ],
            },
            SchemaInfo {
                name: "orders".to_string(),
                columns: vec![ColumnInfo {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                }],
            },
        ]
    }

    fn source() -> CompletionSource {
        CompletionSource::new(&["SELECT", "SET", "UPDATE"], &schema())
    }

    #[test]
    fn matching_is_case_insensitive() {
        let matches = source().matches("sel");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "SELECT");
    }

    #[test]
    fn schema_names_rank_above_keywords() {
        let matches = source().matches("u");

        // users (table) and user_email (column) before UPDATE (keyword).
        let order: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(order, vec!["users", "user_email", "UPDATE"]);
    }

    #[test]
    fn a_column_shared_by_two_tables_is_offered_once() {
        let matches = source().matches("id");

        assert_eq!(
            matches.len(),
            0,
            "'id' matches itself exactly, so nothing to add"
        );

        let matches = source().matches("i");
        assert_eq!(matches.len(), 1, "one 'id', not one per table");
        assert_eq!(matches[0].kind, CandidateKind::Column);
    }

    #[test]
    fn an_exact_match_is_not_offered() {
        // Nothing to complete once the word is already typed in full.
        assert!(source().matches("users").is_empty());
        assert!(source().matches("SELECT").is_empty());
        assert!(source().matches("select").is_empty());
    }

    #[test]
    fn an_empty_prefix_suggests_nothing() {
        assert!(source().matches("").is_empty());
    }

    #[test]
    fn a_driver_without_keywords_still_completes_schema_names() {
        let source = CompletionSource::new(&[], &schema());

        let matches = source.matches("or");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "orders");
    }

    #[test]
    fn refiltering_keeps_the_selection_on_the_same_word() {
        let mut popup = CompletionPopup::new(source().matches("u"));
        popup.next();
        let selected = popup.selected_text().unwrap().to_string();

        popup.set_items(source().matches("us"));

        assert_eq!(
            popup.selected_text(),
            Some(selected.as_str()),
            "typing another character must not move the highlight elsewhere"
        );
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut popup = CompletionPopup::new(source().matches("u"));
        assert_eq!(popup.selected_text(), Some("users"));

        popup.prev();
        assert_eq!(popup.selected_text(), Some("UPDATE"), "wraps to the end");

        popup.next();
        assert_eq!(popup.selected_text(), Some("users"), "and back round");
    }
}
