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

pub use tradar_query::{Candidate, CandidateKind, CompletionSource};

#[cfg(test)]
use crate::query_driver::{self, SchemaInfo};

/// Longest list shown at once. A popup taller than this is unreadable and
/// covers the query you're writing.
const MAX_VISIBLE: usize = 8;

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
                        indexed: false,
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

    /// Cassandra-shaped: `list_schema` always names an entry
    /// `<keyspace>.<table>`, never bare, unlike Postgres's own
    /// `qualify_colliding_names` (bare unless two schemas collide).
    fn keyspace_qualified_source() -> CompletionSource {
        CompletionSource::new(
            &[],
            &[
                SchemaInfo {
                    name: "demo.users".to_string(),
                    columns: vec![ColumnInfo::new("id", "uuid")],
                    kind: None,
                    ttl: None,
                    schema: Some("demo".to_string()),
                    object_kind: None,
                },
                SchemaInfo {
                    name: "demo.usage_log".to_string(),
                    columns: vec![],
                    kind: None,
                    ttl: None,
                    schema: Some("demo".to_string()),
                    object_kind: None,
                },
                SchemaInfo {
                    name: "other.users".to_string(),
                    columns: vec![],
                    kind: None,
                    ttl: None,
                    schema: Some("other".to_string()),
                    object_kind: None,
                },
            ],
        )
    }

    #[test]
    fn schema_qualified_table_context_offers_unqualified_names_within_that_schema() {
        let matches = keyspace_qualified_source().matches_in_context(
            "us",
            &query_driver::CompletionContext::SchemaQualifiedTable {
                schema: "demo".to_string(),
            },
        );

        // Unqualified -- `demo.` is already typed, so the candidate must
        // be just what replaces "us", not the whole qualified name again.
        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, vec!["usage_log", "users"]);
        assert!(matches.iter().all(|c| c.kind == CandidateKind::Table));
    }

    #[test]
    fn schema_qualified_table_context_is_case_insensitive_on_the_schema_name() {
        let matches = keyspace_qualified_source().matches_in_context(
            "us",
            &query_driver::CompletionContext::SchemaQualifiedTable {
                schema: "DEMO".to_string(),
            },
        );

        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn schema_qualified_table_context_falls_back_to_the_flat_list_for_an_unknown_schema() {
        let matches = keyspace_qualified_source().matches_in_context(
            "us",
            &query_driver::CompletionContext::SchemaQualifiedTable {
                schema: "nope".to_string(),
            },
        );

        assert_eq!(
            matches,
            keyspace_qualified_source().matches("us"),
            "no entry qualified by an unknown schema -- same answer as no context at all"
        );
    }

    /// A Mongo-shaped vocabulary, deliberately picked so an unscoped
    /// `matches("f")` would rank a field/collection ahead of the method
    /// name you're actually typing -- `MongoMethod` exists to fix exactly
    /// that.
    fn mongo_source() -> CompletionSource {
        CompletionSource::new(
            &["find", "findOne", "aggregate"],
            &[SchemaInfo {
                name: "foo_collection".to_string(),
                columns: vec![ColumnInfo::new("favorite_color", "String")],
                kind: None,
                ttl: None,
                schema: None,
                object_kind: None,
            }],
        )
    }

    #[test]
    fn mongo_method_context_offers_only_methods_even_though_a_field_and_collection_also_match() {
        let matches =
            mongo_source().matches_in_context("f", &query_driver::CompletionContext::MongoMethod);

        let names: Vec<&str> = matches.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(names, vec!["find", "findOne"]);
        assert!(
            matches.iter().all(|c| c.kind == CandidateKind::Keyword),
            "a collection or field name is never legal here"
        );
    }

    #[test]
    fn mongo_method_context_is_empty_when_nothing_starts_with_the_prefix() {
        assert!(
            mongo_source()
                .matches_in_context("zzz", &query_driver::CompletionContext::MongoMethod)
                .is_empty()
        );
    }
}
