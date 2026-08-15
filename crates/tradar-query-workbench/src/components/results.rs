//! The results/error pane on the query screen. Owns no keys of its own —
//! driven entirely by `QueryScreenComponent` calling `set_result`/`set_error`
//! and its movement/yank methods directly from key handling.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};

use tradar_core::theme::theme;
use tradar_core::ui;
use tradar_core::vim_list::{self, VimMove};

use crate::query_driver::QueryResult;

/// Width for each column: whatever its widest value needs, capped at
/// `MAX_COLUMN_WIDTH`, and never narrower than its own header.
fn column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|name| name.chars().count().min(MAX_COLUMN_WIDTH))
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(cell.chars().count()).min(MAX_COLUMN_WIDTH);
            }
        }
    }
    widths
}

/// Cuts `text` to `width` columns, marking the cut with `…` so a truncated
/// value can't be mistaken for a complete one.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    text.chars()
        .take(keep)
        .chain(std::iter::once('…'))
        .collect()
}

/// `1 row` / `2 rows` -- the results title reads as a sentence, so the
/// plural has to agree.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Widest a single column may get before its cells are truncated. Without
/// a cap one long text column pushes every other column off screen, which
/// is worse than losing the tail of that one value.
const MAX_COLUMN_WIDTH: usize = 40;

/// Blank columns between two rendered columns.
const COLUMN_SPACING: u16 = 2;

/// How a `Documents` result (Mongo, Elasticsearch) is shown -- see
/// `ResultsComponent::toggle_document_view`. Irrelevant for `Table`/
/// `Affected`, which have nothing to toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentView {
    Json,
    Table,
}

/// Flattens `docs` into a table: columns are every top-level key seen
/// across the whole result, alphabetically -- so heterogeneous documents
/// (a field only some of them have) still line up rather than erroring or
/// dropping data, and the column order is deterministic regardless of a
/// document's own field order (JSON objects don't guarantee one) or which
/// document in the result happens to be first. Sorted explicitly with a
/// `BTreeSet` rather than trusting `serde_json::Map`'s own iteration order,
/// which depends on whether the `preserve_order` feature happens to be
/// enabled elsewhere in the dependency tree -- not something this code
/// should be silently at the mercy of. A document missing a column leaves
/// that cell blank rather than, say, "null" -- blank reads as "not
/// present", which is what's actually true here (`null` would claim the
/// key exists with a null value, a different thing). Nested objects/arrays
/// render as compact one-line JSON -- full detail is still one `Space`
/// away via the existing cell-preview panel, which already pretty-prints
/// anything that parses as JSON.
fn documents_as_table(docs: &[serde_json::Value]) -> (Vec<String>, Vec<Vec<String>>) {
    let mut column_set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for doc in docs {
        if let serde_json::Value::Object(map) = doc {
            column_set.extend(map.keys().map(String::as_str));
        }
    }
    let columns: Vec<String> = column_set.into_iter().map(str::to_string).collect();
    let rows = docs
        .iter()
        .map(|doc| {
            columns
                .iter()
                .map(|column| match doc.get(column) {
                    None | Some(serde_json::Value::Null) => String::new(),
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                })
                .collect()
        })
        .collect();
    (columns, rows)
}

/// Rows given to the cell-preview panel when it's open, capped so it can
/// never crowd the table out of the results pane entirely.
const PREVIEW_HEIGHT: u16 = 8;

/// The selected cell's value, pretty-printed if it parses as a JSON object
/// or array -- a jsonb column, a Postgres/SQLite JSON-text value -- and
/// left as-is otherwise (a plain string gains nothing from reformatting,
/// and this is also the fallback for a value that isn't JSON at all).
fn preview_text(value: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(value) {
        Ok(parsed @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
            serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| value.to_string())
        }
        _ => value.to_string(),
    }
}

/// Indices of the rows in `rows` that survive a case-insensitive substring
/// search for `filter` in any cell -- shared by
/// `ResultsComponent::visible_items` (drives row selection/movement) and
/// `draw_table_body` (draws the result), so the two can never disagree
/// about which rows are visible.
fn filter_table_rows(rows: &[Vec<String>], filter: &str) -> Vec<usize> {
    let needle = filter.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            needle.is_empty() || row.iter().any(|cell| cell.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect()
}

/// The columns from `offset` on that fit in `total` once the row-number
/// gutter has taken its share. Always at least one, even when it doesn't
/// fit: a column too wide for the terminal still has to be reachable.
fn fitting_columns(widths: &[usize], offset: usize, total: u16, gutter: u16) -> Vec<usize> {
    let mut used = gutter as usize;
    let mut visible = Vec::new();
    for (index, width) in widths.iter().enumerate().skip(offset) {
        let next = used + COLUMN_SPACING as usize + width;
        if !visible.is_empty() && next > total as usize {
            break;
        }
        used = next;
        visible.push(index);
    }
    visible
}

pub struct ResultsComponent {
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub selected: usize,
    /// The selected column, so the grid has a *cell* cursor rather than
    /// only a row cursor -- what "edit this value" needs to point at.
    pub selected_col: usize,
    /// Rows not matching this are hidden. Filtering rather than jumping
    /// between matches: on a result you can't see all of, "show me only
    /// these" is the question people actually have, and it composes with
    /// everything else the grid does (yank, edit, delete all act on what's
    /// under the cursor either way).
    filter: String,
    /// First visible column, for tables too wide to fit. Derived rather
    /// than driven: `draw` scrolls it just far enough to keep
    /// `selected_col` on screen, the way a spreadsheet follows its cursor.
    col_offset: usize,
    /// Whether a query is in flight, so the pane can say so: without it a
    /// slow query is indistinguishable from a key that didn't register.
    running: bool,
    /// Kept between frames so the scroll offset survives, which is what
    /// makes a click land on the row that was actually pointed at.
    table_state: TableState,
    list_state: ListState,
    /// Where the rows were last drawn, for hit-testing clicks. Excludes
    /// the border and (for tables) the header row.
    rows_area: Rect,
    /// Each visible column as `(index, x, width)`, recorded at draw time so
    /// a click can land on a *cell* and not just a row.
    column_spans: Vec<(usize, u16, u16)>,
    visible_height: usize,
    /// Whether the selected cell's full value is shown in a panel below the
    /// grid -- see `toggle_preview`. Closed on every selection change: a
    /// preview left open while the cursor moves on would show the wrong
    /// cell's value without saying so.
    preview_open: bool,
    /// How a `Documents` result is currently shown -- see
    /// `toggle_document_view`. Deliberately *not* reset by `set_result`:
    /// unlike the cell cursor, this is a standing preference for the
    /// connection, not something that only makes sense for one query's
    /// worth of rows.
    doc_view: DocumentView,
    /// `documents_as_table` of the current `Documents` result, recomputed
    /// whenever `set_result`/`set_result_keeping_cursor` runs. Cached
    /// rather than rebuilt every frame -- a document result can be large,
    /// and `draw()` runs on every redraw, not just when the result changes.
    doc_table: Option<(Vec<String>, Vec<Vec<String>>)>,
}

impl Default for ResultsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultsComponent {
    pub fn new() -> Self {
        Self {
            last_result: None,
            last_error: None,
            selected: 0,
            selected_col: 0,
            filter: String::new(),
            col_offset: 0,
            running: false,
            table_state: TableState::default(),
            list_state: ListState::default(),
            rows_area: Rect::ZERO,
            column_spans: Vec::new(),
            visible_height: 0,
            preview_open: false,
            doc_view: DocumentView::Json,
            doc_table: None,
        }
    }

    /// Told by the screen each frame, since the engine owns that state.
    pub fn draw_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.doc_table = match &result {
            QueryResult::Documents(docs) => Some(documents_as_table(docs)),
            _ => None,
        };
        self.last_result = Some(result);
        self.last_error = None;
        self.selected = 0;
        self.selected_col = 0;
        self.col_offset = 0;
        self.preview_open = false;
        // A filter from the last result would silently hide rows of this
        // one, and you'd be reading a subset without knowing it.
        self.filter.clear();
    }

    /// Same as `set_result`, but leaves the cell cursor and the filter
    /// where they were -- used when a result is *re-read* rather than
    /// replaced (the refresh after an edit), so the row you just changed is
    /// still under the cursor instead of the view jumping back to the top.
    pub fn set_result_keeping_cursor(&mut self, result: QueryResult) {
        let (row, column, filter) = (
            self.selected,
            self.selected_col,
            std::mem::take(&mut self.filter),
        );
        self.set_result(result);
        self.filter = filter;
        self.selected = row.min(self.item_count().saturating_sub(1));
        self.selected_col = column.min(self.column_count().saturating_sub(1));
    }

    /// Narrows the grid to rows containing `filter`, case-insensitively,
    /// anywhere in them. An empty string shows everything again.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        // The row that was under the cursor may not be in the new set, and
        // a cursor pointing past the end selects nothing at all.
        self.selected = self.selected.min(self.item_count().saturating_sub(1));
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Indices into the underlying rows/documents that survive the filter,
    /// in order. This is what `selected` indexes -- everything that acts on
    /// "the selected row" goes through here, so an edit made while filtered
    /// still lands on the row the user is pointing at.
    fn visible_items(&self) -> Vec<usize> {
        match &self.last_result {
            Some(QueryResult::Table { rows, .. }) => filter_table_rows(rows, &self.filter),
            Some(QueryResult::Documents(_)) if self.doc_view == DocumentView::Table => self
                .doc_table
                .as_ref()
                .map(|(_, rows)| filter_table_rows(rows, &self.filter))
                .unwrap_or_default(),
            Some(QueryResult::Documents(docs)) => {
                let needle = self.filter.to_lowercase();
                docs.iter()
                    .enumerate()
                    .filter(|(_, doc)| {
                        needle.is_empty()
                            || serde_json::to_string(doc)
                                .unwrap_or_default()
                                .to_lowercase()
                                .contains(&needle)
                    })
                    .map(|(index, _)| index)
                    .collect()
            }
            Some(QueryResult::Affected { .. }) | None => Vec::new(),
        }
    }

    /// Where the selected row sits in the *unfiltered* result -- what the
    /// row-number gutter shows, so the numbers stay honest while filtered.
    fn selected_item(&self) -> Option<usize> {
        self.visible_items().get(self.selected).copied()
    }

    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.last_result = None;
        self.selected = 0;
        self.preview_open = false;
    }

    /// How many rows are selectable right now -- after filtering, since
    /// that's what's on screen to move through.
    fn item_count(&self) -> usize {
        self.visible_items().len()
    }

    /// How many there are in total, filter or no filter.
    fn total_count(&self) -> usize {
        match &self.last_result {
            Some(QueryResult::Table { rows, .. }) => rows.len(),
            Some(QueryResult::Documents(docs)) => docs.len(),
            // Nothing to select: it's a one-line report, not a list.
            Some(QueryResult::Affected { .. }) | None => 0,
        }
    }

    /// For `Documents` `visible_height`-based half-page scrolling is an
    /// approximation (items can span multiple rows), same tradeoff as the
    /// row-count-based scrolling in `SchemaSidebarComponent`.
    /// `pub` because `QueryScreenComponent` resolves the key (it owns
    /// focus) and hands the movement down.
    pub fn apply_move(&mut self, mv: VimMove) {
        let count = self.item_count();
        vim_list::apply(mv, &mut self.selected, count, self.visible_height);
        self.preview_open = false;
    }

    /// Shows/hides the selected cell's full value in a panel below the
    /// grid -- a no-op when nothing is selected (no result, an error, or an
    /// `Affected` report, none of which have a cell to preview).
    pub fn toggle_preview(&mut self) {
        if self.selected_cell().is_some() {
            self.preview_open = !self.preview_open;
        }
    }

    /// Switches a `Documents` result between its pretty-printed JSON list
    /// and a flattened table (see `documents_as_table`) -- a no-op for a
    /// `Table` or `Affected` result, which have nothing to toggle. The
    /// cell cursor resets rather than carrying over: JSON view has no
    /// column for it to mean the same thing in.
    pub fn toggle_document_view(&mut self) {
        if !matches!(self.last_result, Some(QueryResult::Documents(_))) {
            return;
        }
        self.doc_view = match self.doc_view {
            DocumentView::Json => DocumentView::Table,
            DocumentView::Table => DocumentView::Json,
        };
        self.selected_col = 0;
        self.col_offset = 0;
        self.preview_open = false;
    }

    pub fn move_down(&mut self) {
        self.apply_move(VimMove::Down);
    }

    pub fn move_up(&mut self) {
        self.apply_move(VimMove::Up);
    }

    pub fn move_to_top(&mut self) {
        self.apply_move(VimMove::Top);
    }

    pub fn move_to_bottom(&mut self) {
        self.apply_move(VimMove::Bottom);
    }

    pub fn move_half_page_down(&mut self) {
        self.apply_move(VimMove::HalfPageDown);
    }

    pub fn move_half_page_up(&mut self) {
        self.apply_move(VimMove::HalfPageUp);
    }

    /// Column count of the current table result -- also the flattened
    /// column count of a `Documents` result being shown as a table, `0`
    /// otherwise (documents in JSON view, or no result), since neither
    /// scrolls horizontally.
    fn column_count(&self) -> usize {
        self.columns().len()
    }

    /// Moves the cell cursor one column left. Horizontal scrolling follows
    /// the cursor at draw time, so there's no separate "scroll" to do.
    pub fn prev_column(&mut self) {
        self.selected_col = self.selected_col.saturating_sub(1);
        self.preview_open = false;
    }

    pub fn next_column(&mut self) {
        let last = self.column_count().saturating_sub(1);
        self.selected_col = (self.selected_col + 1).min(last);
        self.preview_open = false;
    }

    /// The column names of the current table result -- also the flattened
    /// columns of a `Documents` result shown as a table -- for whoever
    /// needs to name the selected cell's column. Empty for anything else
    /// (documents in JSON view have no columns to name).
    pub fn columns(&self) -> &[String] {
        match &self.last_result {
            Some(QueryResult::Table { columns, .. }) => columns,
            Some(QueryResult::Documents(_)) if self.doc_view == DocumentView::Table => self
                .doc_table
                .as_ref()
                .map_or(&[], |(columns, _)| columns.as_slice()),
            _ => &[],
        }
    }

    /// The selected row's values, in column order -- the raw strings as
    /// fetched, not the truncated forms on screen. For a `Documents` result
    /// in table view, the selected document's flattened row.
    pub fn selected_row(&self) -> Option<&Vec<String>> {
        let index = self.selected_item()?;
        match &self.last_result {
            Some(QueryResult::Table { rows, .. }) => rows.get(index),
            Some(QueryResult::Documents(_)) if self.doc_view == DocumentView::Table => {
                self.doc_table.as_ref()?.1.get(index)
            }
            _ => None,
        }
    }

    /// The selected cell as `(column name, value)`.
    pub fn selected_cell(&self) -> Option<(&str, &str)> {
        let column = self.columns().get(self.selected_col)?;
        let value = self.selected_row()?.get(self.selected_col)?;
        Some((column.as_str(), value.as_str()))
    }

    /// Selects the clicked cell. Returns whether the click was inside this
    /// pane's rows at all.
    pub fn click(&mut self, column: u16, row: u16) -> bool {
        if !ui::contains(self.rows_area, column, row) {
            return false;
        }
        let offset = match &self.last_result {
            Some(QueryResult::Documents(_)) if self.doc_view == DocumentView::Json => {
                self.list_state.offset()
            }
            _ => self.table_state.offset(),
        };
        if let Some(index) = ui::index_at(self.rows_area, offset, row, self.item_count()) {
            self.selected = index;
        }
        // Clicking to the left of the first column (the row-number gutter)
        // or past the last one keeps whichever column was already current.
        if let Some((index, ..)) = self
            .column_spans
            .iter()
            .find(|(_, x, width)| column >= *x && column < x.saturating_add(*width))
        {
            self.selected_col = *index;
        }
        self.preview_open = false;
        true
    }

    /// Plain-text form of the currently selected row/document, ready to
    /// yank to the clipboard. `None` when there's nothing to select (no
    /// result yet, or the last response was an error). Table rows are
    /// tab-separated, matching what spreadsheets expect when pasted.
    pub fn selected_text(&self) -> Option<String> {
        let index = self.selected_item()?;
        match self.last_result.as_ref()? {
            QueryResult::Table { rows, .. } => rows.get(index).map(|row| row.join("\t")),
            // Matches whatever's actually on screen: tab-separated like any
            // other table in table view, pretty JSON in JSON view.
            QueryResult::Documents(_) if self.doc_view == DocumentView::Table => self
                .doc_table
                .as_ref()?
                .1
                .get(index)
                .map(|row| row.join("\t")),
            QueryResult::Documents(docs) => docs
                .get(index)
                .map(|doc| serde_json::to_string_pretty(doc).unwrap_or_default()),
            QueryResult::Affected { .. } => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = theme();
        // The row count belongs in the title: it's the first thing anyone
        // wants to know about a result, and it costs no extra space.
        if self.running {
            let block = ui::panel("Results — running…", focused);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            self.visible_height = 0;
            let cancel = tradar_core::keymap::keymap()
                .binding_for(
                    tradar_core::keymap::Context::QueryScreen,
                    tradar_core::keymap::Command::CancelQuery,
                )
                .unwrap_or_default();
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("running… {cancel} to cancel"),
                    Style::default().fg(theme.text_dim),
                )),
                inner,
            );
            return;
        }

        let title = match (&self.last_error, &self.last_result) {
            (Some(_), _) => "Results — error".to_string(),
            (None, Some(QueryResult::Table { truncated, .. })) => {
                let total = self.total_count();
                if !self.filter.is_empty() {
                    // Both numbers, so a filtered view can never be read as
                    // the whole result.
                    format!(
                        "Results ({} of {} — filter: {})",
                        self.item_count(),
                        count(total, "row"),
                        self.filter
                    )
                } else if *truncated {
                    // Say so loudly: a silently clipped result set is a
                    // wrong answer you can't see is wrong.
                    format!("Results (first {total} rows — truncated)")
                } else {
                    format!("Results ({})", count(total, "row"))
                }
            }
            (None, Some(QueryResult::Documents(_))) => {
                let total = self.total_count();
                // Still says "documents", even in table view: the data is a
                // set of documents no matter how it's rendered, and saying
                // "rows" would claim a tabular shape that isn't really there
                // (see `documents_as_table`'s doc comment on missing keys).
                let view = if self.doc_view == DocumentView::Table {
                    " — table view"
                } else {
                    ""
                };
                if self.filter.is_empty() {
                    format!("Results ({}){view}", count(total, "document"))
                } else {
                    format!(
                        "Results ({} of {} — filter: {}){view}",
                        self.item_count(),
                        count(total, "document"),
                        self.filter
                    )
                }
            }
            (None, Some(QueryResult::Affected { .. })) => "Results".to_string(),
            (None, None) => "Results".to_string(),
        };
        let block = ui::panel(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Only a table has columns to click on; anything else must not
        // leave last frame's hit-boxes lying around.
        self.column_spans.clear();

        if let Some(error) = &self.last_error {
            self.visible_height = 0;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    error.as_str(),
                    Style::default().fg(theme.error),
                ))
                .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }

        let Some(result) = &self.last_result else {
            self.visible_height = 0;
            return;
        };

        match result {
            QueryResult::Table { columns, rows, .. } => {
                draw_table_body(
                    frame,
                    theme,
                    inner,
                    columns,
                    rows,
                    &self.filter,
                    self.selected,
                    self.preview_open,
                    &mut self.selected_col,
                    &mut self.col_offset,
                    &mut self.visible_height,
                    &mut self.column_spans,
                    &mut self.rows_area,
                    &mut self.table_state,
                );
            }
            QueryResult::Documents(_) if self.doc_view == DocumentView::Table => {
                let Some((columns, rows)) = &self.doc_table else {
                    return;
                };
                draw_table_body(
                    frame,
                    theme,
                    inner,
                    columns,
                    rows,
                    &self.filter,
                    self.selected,
                    self.preview_open,
                    &mut self.selected_col,
                    &mut self.col_offset,
                    &mut self.visible_height,
                    &mut self.column_spans,
                    &mut self.rows_area,
                    &mut self.table_state,
                );
            }
            QueryResult::Affected { rows } => {
                self.visible_height = 0;
                // A write reports what it did, in the pane where results
                // would otherwise appear -- the whole point is that this
                // can't be mistaken for an empty SELECT.
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!("OK — {} affected", count(*rows as usize, "row")),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )),
                    inner,
                );
            }
            QueryResult::Documents(docs) => {
                self.visible_height = inner.height as usize;

                let visible = self.visible_items();
                let items: Vec<ListItem> = visible
                    .iter()
                    .map(|&index| {
                        let doc = &docs[index];
                        let pretty = serde_json::to_string_pretty(doc).unwrap_or_default();
                        ListItem::new(Text::from(
                            pretty
                                .lines()
                                .map(|line| Line::from(line.to_string()))
                                .collect::<Vec<_>>(),
                        ))
                    })
                    .collect();
                if visible.is_empty() {
                    self.list_state.select(None);
                } else {
                    self.list_state.select(Some(self.selected));
                }
                self.rows_area = inner;
                let list = List::new(items).highlight_style(ui::selection_style());
                frame.render_stateful_widget(list, inner, &mut self.list_state);
            }
        }
    }
}

/// The table widget shared by a real `Table` result and a `Documents`
/// result being shown flattened -- see `documents_as_table`. A free
/// function, not a method: `columns`/`rows` borrow from whichever of
/// `last_result`/`doc_table` the caller matched on, and a `&mut self`
/// method call can't be handed a borrow of one of its own fields as an
/// argument. Splitting the handful of fields this actually mutates into
/// their own `&mut` parameters instead sidesteps that -- the caller passes
/// disjoint field borrows directly, which Rust allows.
#[allow(clippy::too_many_arguments)]
fn draw_table_body(
    frame: &mut Frame,
    theme: &tradar_core::theme::Theme,
    inner: Rect,
    columns: &[String],
    rows: &[Vec<String>],
    filter: &str,
    selected: usize,
    preview_open: bool,
    selected_col: &mut usize,
    col_offset: &mut usize,
    visible_height: &mut usize,
    column_spans: &mut Vec<(usize, u16, u16)>,
    rows_area: &mut Rect,
    table_state: &mut TableState,
) {
    // A real table widget, not rows joined by spaces: columns have to
    // line up or the values can't be read down a column, which is most
    // of the point of tabular output.
    *selected_col = (*selected_col).min(columns.len().saturating_sub(1));
    let widths = column_widths(columns, rows);

    // A row-number gutter, wide enough for the highest number there is:
    // on a screen full of rows, "which one am I on" is otherwise
    // something you have to count. Computed once and reused below for
    // both the preview (which cell is selected) and the table body.
    let visible_rows = filter_table_rows(rows, filter);
    let gutter = (rows.len().to_string().chars().count() as u16).max(1);

    // Computed with the cell cursor as it stood before this frame's
    // clamp above -- selecting a cell and immediately toggling its
    // preview must show *that* cell, not whatever clamping happened to
    // land on.
    let preview = if preview_open {
        visible_rows.get(selected).and_then(|&index| {
            let value = rows.get(index)?.get(*selected_col)?;
            let column = columns.get(*selected_col)?;
            Some((column.clone(), preview_text(value)))
        })
    } else {
        None
    };
    let preview_height = if preview.is_some() {
        PREVIEW_HEIGHT.min(inner.height.saturating_sub(4))
    } else {
        0
    };
    let table_area = Rect {
        height: inner.height - preview_height,
        ..inner
    };
    let preview_area = (preview_height > 0).then(|| Rect {
        y: inner.y + table_area.height,
        height: preview_height,
        ..inner
    });

    // The header row is drawn by the widget, so it costs one row of the
    // body -- account for it or half-page scrolling overshoots by one.
    *visible_height = (table_area.height as usize).saturating_sub(1);

    // Scroll only as far as it takes to keep the cell cursor on screen,
    // then take as many further columns as fit. Rendering columns that
    // don't fit would make the layout shrink every column to make room,
    // and then nothing lines up.
    if *selected_col < *col_offset {
        *col_offset = *selected_col;
    }
    let visible = loop {
        let visible = fitting_columns(&widths, *col_offset, inner.width, gutter);
        if *col_offset >= *selected_col || visible.contains(&*selected_col) {
            break visible;
        }
        *col_offset += 1;
    };

    let header = Row::new(
        std::iter::once(Cell::from("#"))
            .chain(
                visible
                    .iter()
                    .map(|&index| Cell::from(truncate(&columns[index], widths[index]))),
            )
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    let body: Vec<Row> = visible_rows
        .iter()
        .map(|&number| {
            let row = &rows[number];
            Row::new(
                // Numbered by position in the whole result, not
                // in the filtered view: while filtered, "row 4"
                // still means the fourth row of the query.
                std::iter::once(
                    Cell::from((number + 1).to_string()).style(Style::default().fg(theme.text_dim)),
                )
                .chain(visible.iter().map(|&index| {
                    Cell::from(truncate(
                        row.get(index).map_or("", String::as_str),
                        widths[index],
                    ))
                }))
                .collect::<Vec<_>>(),
            )
            .style(Style::default().fg(theme.text))
        })
        .collect();

    let constraints: Vec<Constraint> = std::iter::once(Constraint::Length(gutter))
        .chain(
            visible
                .iter()
                .map(|&index| Constraint::Length(widths[index] as u16)),
        )
        .collect();

    // Where each visible column landed, so a click can pick a
    // cell. The gutter is column 0 and belongs to nobody.
    let mut x = inner
        .x
        .saturating_add(gutter)
        .saturating_add(COLUMN_SPACING);
    for &index in &visible {
        let width = widths[index] as u16;
        column_spans.push((index, x, width));
        x = x.saturating_add(width).saturating_add(COLUMN_SPACING);
    }

    if visible_rows.is_empty() {
        table_state.select_cell(None);
    } else {
        let column = visible
            .iter()
            .position(|&index| index == *selected_col)
            .map_or(0, |offset| offset + 1);
        table_state.select_cell(Some((selected, column)));
    }
    // The widget draws its header on the first row, so the
    // clickable rows start one below.
    *rows_area = Rect {
        y: table_area.y.saturating_add(1),
        height: table_area.height.saturating_sub(1),
        ..table_area
    };
    let table = Table::new(body, constraints)
        .header(header)
        .column_spacing(COLUMN_SPACING)
        .row_highlight_style(ui::selection_style())
        .cell_highlight_style(ui::cell_cursor_style());
    frame.render_stateful_widget(table, table_area, table_state);

    if let (Some(area), Some((column, text))) = (preview_area, &preview) {
        let block = ui::panel(&format!("{column} — full value"), false);
        let content = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(text.as_str()).wrap(Wrap { trim: false }),
            content,
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::query_driver::QueryResult;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    /// One rendered row as a string, for asserting on column positions.
    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer.cell((x, y)).unwrap().symbol())
            .collect()
    }

    fn draw_component(component: &mut ResultsComponent, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, Rect::new(0, 0, width, height), false))
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn table(rows: usize) -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: (0..rows).map(|i| vec![i.to_string()]).collect(),
            truncated: false,
        }
    }

    /// Two columns whose values are much wider than their headers -- the
    /// case that used to render ragged.
    fn wide_table() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string(), "n".to_string()],
            rows: vec![
                vec!["1".to_string(), "alice".to_string(), "10".to_string()],
                vec!["1000".to_string(), "bo".to_string(), "2000".to_string()],
            ],
            truncated: false,
        }
    }

    #[test]
    fn columns_line_up_under_their_headers() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 40, 8), false))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Column starts, read off the header row, must be where the data
        // rows start too -- that's what "aligned" means.
        let header = row_text(&buffer, 1);
        let first = row_text(&buffer, 2);
        let second = row_text(&buffer, 3);
        let name_column = header.find("name").expect("header row was: {header}");
        assert_eq!(
            first.find("alice"),
            Some(name_column),
            "row 1 was: {first:?}, header: {header:?}"
        );
        assert_eq!(
            second.find("bo"),
            Some(name_column),
            "row 2 was: {second:?}, header: {header:?}"
        );
    }

    #[test]
    fn preview_text_pretty_prints_a_json_object_but_not_a_plain_scalar() {
        assert_eq!(
            preview_text(r#"{"a":1}"#),
            serde_json::to_string_pretty(&serde_json::json!({"a": 1})).unwrap()
        );
        assert_eq!(
            preview_text(r#"[1,2]"#),
            serde_json::to_string_pretty(&serde_json::json!([1, 2])).unwrap()
        );
        assert_eq!(preview_text("42"), "42", "a bare number is not reformatted");
        assert_eq!(
            preview_text("not json at all"),
            "not json at all",
            "non-JSON text passes through unchanged"
        );
    }

    #[test]
    fn a_value_wider_than_the_column_cap_is_truncated_with_an_ellipsis() {
        assert_eq!(truncate("alice", 5), "alice");
        assert_eq!(truncate("alexandra", 5), "alex…");
        assert_eq!(
            column_widths(
                &["v".to_string()],
                &[vec!["x".repeat(MAX_COLUMN_WIDTH + 10)]]
            ),
            vec![MAX_COLUMN_WIDTH],
            "one huge value must not push the other columns off screen"
        );
    }

    #[test]
    fn column_width_is_the_widest_of_the_header_and_its_values() {
        let widths = column_widths(
            &["id".to_string(), "name".to_string()],
            &[
                vec!["1".to_string(), "alice".to_string()],
                vec!["1000".to_string(), "bo".to_string()],
            ],
        );

        assert_eq!(widths, vec![4, 5], "id -> '1000', name -> 'alice'");
    }

    #[test]
    fn moving_sideways_walks_the_cell_cursor_and_stops_at_the_ends() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());

        results.next_column();
        assert_eq!(results.selected_col, 1);

        results.next_column();
        results.next_column();
        assert_eq!(results.selected_col, 2, "must stop at the last column");

        results.prev_column();
        assert_eq!(results.selected_col, 1);
        results.prev_column();
        results.prev_column();
        assert_eq!(results.selected_col, 0, "must not move past the first");
    }

    #[test]
    fn the_selected_cell_reports_its_column_and_the_untruncated_value() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        results.move_down();
        results.next_column();

        assert_eq!(results.selected_cell(), Some(("name", "bo")));
    }

    #[test]
    fn the_view_scrolls_only_as_far_as_it_takes_to_show_the_cell_cursor() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());

        // Narrow enough that `id`, `name` and `n` can't all be shown at
        // once, so reaching the last column has to scroll the first off.
        results.next_column();
        results.next_column();
        let text = draw_component(&mut results, 18, 8);

        assert!(text.contains("2000"), "the cursor's column: {text}");
        assert!(
            !text.contains("1000"),
            "the leading column should have scrolled off: {text}"
        );
    }

    #[test]
    fn every_row_is_numbered_so_you_never_have_to_count() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));

        let buffer_text = draw_component(&mut results, 20, 8);

        assert!(buffer_text.contains('#'), "buffer was: {buffer_text}");
        // Rows are numbered from one; the `id` values here are 0..2, so a
        // "3" can only be the third row's number.
        assert!(buffer_text.contains('3'), "buffer was: {buffer_text}");
    }

    #[test]
    fn a_new_result_resets_the_cell_cursor() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        results.next_column();

        results.set_result(wide_table());

        assert_eq!(results.selected_col, 0);
        assert_eq!(results.col_offset, 0);
    }

    #[test]
    fn a_refresh_keeps_the_cell_cursor_where_it_was() {
        let mut results = ResultsComponent::new();
        results.set_result(wide_table());
        results.move_down();
        results.next_column();

        results.set_result_keeping_cursor(wide_table());

        assert_eq!((results.selected, results.selected_col), (1, 1));
    }

    #[test]
    fn a_refresh_that_returns_fewer_rows_clamps_the_cursor_into_range() {
        let mut results = ResultsComponent::new();
        results.set_result(table(5));
        results.move_to_bottom();

        results.set_result_keeping_cursor(table(2));

        assert_eq!(results.selected, 1);
    }

    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());

        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
            truncated: false,
        });

        assert!(results.last_error.is_none());
        assert_eq!(
            results.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
                truncated: false,
            })
        );
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
            truncated: false,
        });

        results.set_error("boom".to_string());

        assert!(results.last_result.is_none());
        assert_eq!(results.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn set_result_resets_the_selection() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));
        results.move_down();
        assert_eq!(results.selected, 1);

        results.set_result(table(5));

        assert_eq!(results.selected, 0);
    }

    #[test]
    fn move_down_advances_and_stops_at_the_last_row() {
        let mut results = ResultsComponent::new();
        results.set_result(table(2));

        results.move_down();
        assert_eq!(results.selected, 1);

        results.move_down();
        assert_eq!(results.selected, 1, "should stop at the last row, not wrap");
    }

    #[test]
    fn move_up_retreats_and_stops_at_zero() {
        let mut results = ResultsComponent::new();
        results.set_result(table(2));
        results.move_down();

        results.move_up();
        assert_eq!(results.selected, 0);

        results.move_up();
        assert_eq!(results.selected, 0, "should stop at zero, not go negative");
    }

    #[test]
    fn move_to_top_and_bottom_jump_straight_there() {
        let mut results = ResultsComponent::new();
        results.set_result(table(5));
        results.move_down();

        results.move_to_bottom();
        assert_eq!(results.selected, 4);

        results.move_to_top();
        assert_eq!(results.selected, 0);
    }

    #[test]
    fn movement_is_a_no_op_when_there_is_no_result() {
        let mut results = ResultsComponent::new();

        results.move_down();
        results.move_to_bottom();

        assert_eq!(results.selected, 0);
    }

    #[test]
    fn half_page_scroll_moves_by_half_the_visible_rows() {
        let mut results = ResultsComponent::new();
        results.set_result(table(20));
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 30, 12), false))
            .unwrap();
        // 12-row area minus 2 border rows minus 1 header row = 9 visible
        // rows -> half page = 4.

        results.move_half_page_down();
        assert_eq!(results.selected, 4);

        results.move_half_page_up();
        assert_eq!(results.selected, 0);
    }

    #[test]
    fn selected_text_is_none_without_a_result() {
        let results = ResultsComponent::new();
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn selected_text_is_none_for_an_error() {
        let mut results = ResultsComponent::new();
        results.set_error("boom".to_string());
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn selected_text_tab_separates_the_selected_table_row() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![
                vec!["1".to_string(), "Ada".to_string()],
                vec!["2".to_string(), "Lin".to_string()],
            ],
            truncated: false,
        });
        results.move_down();

        assert_eq!(results.selected_text().as_deref(), Some("2\tLin"));
    }

    #[test]
    fn selected_text_pretty_prints_the_selected_document() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"a": 1}),
            serde_json::json!({"b": 2}),
        ]));
        results.move_down();

        assert_eq!(
            results.selected_text().as_deref(),
            Some(
                serde_json::to_string_pretty(&serde_json::json!({"b": 2}))
                    .unwrap()
                    .as_str()
            )
        );
    }

    #[test]
    fn draw_shows_the_last_table_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["42".to_string()]],
            truncated: false,
        });

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("42"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_documents_pretty_printed() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada"}),
        ]));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("Ada"), "buffer was: {text}");
    }

    #[test]
    fn draw_shows_the_last_error() {
        let mut results = ResultsComponent::new();
        results.set_error("syntax error".to_string());

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("syntax error"), "buffer was: {text}");
    }

    /// The border color of the panel's top-left corner, which is what
    /// tells the user where the keyboard focus is now that the title no
    /// longer spells it out.
    fn border_color(results: &mut ResultsComponent, focused: bool) -> Option<Color> {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, frame.area(), focused))
            .unwrap();
        terminal.backend().buffer().cell((0, 0)).unwrap().fg.into()
    }

    #[test]
    fn draw_marks_the_panel_as_focused_with_the_border_color() {
        let mut results = ResultsComponent::new();

        let focused = border_color(&mut results, true);
        let unfocused = border_color(&mut results, false);

        assert_eq!(focused, Some(theme().border_focused));
        assert_eq!(unfocused, Some(theme().border));
        assert_ne!(focused, unfocused);
    }

    #[test]
    fn draw_shows_the_row_count_in_the_title() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("3 rows"), "buffer was: {text}");
    }

    #[test]
    fn a_write_reports_what_it_affected_instead_of_looking_empty() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Affected { rows: 3 });

        let text = draw_component(&mut results, 40, 8);

        assert!(text.contains("OK"), "buffer was: {text}");
        assert!(text.contains("3 rows affected"), "buffer was: {text}");
        assert!(
            !text.contains("0 rows)"),
            "must not read like an empty result: {text}"
        );
    }

    #[test]
    fn a_write_has_nothing_to_select_or_yank() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Affected { rows: 3 });

        results.move_down();

        assert_eq!(results.selected, 0);
        assert_eq!(results.selected_text(), None);
    }

    #[test]
    fn a_single_row_is_not_pluralised() {
        let mut results = ResultsComponent::new();
        results.set_result(table(1));

        let text = draw_component(&mut results, 40, 10);

        assert!(text.contains("(1 row)"), "buffer was: {text}");
    }

    /// Three cities, two of them containing "han" (Hanoi twice) -- enough
    /// to tell filtering from jumping.
    fn cities() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "city".to_string()],
            rows: vec![
                vec!["1".to_string(), "Hanoi".to_string()],
                vec!["2".to_string(), "Da Nang".to_string()],
                vec!["3".to_string(), "Hanoi".to_string()],
            ],
            truncated: false,
        }
    }

    #[test]
    fn a_filter_hides_the_rows_that_do_not_match() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());

        results.set_filter("han");

        let text = draw_component(&mut results, 30, 8);
        assert!(text.contains("Hanoi"), "buffer was: {text}");
        assert!(!text.contains("Da Nang"), "buffer was: {text}");
    }

    #[test]
    fn filtering_is_case_insensitive_and_matches_any_column() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());

        results.set_filter("DA NA");

        assert_eq!(results.selected_text().as_deref(), Some("2\tDa Nang"));
    }

    #[test]
    fn row_numbers_stay_the_ones_from_the_whole_result() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.set_filter("nang");

        let backend = TestBackend::new(30, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 30, 8), false))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Row 0 is the border, row 1 the header, row 2 the only surviving
        // data row -- whose gutter must read 2, its place in the whole
        // result, not 1, its place in the filtered view.
        let line = row_text(&buffer, 2);
        assert!(line.contains("Da Nang"), "row was: {line:?}");
        assert_eq!(
            line.trim_start_matches('\u{2502}')
                .trim_start()
                .chars()
                .next(),
            Some('2'),
            "the surviving row must keep its own number: {line:?}"
        );
    }

    #[test]
    fn the_title_says_how_much_is_being_hidden() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.set_filter("han");

        let text = draw_component(&mut results, 50, 8);

        assert!(text.contains("2 of 3 rows"), "buffer was: {text}");
        assert!(text.contains("filter: han"), "buffer was: {text}");
    }

    #[test]
    fn acting_on_the_selection_while_filtered_uses_the_row_you_can_see() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.set_filter("han");

        results.move_down();

        assert_eq!(
            results.selected_row().map(|r| r[0].as_str()),
            Some("3"),
            "the second visible row is the third row of the result"
        );
    }

    #[test]
    fn a_cursor_past_the_end_of_a_narrower_filter_is_pulled_back_into_range() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.move_to_bottom();
        assert_eq!(results.selected, 2);

        results.set_filter("nang");

        assert_eq!(results.selected, 0, "only one row survives");
        assert!(
            results.selected_row().is_some(),
            "and it must be selectable"
        );
    }

    #[test]
    fn a_new_result_drops_the_filter_rather_than_hiding_rows_of_it() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.set_filter("han");

        results.set_result(cities());

        assert_eq!(results.filter(), "");
        assert_eq!(results.item_count(), 3);
    }

    #[test]
    fn a_refresh_after_an_edit_keeps_the_filter() {
        let mut results = ResultsComponent::new();
        results.set_result(cities());
        results.set_filter("han");

        results.set_result_keeping_cursor(cities());

        assert_eq!(results.filter(), "han");
        assert_eq!(results.item_count(), 2);
    }

    #[test]
    fn documents_are_filtered_on_their_json_text() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada"}),
            serde_json::json!({"name": "Lin"}),
        ]));

        results.set_filter("lin");

        assert_eq!(results.item_count(), 1);
        assert!(
            results.selected_text().unwrap().contains("Lin"),
            "the surviving document has to be the matching one"
        );
    }

    /// Two documents that don't share every field -- `age` only on the
    /// first, `city` only on the second -- and a nested object, all in one
    /// result: the shape `documents_as_table` actually has to handle.
    fn heterogeneous_docs() -> QueryResult {
        QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada", "age": 28}),
            serde_json::json!({"name": "Lin", "city": "Hanoi", "meta": {"vip": true}}),
        ])
    }

    #[test]
    fn toggling_document_view_switches_to_a_flattened_table_and_back() {
        let mut results = ResultsComponent::new();
        results.set_result(heterogeneous_docs());

        results.toggle_document_view();
        assert_eq!(results.columns(), ["age", "city", "meta", "name"]);
        assert_eq!(
            results.selected_row().map(|r| r.as_slice()),
            Some(
                [
                    "28".to_string(),
                    String::new(),
                    String::new(),
                    "Ada".to_string(),
                ]
                .as_slice()
            )
        );

        results.toggle_document_view();
        assert!(
            results.columns().is_empty(),
            "back in JSON view, there are no columns to name"
        );
    }

    #[test]
    fn a_missing_field_is_blank_not_null_and_a_nested_object_is_compact_json() {
        let mut results = ResultsComponent::new();
        results.set_result(heterogeneous_docs());
        results.toggle_document_view();
        results.move_down();

        let row = results.selected_row().expect("second document");
        assert_eq!(row[0], "", "no 'age' field on this document");
        assert_eq!(row[1], "Hanoi");
        assert_eq!(row[2], r#"{"vip":true}"#, "nested object as compact JSON");
        assert_eq!(row[3], "Lin");
    }

    #[test]
    fn toggling_document_view_is_a_no_op_for_a_table_or_affected_result() {
        let mut results = ResultsComponent::new();
        results.set_result(table(1));
        results.toggle_document_view();
        assert_eq!(results.columns(), ["id"], "still the real table, untouched");

        results.set_result(QueryResult::Affected { rows: 1 });
        results.toggle_document_view();
        assert!(results.columns().is_empty());
    }

    #[test]
    fn table_view_of_documents_draws_a_real_table_with_a_view_marker_in_the_title() {
        let mut results = ResultsComponent::new();
        results.set_result(heterogeneous_docs());
        results.toggle_document_view();

        let text = draw_component(&mut results, 60, 10);

        assert!(text.contains("table view"), "buffer was: {text}");
        assert!(text.contains("name"), "column header: {text}");
        assert!(text.contains("Hanoi"), "flattened cell value: {text}");
    }

    #[test]
    fn filtering_in_table_view_searches_the_flattened_cells() {
        let mut results = ResultsComponent::new();
        results.set_result(heterogeneous_docs());
        results.toggle_document_view();

        results.set_filter("hanoi");

        assert_eq!(results.item_count(), 1);
        // Columns are sorted alphabetically (age, city, meta, name) -- "name" is index 3.
        assert_eq!(results.selected_row().map(|r| r[3].as_str()), Some("Lin"));
    }

    #[test]
    fn yanking_in_table_view_is_tab_separated_like_any_other_table() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![
            serde_json::json!({"a": 1, "b": 2}),
        ]));
        results.toggle_document_view();

        assert_eq!(results.selected_text().as_deref(), Some("1\t2"));
    }

    #[test]
    fn a_refresh_recomputes_the_flattened_table_for_the_new_result() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![serde_json::json!({"a": 1})]));
        results.toggle_document_view();

        results.set_result(QueryResult::Documents(vec![serde_json::json!({"b": 2})]));

        assert_eq!(
            results.columns(),
            ["b"],
            "the view preference persists, but the columns must reflect the new result"
        );
    }

    /// A single-column table whose value is a compact JSON object -- a
    /// jsonb column, in a shape a real driver would actually return.
    fn jsonb_row() -> QueryResult {
        QueryResult::Table {
            columns: vec!["id".to_string(), "metadata".to_string()],
            rows: vec![vec![
                "1".to_string(),
                r#"{"theme":"dark","flags":["a","b"]}"#.to_string(),
            ]],
            truncated: false,
        }
    }

    #[test]
    fn toggle_preview_pretty_prints_a_json_object_value() {
        let mut results = ResultsComponent::new();
        results.set_result(jsonb_row());
        results.next_column(); // onto "metadata"

        results.toggle_preview();
        let text = draw_component(&mut results, 60, 16);

        assert!(text.contains("full value"), "buffer was: {text}");
        assert!(
            text.contains("\"theme\""),
            "pretty-printed JSON has its own line per key: {text}"
        );
        assert!(text.contains("\"dark\""), "buffer was: {text}");
    }

    #[test]
    fn toggle_preview_leaves_a_plain_value_as_is() {
        let mut results = ResultsComponent::new();
        results.set_result(jsonb_row()); // "id" column, value "1"

        results.toggle_preview();
        let text = draw_component(&mut results, 60, 16);

        assert!(
            text.contains("full value"),
            "a non-JSON value still gets a preview panel: {text}"
        );
    }

    #[test]
    fn toggle_preview_is_a_no_op_without_a_selected_cell() {
        let mut results = ResultsComponent::new();
        results.set_result(QueryResult::Documents(vec![serde_json::json!({"a": 1})]));

        results.toggle_preview();
        let text = draw_component(&mut results, 60, 16);

        assert!(
            !text.contains("full value"),
            "documents have no cell to preview: {text}"
        );
    }

    #[test]
    fn toggling_again_closes_the_preview() {
        let mut results = ResultsComponent::new();
        results.set_result(jsonb_row());
        results.next_column();
        results.toggle_preview();

        results.toggle_preview();
        let text = draw_component(&mut results, 60, 16);

        assert!(!text.contains("full value"), "buffer was: {text}");
    }

    #[test]
    fn moving_the_cursor_closes_an_open_preview() {
        let mut results = ResultsComponent::new();
        results.set_result(jsonb_row());
        results.next_column();
        results.toggle_preview();

        results.prev_column();
        let text = draw_component(&mut results, 60, 16);

        assert!(
            !text.contains("full value"),
            "a stale preview would show the wrong cell: {text}"
        );
    }

    #[test]
    fn a_new_result_closes_any_open_preview() {
        let mut results = ResultsComponent::new();
        results.set_result(jsonb_row());
        results.next_column();
        results.toggle_preview();

        results.set_result(jsonb_row());
        let text = draw_component(&mut results, 60, 16);

        assert!(!text.contains("full value"), "buffer was: {text}");
    }

    #[test]
    fn draw_selection_highlight_tracks_selected() {
        let mut results = ResultsComponent::new();
        results.set_result(table(3));
        results.move_down();

        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| results.draw(frame, Rect::new(0, 0, 20, 10), false))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Row 0 is the border+title, row 1 is the column header, row 2 is
        // the first data row ("0"), row 3 is the selected row ("1").
        let unselected_cell = buffer.cell((1, 2)).unwrap();
        let selected_cell = buffer.cell((1, 3)).unwrap();
        assert_eq!(selected_cell.bg, theme().selection_bg);
        assert!(selected_cell.modifier.contains(Modifier::BOLD));
        assert_ne!(unselected_cell.bg, theme().selection_bg);
    }
}
