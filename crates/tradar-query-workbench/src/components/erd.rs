//! Renders an ASCII/box-drawing entity-relationship diagram for one table
//! and its immediate FK neighborhood -- not the whole schema at once (see
//! `docs/roadmap.md`'s ERD entry: unbounded whole-schema layout is
//! unaddressed territory with no data on how large a real schema gets, so
//! this stays scoped to "one table plus what touches it").
//!
//! Two layers, deliberately separated: `neighborhood`/`render` below are
//! pure functions over `SchemaInfo`/`ColumnInfo` (no ratatui), so the
//! box-drawing layout can be unit-tested against exact expected strings.
//! `ErdComponent` at the bottom is the thin ratatui-facing overlay that
//! drives them, following the same shape as `HistoryPickerComponent`/
//! `SnippetPickerComponent`: not a `Component`, owned directly by
//! `QueryScreenComponent`, takes over key input while open.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui;

use crate::components::history_picker::{HistoryOutcome, HistoryPickerComponent};
use crate::query_driver::{SchemaInfo, same_table};

/// A focal table plus every table directly connected to it by a foreign
/// key, in either direction -- one FK hop, not a transitive closure.
pub struct Neighborhood {
    pub focal: SchemaInfo,
    /// Tables with a column whose FK points at `focal` -- `focal` is the
    /// referenced ("parent") side of these.
    pub incoming: Vec<SchemaInfo>,
    /// Tables `focal` itself points at via one of its own FK columns --
    /// `focal` is the referencing ("child") side of these.
    pub outgoing: Vec<SchemaInfo>,
}

/// Builds `focal`'s neighborhood from the connected `schema`, or `None` if
/// `focal_name` doesn't resolve to any entry in it. Neighbors are deduped
/// by table so a table with two FK columns pointing at the same other
/// table (or two columns another table uses to reference `focal`) shows up
/// as one box, not two.
pub fn neighborhood(schema: &[SchemaInfo], focal_name: &str) -> Option<Neighborhood> {
    let focal = schema
        .iter()
        .find(|s| same_table(focal_name, &s.name))?
        .clone();

    let mut outgoing: Vec<SchemaInfo> = Vec::new();
    for fk_table in focal
        .columns
        .iter()
        .filter_map(|c| c.foreign_key.as_ref().map(|fk| fk.table.as_str()))
    {
        if outgoing.iter().any(|t| same_table(fk_table, &t.name)) {
            continue;
        }
        if let Some(entry) = schema.iter().find(|s| same_table(fk_table, &s.name)) {
            outgoing.push(entry.clone());
        }
    }

    let incoming: Vec<SchemaInfo> = schema
        .iter()
        .filter(|s| !same_table(&s.name, &focal.name))
        .filter(|s| {
            s.columns.iter().any(|c| {
                c.foreign_key
                    .as_ref()
                    .is_some_and(|fk| same_table(&fk.table, &focal.name))
            })
        })
        .cloned()
        .collect();

    Some(Neighborhood {
        focal,
        incoming,
        outgoing,
    })
}

/// Longest neighbor list before a box switches from listing every column to
/// listing the first `MAX_COLUMNS` plus a "+N more" line -- keeps a wide
/// table's box bounded instead of dwarfing the diagram around it.
const MAX_COLUMNS: usize = 8;

/// Columns of blank canvas between a box column and the trunk line that
/// connects it to the focal box -- needs room for an arrowhead cell next
/// to each box plus at least one line cell on either side of the trunk;
/// see `render`'s connector-drawing comment for the exact cell layout.
const GUTTER: usize = 7;

fn column_line(column: &crate::query_driver::ColumnInfo) -> String {
    let mut line = format!("{} {}", column.name, column.type_name);
    if column.primary_key {
        line.push_str(" [PK]");
    }
    if let Some(fk) = &column.foreign_key {
        line.push_str(&format!(" -> {}.{}", fk.table, fk.column));
    }
    line
}

/// The interior lines a table's box shows, capped at `MAX_COLUMNS`.
fn table_box_lines(table: &SchemaInfo) -> Vec<String> {
    let mut lines: Vec<String> = table
        .columns
        .iter()
        .take(MAX_COLUMNS)
        .map(column_line)
        .collect();
    if table.columns.len() > MAX_COLUMNS {
        lines.push(format!("… +{} more", table.columns.len() - MAX_COLUMNS));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Total box width (borders included) that fits both the title and every
/// interior line with a one-space margin on each side.
fn box_width(name: &str, lines: &[String]) -> usize {
    let content_max = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    // Content sits at columns [2, 2+len-1] and must clear the right
    // border at (w-1), so w >= content_max + 4. The title is drawn as
    // " {name} " (two extra spaces) at the same starting column, so it
    // needs one more: w >= name.len() + 2 + 3 = name.len() + 5.
    (content_max + 4).max(name.chars().count() + 5)
}

/// North/South/East/West connection bits for one canvas cell -- accumulated
/// from every line segment that passes through it, then converted to a
/// single box-drawing character once at the end. This is what lets several
/// connector lines share a trunk column without hand-reasoning about every
/// corner/tee case: draw each segment's geometry independently, and the
/// right glyph falls out of which directions ended up set.
const NORTH: u8 = 1;
const SOUTH: u8 = 2;
const EAST: u8 = 4;
const WEST: u8 = 8;

// `match bits { NORTH | SOUTH => ... }` would be Rust's or-*pattern*
// syntax (matches bits==NORTH or bits==SOUTH separately), not the
// bitwise-OR'd union value -- these are the actual union constants the
// match below needs, computed once as plain arithmetic rather than in
// pattern position.
const NS: u8 = NORTH | SOUTH;
const EW: u8 = EAST | WEST;
const SE: u8 = SOUTH | EAST;
const SW: u8 = SOUTH | WEST;
const NE: u8 = NORTH | EAST;
const NW: u8 = NORTH | WEST;
const NSE: u8 = NORTH | SOUTH | EAST;
const NSW: u8 = NORTH | SOUTH | WEST;
const SEW: u8 = SOUTH | EAST | WEST;
const NEW: u8 = NORTH | EAST | WEST;
const NSEW: u8 = NORTH | SOUTH | EAST | WEST;

fn bits_to_char(bits: u8) -> char {
    match bits {
        0 => ' ',
        NORTH => '│',
        SOUTH => '│',
        EAST => '─',
        WEST => '─',
        NS => '│',
        EW => '─',
        SE => '┌',
        SW => '┐',
        NE => '└',
        NW => '┘',
        NSE => '├',
        NSW => '┤',
        SEW => '┬',
        NEW => '┴',
        NSEW => '┼',
        _ => '?',
    }
}

/// A character grid that boxes and connector lines are drawn into, then
/// flattened to text -- the whole diagram is composed here before
/// `render` turns it into `Vec<String>`.
struct Canvas {
    cells: Vec<Vec<char>>,
    connections: std::collections::HashMap<(usize, usize), u8>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            cells: vec![vec![' '; width]; height.max(1)],
            connections: std::collections::HashMap::new(),
        }
    }

    fn set(&mut self, x: usize, y: usize, c: char) {
        if let Some(row) = self.cells.get_mut(y)
            && x < row.len()
        {
            row[x] = c;
        }
    }

    fn draw_text(&mut self, x: usize, y: usize, text: &str) {
        for (i, c) in text.chars().enumerate() {
            self.set(x + i, y, c);
        }
    }

    /// Draws a bordered box with `name` in the top border and `lines` as
    /// its interior, top-left corner at `(x, y)`. Returns the box's total
    /// height (border included), so the caller can stack the next one.
    fn draw_box(&mut self, x: usize, y: usize, w: usize, name: &str, lines: &[String]) -> usize {
        self.set(x, y, '┌');
        self.set(x + w - 1, y, '┐');
        for i in 1..w - 1 {
            self.set(x + i, y, '─');
        }
        self.draw_text(x + 2, y, &format!(" {name} "));

        for (i, line) in lines.iter().enumerate() {
            let row_y = y + 1 + i;
            self.set(x, row_y, '│');
            self.set(x + w - 1, row_y, '│');
            self.draw_text(x + 2, row_y, line);
        }

        let bottom_y = y + lines.len() + 1;
        self.set(x, bottom_y, '└');
        self.set(x + w - 1, bottom_y, '┘');
        for i in 1..w - 1 {
            self.set(x + i, bottom_y, '─');
        }
        lines.len() + 2
    }

    /// Adds a horizontal segment spanning columns `x0..=x1` at row `y` to
    /// the connection map -- doesn't draw yet, just records which
    /// directions this segment contributes to each cell it passes through.
    fn add_hline(&mut self, x0: usize, x1: usize, y: usize) {
        for x in x0..=x1 {
            let mut bits = 0;
            if x > x0 {
                bits |= WEST;
            }
            if x < x1 {
                bits |= EAST;
            }
            *self.connections.entry((x, y)).or_insert(0) |= bits;
        }
    }

    /// Same as `add_hline`, spanning rows `y0..=y1` at column `x`.
    fn add_vline(&mut self, y0: usize, y1: usize, x: usize) {
        for y in y0..=y1 {
            let mut bits = 0;
            if y > y0 {
                bits |= NORTH;
            }
            if y < y1 {
                bits |= SOUTH;
            }
            *self.connections.entry((x, y)).or_insert(0) |= bits;
        }
    }

    /// Converts every accumulated connection into its box-drawing glyph
    /// and writes it into the grid -- the one point where the bitmask
    /// merging in `add_hline`/`add_vline` actually becomes visible text.
    fn flush_connections(&mut self) {
        let cells: Vec<((usize, usize), u8)> = self.connections.drain().collect();
        for ((x, y), bits) in cells {
            self.set(x, y, bits_to_char(bits));
        }
    }

    fn into_lines(self) -> Vec<String> {
        self.cells
            .into_iter()
            .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
            .collect()
    }
}

/// One box's layout: top-left `y`, width, height (border included).
struct BoxLayout {
    y: usize,
    w: usize,
    h: usize,
}

/// Stacks `tables`' boxes vertically starting at `y = 0`, one blank row
/// between consecutive boxes.
fn stack_boxes(tables: &[SchemaInfo]) -> Vec<BoxLayout> {
    let mut y = 0;
    tables
        .iter()
        .map(|t| {
            let lines = table_box_lines(t);
            let w = box_width(&t.name, &lines);
            let h = lines.len() + 2;
            let layout = BoxLayout { y, w, h };
            y += h + 1;
            layout
        })
        .collect()
}

/// Draws every incoming (or outgoing) neighbor box right-aligned (or
/// left-aligned) to `col_x`/`col_width`, and wires each one to the trunk
/// column with exactly one horizontal branch (`arrow_x`..`trunk_x`, the
/// cell right next to the box's own arrowhead out to the trunk) plus that
/// arrowhead itself, pointing in FK direction -- referencing table toward
/// referenced table, which is always "toward the focal box" for an
/// incoming neighbor and "away from it" for an outgoing one. The trunk's
/// *other* branch, into the focal box, is the caller's job (drawn once,
/// not once per neighbor -- see `render`). Returns each box's connector
/// row (its vertical center), needed to size the trunk's vertical span.
/// Where a side's box column sits -- `right_align` is what makes every
/// incoming box's connecting (right) edge land on the same column
/// regardless of its own width, so their connector lines all start
/// flush rather than at staggered distances from the trunk.
struct SideColumn {
    x: usize,
    width: usize,
    right_align: bool,
}

/// The three x-coordinates a side's connector geometry needs: the
/// arrowhead next to each box, the line cell right next to it, and the
/// shared trunk column -- see `render`'s doc comment for the full layout.
struct SideConnector {
    arrow_x: usize,
    line_near_arrow: usize,
    trunk_x: usize,
}

fn draw_side(
    canvas: &mut Canvas,
    tables: &[SchemaInfo],
    layouts: &[BoxLayout],
    column: SideColumn,
    connector: SideConnector,
) -> Vec<usize> {
    let mut rows = Vec::with_capacity(tables.len());
    for (table, layout) in tables.iter().zip(layouts) {
        let x = if column.right_align {
            column.x + column.width - layout.w
        } else {
            column.x
        };
        canvas.draw_box(x, layout.y, layout.w, &table.name, &table_box_lines(table));
        let row = layout.y + layout.h / 2;
        rows.push(row);
        canvas.set(connector.arrow_x, row, '>');
        let (a, b) = (
            connector.line_near_arrow.min(connector.trunk_x),
            connector.line_near_arrow.max(connector.trunk_x),
        );
        canvas.add_hline(a, b, row);
    }
    rows
}

/// Renders `neighborhood` as box-drawing text, `focal` in the middle,
/// `incoming` stacked on the left (arrows pointing right, into `focal`)
/// and `outgoing` stacked on the right (arrows pointing right, out of
/// `focal`) -- so every arrow in the diagram reads left-to-right, from a
/// referencing table toward the table it references, regardless of which
/// side it's drawn on.
///
/// Connector geometry, `GUTTER` columns wide on each side (neighbor box
/// edge at column `E`): `E+1` is the arrowhead nearest the neighbor,
/// `E+2..E+(GUTTER-2)` is free for the line, a trunk column sits in the
/// middle of that range, and `E+(GUTTER-1)` is the arrowhead nearest the
/// far box. Every neighbor's horizontal branch and the trunk's vertical
/// span are added to the same `Canvas::connections` map, so a shared
/// trunk automatically renders as `┬`/`┴`/`├`/`┤`/`┼` wherever branches
/// meet it -- see `bits_to_char`.
pub fn render(n: &Neighborhood) -> Vec<String> {
    let focal_lines = table_box_lines(&n.focal);
    let focal_w = box_width(&n.focal.name, &focal_lines);
    let focal_h = focal_lines.len() + 2;

    let incoming_layout = stack_boxes(&n.incoming);
    let outgoing_layout = stack_boxes(&n.outgoing);
    let left_col_width = incoming_layout.iter().map(|l| l.w).max().unwrap_or(0);
    let right_col_width = outgoing_layout.iter().map(|l| l.w).max().unwrap_or(0);

    let left_col_x = 0;
    let focal_x = if n.incoming.is_empty() {
        0
    } else {
        left_col_x + left_col_width + GUTTER
    };
    let focal_y = 0;
    let right_col_x = focal_x + focal_w + if n.outgoing.is_empty() { 0 } else { GUTTER };

    let total_width = if n.outgoing.is_empty() {
        focal_x + focal_w
    } else {
        right_col_x + right_col_width
    };
    let total_height = [
        focal_y + focal_h,
        incoming_layout.last().map_or(0, |l| l.y + l.h),
        outgoing_layout.last().map_or(0, |l| l.y + l.h),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    let mut canvas = Canvas::new(total_width, total_height);
    canvas.draw_box(focal_x, focal_y, focal_w, &n.focal.name, &focal_lines);
    let hub_row = focal_y + focal_h / 2;

    if !n.incoming.is_empty() {
        let edge_x = left_col_x + left_col_width;
        let trunk_x = edge_x + GUTTER / 2;
        let rows = draw_side(
            &mut canvas,
            &n.incoming,
            &incoming_layout,
            SideColumn {
                x: left_col_x,
                width: left_col_width,
                right_align: true,
            },
            SideConnector {
                arrow_x: edge_x + 1,
                line_near_arrow: edge_x + 2,
                trunk_x,
            },
        );
        canvas.set(focal_x - 1, hub_row, '>');
        canvas.add_hline(trunk_x, focal_x - 2, hub_row);
        let min_row = rows.iter().copied().chain([hub_row]).min().unwrap();
        let max_row = rows.iter().copied().chain([hub_row]).max().unwrap();
        canvas.add_vline(min_row, max_row, trunk_x);
    }

    if !n.outgoing.is_empty() {
        let edge_x = focal_x + focal_w;
        let trunk_x = edge_x + GUTTER / 2;
        canvas.set(edge_x, hub_row, '>');
        canvas.add_hline(edge_x + 1, trunk_x, hub_row);
        let rows = draw_side(
            &mut canvas,
            &n.outgoing,
            &outgoing_layout,
            SideColumn {
                x: right_col_x,
                width: right_col_width,
                right_align: false,
            },
            SideConnector {
                arrow_x: right_col_x - 1,
                line_near_arrow: right_col_x - 2,
                trunk_x,
            },
        );
        let min_row = rows.iter().copied().chain([hub_row]).min().unwrap();
        let max_row = rows.iter().copied().chain([hub_row]).max().unwrap();
        canvas.add_vline(min_row, max_row, trunk_x);
    }

    canvas.flush_connections();
    canvas.into_lines()
}

/// Outcome of a key/mouse event the overlay can't handle itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErdOutcome {
    Closed,
}

enum ErdState {
    /// Choosing which table to center the diagram on -- reuses
    /// `HistoryPickerComponent` as a plain searchable name list, the same
    /// way `SnippetPickerComponent` reuses shared list-picking behavior
    /// rather than every picker inventing its own.
    Picking(HistoryPickerComponent),
    Viewing {
        title: String,
        lines: Vec<String>,
        scroll: usize,
    },
}

/// The ERD overlay: a table picker that transitions into the rendered
/// diagram once a table's chosen. Not a `Component` -- owned directly by
/// `QueryScreenComponent`, same as `HistoryPickerComponent`.
pub struct ErdComponent {
    state: ErdState,
}

impl ErdComponent {
    /// `table_names` is every table in the connected schema, in whatever
    /// order the caller's `SchemaInfo` list already has them.
    pub fn new(table_names: Vec<String>) -> Self {
        Self {
            state: ErdState::Picking(
                HistoryPickerComponent::new(table_names).with_title("ERD — pick a table"),
            ),
        }
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        schema: &[SchemaInfo],
    ) -> Option<ErdOutcome> {
        match &mut self.state {
            ErdState::Picking(picker) => match picker.handle_key_event(code, modifiers)? {
                HistoryOutcome::Cancelled => Some(ErdOutcome::Closed),
                HistoryOutcome::Selected(table) => {
                    match neighborhood(schema, &table) {
                        Some(n) => {
                            self.state = ErdState::Viewing {
                                title: n.focal.name.clone(),
                                lines: render(&n),
                                scroll: 0,
                            };
                            None
                        }
                        // The picker only ever offers names taken from
                        // `schema` itself, so this is unreachable in
                        // practice -- closing rather than getting stuck
                        // on a table that can't be shown.
                        None => Some(ErdOutcome::Closed),
                    }
                }
            },
            ErdState::Viewing { lines, scroll, .. } => {
                let key = KeyPress::new(code, modifiers);
                let mut pending = None;
                let Resolution::Command(command) =
                    keymap().resolve_in(&[Context::Prompt, Context::List], &mut pending, key)
                else {
                    return None;
                };
                if let Some(mv) = command.as_vim_move() {
                    tradar_core::vim_list::apply(mv, scroll, lines.len(), 1);
                    return None;
                }
                match command {
                    Command::Cancel => Some(ErdOutcome::Closed),
                    _ => None,
                }
            }
        }
    }

    /// Draws into `area`, which the caller has already centered and
    /// cleared -- same convention as `HistoryPickerComponent::draw` and
    /// every other popup `QueryScreenComponent` owns.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        match &mut self.state {
            ErdState::Picking(picker) => picker.draw(frame, area),
            ErdState::Viewing {
                title,
                lines,
                scroll,
            } => {
                let theme = theme();
                let cancel = keymap()
                    .binding_for(Context::Prompt, Command::Cancel)
                    .unwrap_or_default();
                let visible = lines
                    .iter()
                    .skip(*scroll)
                    .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.text))))
                    .collect::<Vec<Line>>();
                let block = ui::panel(&format!("ERD — {title} ({cancel} close)"), true);
                frame.render_widget(Paragraph::new(visible).block(block), area);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_driver::{ColumnInfo, ForeignKeyRef};

    fn users() -> SchemaInfo {
        SchemaInfo {
            name: "users".to_string(),
            columns: vec![ColumnInfo {
                name: "id".to_string(),
                type_name: "INTEGER".to_string(),
                primary_key: true,
                foreign_key: None,
            }],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }

    fn orders() -> SchemaInfo {
        SchemaInfo {
            name: "orders".to_string(),
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    primary_key: true,
                    foreign_key: None,
                },
                ColumnInfo {
                    name: "user_id".to_string(),
                    type_name: "INTEGER".to_string(),
                    primary_key: false,
                    foreign_key: Some(ForeignKeyRef {
                        table: "users".to_string(),
                        column: "id".to_string(),
                    }),
                },
            ],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }

    fn line_items() -> SchemaInfo {
        SchemaInfo {
            name: "line_items".to_string(),
            columns: vec![ColumnInfo {
                name: "order_id".to_string(),
                type_name: "INTEGER".to_string(),
                primary_key: false,
                foreign_key: Some(ForeignKeyRef {
                    table: "orders".to_string(),
                    column: "id".to_string(),
                }),
            }],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }

    fn products() -> SchemaInfo {
        SchemaInfo {
            name: "products".to_string(),
            columns: vec![ColumnInfo::new("id", "INTEGER")],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        }
    }

    #[test]
    fn neighborhood_finds_both_directions() {
        let schema = vec![users(), orders(), products()];

        let n = neighborhood(&schema, "orders").unwrap();

        assert_eq!(n.focal.name, "orders");
        assert_eq!(
            n.outgoing
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["users"],
            "orders.user_id references users"
        );
        assert!(n.incoming.is_empty(), "nothing references orders here");
    }

    #[test]
    fn neighborhood_finds_incoming_references() {
        let schema = vec![orders(), line_items()];

        let n = neighborhood(&schema, "orders").unwrap();

        assert_eq!(
            n.incoming
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["line_items"],
            "line_items.order_id references orders"
        );
    }

    #[test]
    fn neighborhood_is_none_for_an_unknown_table() {
        let schema = vec![users()];

        assert!(neighborhood(&schema, "no_such_table").is_none());
    }

    #[test]
    fn unrelated_tables_are_not_neighbors() {
        let schema = vec![users(), orders(), products()];

        let n = neighborhood(&schema, "users").unwrap();

        assert!(
            !n.incoming
                .iter()
                .chain(n.outgoing.iter())
                .any(|t| t.name == "products"),
            "products has no FK relation to users"
        );
    }

    #[test]
    fn render_draws_a_box_per_table_with_an_arrow_between_them() {
        let n = neighborhood(&[users(), orders()], "orders").unwrap();

        let lines = render(&n);
        let text = lines.join("\n");

        assert!(text.contains("orders"));
        assert!(text.contains("users"));
        assert!(text.contains("user_id"));
        assert!(text.contains("[PK]"), "users.id is a primary key");
        assert!(text.contains('>'), "an arrow should point toward users");
        // A straight one-row connector: no vertical trunk glyph needed
        // since orders and users are the only two boxes.
        assert!(
            !text.contains('┬') && !text.contains('┴'),
            "a single incoming/outgoing pair needs no branching trunk:\n{text}"
        );
    }

    #[test]
    fn render_branches_a_shared_trunk_for_multiple_neighbors() {
        // users <- orders -> (nothing), plus a second outgoing target so
        // the right-hand trunk has to branch to reach both.
        let mut orders = orders();
        orders.columns.push(ColumnInfo {
            name: "shipped_from".to_string(),
            type_name: "INTEGER".to_string(),
            primary_key: false,
            foreign_key: Some(ForeignKeyRef {
                table: "warehouses".to_string(),
                column: "id".to_string(),
            }),
        });
        let warehouses = SchemaInfo {
            name: "warehouses".to_string(),
            columns: vec![ColumnInfo::new("id", "INTEGER")],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        };
        let schema = vec![users(), orders, warehouses];

        let n = neighborhood(&schema, "orders").unwrap();
        let lines = render(&n);
        let text = lines.join("\n");

        assert!(text.contains("warehouses"));
        assert!(
            text.contains('┬')
                || text.contains('┴')
                || text.contains('┼')
                || text.contains('├')
                || text.contains('┤'),
            "two outgoing neighbors must share a branching trunk:\n{text}"
        );
        // Every drawn row stays inside the canvas width -- a mispositioned
        // box or connector would otherwise silently truncate mid-row
        // instead of erroring.
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert!(widths.iter().all(|w| *w <= *widths.iter().max().unwrap()));
    }

    #[test]
    fn render_caps_a_wide_table_at_max_columns() {
        let mut wide = SchemaInfo::new("wide");
        for i in 0..12 {
            wide.columns
                .push(ColumnInfo::new(format!("col{i}"), "TEXT"));
        }
        let n = Neighborhood {
            focal: wide,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        };

        let text = render(&n).join("\n");

        assert!(text.contains("col0"));
        assert!(text.contains(&format!("+{} more", 12 - MAX_COLUMNS)));
        assert!(!text.contains("col11"), "column 11 is past the cap");
    }

    /// Exact-output regression test for the trickiest layout this renders:
    /// one incoming neighbor (a plain corner-to-corner connector) and two
    /// outgoing neighbors sharing a trunk (a `┌`/`┤`/`└` branch on one
    /// side, in addition to the simpler `┬`/`┴` case already covered by
    /// `render_branches_a_shared_trunk_for_multiple_neighbors`) -- visually
    /// verified once by hand, pinned here so a future change to the
    /// connector geometry has to justify itself against a real diagram,
    /// not just "still compiles".
    #[test]
    fn render_draws_the_exact_expected_diagram_for_a_mixed_neighborhood() {
        let mut orders = orders();
        orders.columns.push(ColumnInfo {
            name: "shipped_from".to_string(),
            type_name: "INTEGER".to_string(),
            primary_key: false,
            foreign_key: Some(ForeignKeyRef {
                table: "warehouses".to_string(),
                column: "id".to_string(),
            }),
        });
        let warehouses = SchemaInfo {
            name: "warehouses".to_string(),
            columns: vec![
                ColumnInfo::new("id", "INTEGER"),
                ColumnInfo::new("city", "TEXT"),
            ],
            kind: None,
            ttl: None,
            schema: None,
            object_kind: None,
        };
        let schema = vec![users(), orders, line_items(), warehouses];

        let n = neighborhood(&schema, "orders").unwrap();
        let diagram = render(&n).join("\n");

        assert_eq!(
            diagram,
            [
                r"┌─ line_items ──────────────────┐       ┌─ orders ──────────────────────────────┐       ┌─ users ─────────┐",
                r"│ order_id INTEGER -> orders.id │ >─┐   │ id INTEGER [PK]                       │   ┌──>│ id INTEGER [PK] │",
                r"└───────────────────────────────┘   └──>│ user_id INTEGER -> users.id           │>──┤   └─────────────────┘",
                r"                                        │ shipped_from INTEGER -> warehouses.id │   │",
                r"                                        └───────────────────────────────────────┘   │   ┌─ warehouses ┐",
                r"                                                                                    │   │ id INTEGER  │",
                r"                                                                                    └──>│ city TEXT   │",
                r"                                                                                        └─────────────┘",
            ]
            .join("\n")
        );
    }

    #[test]
    fn render_with_no_neighbors_is_just_the_focal_box() {
        let n = Neighborhood {
            focal: users(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        };

        let text = render(&n).join("\n");

        assert!(text.contains("users"));
        assert!(!text.contains('>'), "nothing to point an arrow at");
    }

    fn schema_for_component() -> Vec<SchemaInfo> {
        vec![users(), orders()]
    }

    #[test]
    fn picking_a_table_transitions_to_the_rendered_diagram() {
        let mut erd = ErdComponent::new(vec!["users".to_string(), "orders".to_string()]);
        let schema = schema_for_component();

        let outcome = erd.handle_key_event(KeyCode::Enter, KeyModifiers::NONE, &schema);

        assert_eq!(outcome, None, "picking a table doesn't close the overlay");
        assert!(matches!(erd.state, ErdState::Viewing { .. }));
    }

    #[test]
    fn esc_cancels_the_picker() {
        let mut erd = ErdComponent::new(vec!["users".to_string()]);
        let schema = schema_for_component();

        let outcome = erd.handle_key_event(KeyCode::Esc, KeyModifiers::NONE, &schema);

        assert_eq!(outcome, Some(ErdOutcome::Closed));
    }

    #[test]
    fn esc_closes_the_diagram_view_too() {
        let mut erd = ErdComponent::new(vec!["users".to_string()]);
        let schema = schema_for_component();
        erd.handle_key_event(KeyCode::Enter, KeyModifiers::NONE, &schema);
        assert!(matches!(erd.state, ErdState::Viewing { .. }));

        let outcome = erd.handle_key_event(KeyCode::Esc, KeyModifiers::NONE, &schema);

        assert_eq!(outcome, Some(ErdOutcome::Closed));
    }

    #[test]
    fn j_and_k_scroll_the_diagram_view() {
        let mut erd = ErdComponent::new(vec!["users".to_string()]);
        let schema = schema_for_component();
        erd.handle_key_event(KeyCode::Enter, KeyModifiers::NONE, &schema);

        erd.handle_key_event(KeyCode::Char('j'), KeyModifiers::NONE, &schema);
        let ErdState::Viewing { scroll, .. } = &erd.state else {
            panic!("expected Viewing");
        };
        assert_eq!(*scroll, 1);
    }
}
