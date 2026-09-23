//! Read-only comparison of two connections' schemas (columns/types), built
//! from `Component::outline()` -- the same opaque cross-screen contract the
//! navigator itself already uses, so this never has to know what a table
//! *is* for any given driver. See `docs/roadmap.md`'s "#3 Schema
//! diff/compare" entry for the scope this was chosen against: v1 only
//! compares what `outline()` already carries (column name, type, whether
//! it's a primary key) -- not index/constraint/default, and not any
//! generated DDL to reconcile them -- and only between two connections that
//! are already open, since `outline()` only exists once a screen is; nothing
//! here connects anything itself.
//!
//! Two layers, same split as the ERD overlay (`erd` in
//! `tradar-query-workbench`): `diff`/`render` below are pure functions over
//! `OutlineEntry`, unit-testable against exact strings. `SchemaDiffComponent`
//! at the bottom is the thin ratatui-facing `Component` -- but unlike the ERD
//! overlay (owned by one `QueryScreenComponent`, which only ever sees its
//! own tab's schema), a diff is inherently cross-connection, so it can't
//! live inside a single screen the same way. It's built once by
//! `RootComponent` (the only thing that sees every tab) from two already-
//! open tabs' `outline()`s and given a whole tab of its own -- static after
//! that, since nothing here reconnects or refreshes either side.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::action::{Action, Component, OutlineEntry};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, Hint};
use tradar_core::vim_list;

/// One table's columns, pulled out of a flat `outline()` -- see
/// `tables_of`.
struct TableColumns {
    name: String,
    /// (column name, display type).
    columns: Vec<(String, String)>,
}

/// Strips `outline()`'s "{type} pk" display convention back to a bare type
/// -- the format `push_table` in `tradar-query-workbench`'s
/// `query_screen.rs` builds for a primary-key column's `detail`. `detail`
/// is the only place a column's type lives in `OutlineEntry`, so a diff has
/// nothing else to compare against.
fn column_type(entry: &OutlineEntry) -> String {
    if entry.primary_key {
        entry
            .detail
            .strip_suffix(" pk")
            .unwrap_or(&entry.detail)
            .to_string()
    } else {
        entry.detail.clone()
    }
}

/// Walks a flat `outline()` into one entry per `is_object` row (a table,
/// collection, index...) plus its direct-child columns -- the same depth-
/// window idiom `NavigatorComponent::choose_snippet` uses (`take_while`
/// deeper, then `filter` exactly one level down), so a driver that groups
/// by schema/object-kind (Postgres/Cassandra/Mongo) is walked the same as
/// one that doesn't, without this module knowing why.
fn tables_of(outline: &[OutlineEntry]) -> Vec<TableColumns> {
    let mut tables = Vec::new();
    for (i, entry) in outline.iter().enumerate() {
        if !entry.is_object {
            continue;
        }
        let depth = entry.depth;
        let columns = outline[i + 1..]
            .iter()
            .take_while(|e| e.depth > depth)
            .filter(|e| e.depth == depth + 1)
            .map(|e| (e.label.clone(), column_type(e)))
            .collect();
        tables.push(TableColumns {
            name: entry.label.clone(),
            columns,
        });
    }
    tables
}

/// One table's column-level differences -- only built for a table present
/// on both sides. A table missing on one side is reported once, in
/// `SchemaDiff::tables_only_in_a`/`_b`, never duplicated here.
pub struct TableDiff {
    pub name: String,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
    /// (column name, type in A, type in B) -- present on both sides, but
    /// with a different type.
    pub type_changed: Vec<(String, String, String)>,
}

pub struct SchemaDiff {
    pub tables_only_in_a: Vec<String>,
    pub tables_only_in_b: Vec<String>,
    /// Only tables with an actual difference -- an identical table on both
    /// sides never gets an (empty) entry here.
    pub table_diffs: Vec<TableDiff>,
}

/// Compares `a` and `b`'s outlines table-by-table and column-by-column.
/// Order in the source outlines never matters: both sides are matched by
/// name, since a table at index 3 in one schema and index 7 in the other is
/// still the same table.
pub fn diff(a: &[OutlineEntry], b: &[OutlineEntry]) -> SchemaDiff {
    let tables_a = tables_of(a);
    let tables_b = tables_of(b);

    let tables_only_in_a: Vec<String> = tables_a
        .iter()
        .filter(|t| !tables_b.iter().any(|u| u.name == t.name))
        .map(|t| t.name.clone())
        .collect();
    let tables_only_in_b: Vec<String> = tables_b
        .iter()
        .filter(|t| !tables_a.iter().any(|u| u.name == t.name))
        .map(|t| t.name.clone())
        .collect();

    let mut table_diffs = Vec::new();
    for table_a in &tables_a {
        let Some(table_b) = tables_b.iter().find(|t| t.name == table_a.name) else {
            continue;
        };
        let only_in_a: Vec<String> = table_a
            .columns
            .iter()
            .filter(|(name, _)| !table_b.columns.iter().any(|(n, _)| n == name))
            .map(|(name, _)| name.clone())
            .collect();
        let only_in_b: Vec<String> = table_b
            .columns
            .iter()
            .filter(|(name, _)| !table_a.columns.iter().any(|(n, _)| n == name))
            .map(|(name, _)| name.clone())
            .collect();
        let type_changed: Vec<(String, String, String)> = table_a
            .columns
            .iter()
            .filter_map(|(name, type_a)| {
                let (_, type_b) = table_b.columns.iter().find(|(n, _)| n == name)?;
                (type_a != type_b).then(|| (name.clone(), type_a.clone(), type_b.clone()))
            })
            .collect();

        if only_in_a.is_empty() && only_in_b.is_empty() && type_changed.is_empty() {
            continue;
        }
        table_diffs.push(TableDiff {
            name: table_a.name.clone(),
            only_in_a,
            only_in_b,
            type_changed,
        });
    }

    SchemaDiff {
        tables_only_in_a,
        tables_only_in_b,
        table_diffs,
    }
}

/// Renders `d` as plain text lines -- a read-only report, no generated DDL
/// to reconcile the two sides (see this module's doc comment: v1
/// deliberately stops at listing differences).
pub fn render(d: &SchemaDiff, name_a: &str, name_b: &str) -> Vec<String> {
    if d.tables_only_in_a.is_empty() && d.tables_only_in_b.is_empty() && d.table_diffs.is_empty() {
        return vec!["No differences.".to_string()];
    }

    let mut lines = Vec::new();
    if !d.tables_only_in_a.is_empty() {
        lines.push(format!("Only in {name_a}:"));
        lines.extend(d.tables_only_in_a.iter().map(|t| format!("  - {t}")));
        lines.push(String::new());
    }
    if !d.tables_only_in_b.is_empty() {
        lines.push(format!("Only in {name_b}:"));
        lines.extend(d.tables_only_in_b.iter().map(|t| format!("  - {t}")));
        lines.push(String::new());
    }
    for t in &d.table_diffs {
        lines.push(format!("{}:", t.name));
        lines.extend(
            t.only_in_a
                .iter()
                .map(|c| format!("  + {c} (only in {name_a})")),
        );
        lines.extend(
            t.only_in_b
                .iter()
                .map(|c| format!("  + {c} (only in {name_b})")),
        );
        lines.extend(t.type_changed.iter().map(|(c, type_a, type_b)| {
            format!("  ~ {c}: {type_a} ({name_a}) vs {type_b} ({name_b})")
        }));
        lines.push(String::new());
    }
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

pub struct SchemaDiffComponent {
    lines: Vec<String>,
    scroll: usize,
    pending: Option<KeyPress>,
    visible_height: usize,
}

impl SchemaDiffComponent {
    pub fn new(
        name_a: &str,
        outline_a: &[OutlineEntry],
        name_b: &str,
        outline_b: &[OutlineEntry],
    ) -> Self {
        let d = diff(outline_a, outline_b);
        Self {
            lines: render(&d, name_a, name_b),
            scroll: 0,
            pending: None,
            visible_height: 0,
        }
    }
}

impl Component for SchemaDiffComponent {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        let Resolution::Command(command) =
            keymap().resolve_in(&[Context::List], &mut self.pending, key)
        else {
            return None;
        };
        if let Some(mv) = command.as_vim_move() {
            vim_list::apply(mv, &mut self.scroll, self.lines.len(), self.visible_height);
        }
        None
    }

    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }

    fn status_hints(&self) -> Vec<Hint> {
        let mut hints = Vec::new();
        hints.extend(ui::hint(Context::List, Command::MoveDown, "scroll"));
        hints
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        self.visible_height = area.height.saturating_sub(2) as usize;
        let visible: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.text))))
            .collect();
        let block = ui::panel("Schema diff", true);
        frame.render_widget(Paragraph::new(visible).block(block), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(name: &str, columns: &[(&str, &str, bool)]) -> Vec<OutlineEntry> {
        let mut entries = vec![OutlineEntry {
            depth: 0,
            label: name.to_string(),
            detail: String::new(),
            has_children: !columns.is_empty(),
            is_object: true,
            primary_key: false,
        }];
        for (col_name, type_name, pk) in columns {
            entries.push(OutlineEntry {
                depth: 1,
                label: col_name.to_string(),
                detail: if *pk {
                    format!("{type_name} pk")
                } else {
                    type_name.to_string()
                },
                has_children: false,
                is_object: false,
                primary_key: *pk,
            });
        }
        entries
    }

    #[test]
    fn identical_schemas_have_no_differences() {
        let a = table("users", &[("id", "INTEGER", true), ("name", "TEXT", false)]);
        let b = a.clone();

        let d = diff(&a, &b);

        assert!(d.tables_only_in_a.is_empty());
        assert!(d.tables_only_in_b.is_empty());
        assert!(d.table_diffs.is_empty());
        assert_eq!(render(&d, "A", "B"), vec!["No differences.".to_string()]);
    }

    #[test]
    fn a_table_missing_from_one_side_is_reported_once() {
        let mut a = table("users", &[("id", "INTEGER", true)]);
        a.extend(table("orders", &[("id", "INTEGER", true)]));
        let b = table("users", &[("id", "INTEGER", true)]);

        let d = diff(&a, &b);

        assert_eq!(d.tables_only_in_a, vec!["orders".to_string()]);
        assert!(d.tables_only_in_b.is_empty());
        assert!(
            d.table_diffs.is_empty(),
            "a table missing entirely isn't a column-level diff"
        );
    }

    #[test]
    fn a_missing_column_is_reported_on_the_side_it_s_missing_from() {
        let a = table(
            "users",
            &[("id", "INTEGER", true), ("email", "TEXT", false)],
        );
        let b = table("users", &[("id", "INTEGER", true)]);

        let d = diff(&a, &b);

        assert_eq!(d.table_diffs.len(), 1);
        assert_eq!(d.table_diffs[0].only_in_a, vec!["email".to_string()]);
        assert!(d.table_diffs[0].only_in_b.is_empty());
    }

    #[test]
    fn a_type_change_strips_the_pk_suffix_before_comparing() {
        let a = table("users", &[("id", "INTEGER", true)]);
        let b = table("users", &[("id", "BIGINT", true)]);

        let d = diff(&a, &b);

        assert_eq!(d.table_diffs.len(), 1);
        assert_eq!(
            d.table_diffs[0].type_changed,
            vec![(
                "id".to_string(),
                "INTEGER".to_string(),
                "BIGINT".to_string()
            )]
        );
    }

    #[test]
    fn column_order_never_matters() {
        let a = table("users", &[("id", "INTEGER", true), ("name", "TEXT", false)]);
        let b = table("users", &[("name", "TEXT", false), ("id", "INTEGER", true)]);

        let d = diff(&a, &b);

        assert!(
            d.table_diffs.is_empty(),
            "same columns, different order, no diff"
        );
    }

    #[test]
    fn render_lists_every_kind_of_difference() {
        let mut a = table(
            "users",
            &[("id", "INTEGER", true), ("nickname", "TEXT", false)],
        );
        a.extend(table("orders", &[("id", "INTEGER", true)]));
        let b = table("users", &[("id", "BIGINT", true), ("email", "TEXT", false)]);

        let d = diff(&a, &b);
        let text = render(&d, "dev", "prod").join("\n");

        assert!(text.contains("Only in dev:"));
        assert!(text.contains("orders"));
        assert!(text.contains("nickname (only in dev)"));
        assert!(text.contains("email (only in prod)"));
        assert!(text.contains("id: INTEGER (dev) vs BIGINT (prod)"));
    }

    #[test]
    fn component_scrolls_with_vim_keys() {
        let a = table(
            "users",
            &[
                ("id", "INTEGER", true),
                ("very_long_extra_column", "TEXT", false),
            ],
        );
        let b = table("users", &[("id", "INTEGER", true)]);
        let mut component = SchemaDiffComponent::new("dev", &a, "prod", &b);
        component.visible_height = 1;

        component.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE);

        assert_eq!(component.scroll, 1);
    }
}
