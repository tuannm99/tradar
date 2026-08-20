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

use crate::query_driver::{self, SchemaInfo};

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
    /// `text.to_ascii_lowercase()`, computed once at build time rather
    /// than inside `matches()` -- which runs on every keystroke, over
    /// every candidate, and previously redid this same lowercasing twice
    /// per match (once to filter, once more in the sort key).
    text_lower: String,
}

impl Candidate {
    fn new(text: impl Into<String>, kind: CandidateKind) -> Self {
        let text = text.into();
        let text_lower = text.to_ascii_lowercase();
        Self {
            text,
            kind,
            text_lower,
        }
    }
}

/// Everything completable for the current connection, built once when the
/// screen opens rather than per keystroke.
#[derive(Debug, Default, Clone)]
pub struct CompletionSource {
    candidates: Vec<Candidate>,
    /// Kept alongside `candidates` (which flattens/dedupes across tables)
    /// for `matches_in_context`, which needs to know *which* table a
    /// column belongs to and which columns carry an FK -- neither survives
    /// the flattening above.
    schema: Vec<SchemaInfo>,
}

use query_driver::same_table;

impl CompletionSource {
    pub fn new(keywords: &[&str], schema: &[SchemaInfo]) -> Self {
        let mut candidates: Vec<Candidate> = Vec::new();
        for name in keywords {
            candidates.push(Candidate::new(*name, CandidateKind::Keyword));
        }
        // The same column name usually appears in several tables; offering
        // it once keeps the list short. A set rather than an `.any()` scan
        // over the growing candidate list, which would make this quadratic
        // in the number of columns for a schema with many of them.
        let mut seen_columns: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in schema {
            candidates.push(Candidate::new(entry.name.clone(), CandidateKind::Table));
            for column in &entry.columns {
                if seen_columns.insert(column.name.clone()) {
                    candidates.push(Candidate::new(column.name.clone(), CandidateKind::Column));
                }
            }
        }
        Self {
            candidates,
            schema: schema.to_vec(),
        }
    }

    /// `matches`, but adjusted for the SQL context the cursor is sitting
    /// in -- `alias.` completes to that table's own columns instead of the
    /// flattened global list, and a table name typed right after `JOIN`
    /// ranks a table with an FK to one already in the query first. Falls
    /// back to `matches` for `CompletionContext::None`, and for a
    /// `TableColumns`/`JoinTarget` whose table doesn't resolve against the
    /// connected schema (nothing more specific to offer than the flat
    /// list).
    pub fn matches_in_context(
        &self,
        prefix: &str,
        context: &query_driver::CompletionContext,
    ) -> Vec<Candidate> {
        let needle = prefix.to_ascii_lowercase();
        match context {
            query_driver::CompletionContext::TableColumns { table } => {
                let Some(entry) = self.schema.iter().find(|e| same_table(table, &e.name)) else {
                    return self.matches(prefix);
                };
                let mut matches: Vec<Candidate> = entry
                    .columns
                    .iter()
                    .filter(|c| {
                        let lower = c.name.to_ascii_lowercase();
                        lower.starts_with(&needle) && lower != needle
                    })
                    .map(|c| Candidate::new(c.name.clone(), CandidateKind::Column))
                    .collect();
                matches.sort_by(|a, b| a.text_lower.cmp(&b.text_lower));
                matches
            }
            query_driver::CompletionContext::JoinTarget { known_tables } => {
                // A table is "related" if it has an FK to a known table,
                // or a known table has an FK to it -- either direction is
                // a real join condition, and there's no reason to only
                // offer one side of it.
                let related = |entry: &SchemaInfo| -> bool {
                    entry.columns.iter().any(|c| {
                        c.foreign_key
                            .as_ref()
                            .is_some_and(|fk| known_tables.iter().any(|t| same_table(t, &fk.table)))
                    }) || known_tables.iter().any(|t| {
                        self.schema
                            .iter()
                            .find(|e| same_table(t, &e.name))
                            .is_some_and(|kt| {
                                kt.columns.iter().any(|c| {
                                    c.foreign_key
                                        .as_ref()
                                        .is_some_and(|fk| same_table(&fk.table, &entry.name))
                                })
                            })
                    })
                };
                let mut matches: Vec<(bool, Candidate)> = self
                    .schema
                    .iter()
                    .filter(|e| {
                        let lower = e.name.to_ascii_lowercase();
                        lower.starts_with(&needle) && lower != needle
                    })
                    // A table already in the FROM/JOIN list isn't worth
                    // suggesting again -- a deliberate self-join still
                    // works, just by typing the full name rather than
                    // picking it from the popup.
                    .filter(|e| !known_tables.iter().any(|t| same_table(t, &e.name)))
                    .map(|e| {
                        (
                            related(e),
                            Candidate::new(e.name.clone(), CandidateKind::Table),
                        )
                    })
                    .collect();
                matches.sort_by(|(a_related, a), (b_related, b)| {
                    (!a_related, &a.text_lower).cmp(&(!b_related, &b.text_lower))
                });
                matches.into_iter().map(|(_, c)| c).collect()
            }
            query_driver::CompletionContext::None => self.matches(prefix),
        }
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
            .filter(|c| c.text_lower.starts_with(&needle) && c.text_lower != needle)
            .collect();
        matches.sort_by_key(|c| {
            let rank = match c.kind {
                CandidateKind::Table => 0,
                CandidateKind::Column => 1,
                CandidateKind::Keyword => 2,
            };
            (rank, c.text_lower.clone())
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
                    ColumnInfo::new("id", "INTEGER"),
                    ColumnInfo::new("user_email", "TEXT"),
                ],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
            },
            SchemaInfo {
                name: "orders".to_string(),
                columns: vec![ColumnInfo::new("id", "INTEGER")],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
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

    /// `orders.user_id` references `users.id`; `products` has no relation
    /// to either -- enough to tell "ranked first" from "not offered at
    /// all" and from "everything else, alphabetically".
    fn schema_with_fk() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo {
                name: "users".to_string(),
                columns: vec![
                    ColumnInfo::new("id", "INTEGER"),
                    ColumnInfo::new("name", "TEXT"),
                ],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
            },
            SchemaInfo {
                name: "orders".to_string(),
                columns: vec![
                    ColumnInfo::new("id", "INTEGER"),
                    ColumnInfo {
                        name: "user_id".to_string(),
                        type_name: "INTEGER".to_string(),
                        primary_key: false,
                        foreign_key: Some(query_driver::ForeignKeyRef {
                            table: "users".to_string(),
                            column: "id".to_string(),
                        }),
                    },
                ],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
            },
            SchemaInfo {
                name: "products".to_string(),
                columns: vec![ColumnInfo::new("id", "INTEGER")],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
            },
        ]
    }

    fn source_with_fk() -> CompletionSource {
        CompletionSource::new(&["SELECT", "JOIN"], &schema_with_fk())
    }

    #[test]
    fn table_columns_context_offers_only_that_table_s_columns() {
        let matches = source_with_fk().matches_in_context(
            "",
            &query_driver::CompletionContext::TableColumns {
                table: "orders".to_string(),
            },
        );

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            names,
            vec!["id", "user_id"],
            "only orders' own columns, not users' or products'"
        );
        assert!(matches.iter().all(|c| c.kind == CandidateKind::Column));
    }

    #[test]
    fn table_columns_context_still_filters_by_the_typed_prefix() {
        let matches = source_with_fk().matches_in_context(
            "u",
            &query_driver::CompletionContext::TableColumns {
                table: "orders".to_string(),
            },
        );

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, vec!["user_id"]);
    }

    #[test]
    fn table_columns_context_falls_back_to_the_flat_list_for_an_unknown_table() {
        let matches = source_with_fk().matches_in_context(
            "u",
            &query_driver::CompletionContext::TableColumns {
                table: "no_such_table".to_string(),
            },
        );

        assert!(
            !matches.is_empty(),
            "an unresolved table falls back to matches(), not to nothing"
        );
    }

    #[test]
    fn join_target_context_ranks_the_fk_related_table_first() {
        let matches = source_with_fk().matches_in_context(
            "",
            &query_driver::CompletionContext::JoinTarget {
                known_tables: vec!["orders".to_string()],
            },
        );

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            names,
            vec!["users", "products"],
            "users (FK from orders) ranks before products (unrelated)"
        );
    }

    #[test]
    fn join_target_context_relation_works_in_either_direction() {
        // users has no FK column of its own, but orders has one pointing
        // at it -- still "related" from the other side.
        let matches = source_with_fk().matches_in_context(
            "",
            &query_driver::CompletionContext::JoinTarget {
                known_tables: vec!["users".to_string()],
            },
        );

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, vec!["orders", "products"]);
    }

    #[test]
    fn join_target_context_with_no_relations_falls_back_to_alphabetical() {
        let matches = source_with_fk().matches_in_context(
            "",
            &query_driver::CompletionContext::JoinTarget {
                known_tables: vec!["products".to_string()],
            },
        );

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, vec!["orders", "users"]);
    }

    #[test]
    fn none_context_behaves_like_plain_matches() {
        assert_eq!(
            source_with_fk().matches_in_context("u", &query_driver::CompletionContext::None),
            source_with_fk().matches("u")
        );
    }
}
