//! The database navigator: one tree over *every* saved connection, not
//! just the one the current tab is on. This is why it lives in the app
//! shell rather than inside a connector's screen -- only `RootComponent`
//! knows what other connections exist, which of them are open, and on which
//! tab.
//!
//! It replaces the per-screen schema sidebar. Having both would mean the
//! same table listed twice on one screen, and only one of the two could
//! ever reach another connection.
//!
//! Not a `Component`: like the query screen's overlays, it's driven by its
//! host, which owns focus and is the only thing able to act on a choice
//! (open a tab, switch to one, insert a name into its editor).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use tradar_core::action::{CrudOp, OutlineEntry};
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker, TextInput};
use tradar_core::vim_list::{self, VimMove};

/// What the host has to know about one connection to draw it: its name,
/// whether it's open (and where), and what it has to browse.
pub struct NavConnection {
    pub name: String,
    /// The tab this connection is open on, if any. `None` means it has
    /// never been connected in this session -- so there is nothing under it
    /// yet, and opening it is a connect.
    pub tab: Option<usize>,
    pub outline: Vec<OutlineEntry>,
    pub error: Option<String>,
    /// Whether the open tab's connection is still reachable, straight from
    /// `Component::connection_alive`. `None` for a connection with no open
    /// tab -- there's nothing to be alive or dead yet, distinct from a tab
    /// whose screen doesn't report a status at all (also `None`, but that
    /// can't happen in practice since every screen here is a query screen).
    pub alive: Option<bool>,
}

/// One visible line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Connection(usize),
    /// `entry` indexes into that connection's `outline`.
    Entry {
        connection: usize,
        entry: usize,
    },
}

/// What the host should do about the row the user just chose.
pub enum NavOutcome {
    /// Connect this connection (by index into the list the host passed in)
    /// -- it isn't open yet, so there's nothing to expand until it is.
    Open(usize),
    /// Switch to the tab this connection is already on.
    Focus(usize),
    /// Switch to `tab` and put `name` into its editor.
    Insert { tab: usize, name: String },
    /// Switch to `tab` and insert a CRUD skeleton for `name` into its
    /// editor -- `c`/`r`/`u`/`d` on a table/collection/index/key row.
    Snippet {
        tab: usize,
        name: String,
        op: CrudOp,
    },
}

#[derive(Default)]
pub struct NavigatorComponent {
    /// Index into the *visible rows*, not into any one list.
    selected: usize,
    /// `(connection index, entry index)` pairs whose children are shown.
    /// Keyed by name rather than index would survive reordering, but the
    /// list only changes when connections are added or removed -- at which
    /// point a stale expansion is harmless.
    expanded: std::collections::HashSet<(usize, usize)>,
    /// Connections whose own subtree is shown.
    open: std::collections::HashSet<usize>,
    list_state: ListState,
    list_area: Rect,
    visible_height: usize,
    /// A case-insensitive substring narrowing the tree -- see
    /// `visible_rows`. Kept even while `filter_input` is closed, so the
    /// title can still say what's currently applied and a fresh `/`
    /// prefills it rather than starting over.
    filter: String,
    /// `Some` while the filter bar has the keys -- see `open_filter`.
    filter_input: Option<TextInput>,
    /// Recognizes a second click on the same row as "activate" (same as
    /// `Enter`) rather than "select (again)".
    double_click: DoubleClickTracker,
}

impl NavigatorComponent {
    pub fn new() -> Self {
        Self::default()
    }

    /// The visible lines, in order. Rebuilt on demand rather than cached:
    /// it's a walk over a list that's already in memory, and a cache would
    /// have to be invalidated every time a schema arrives.
    fn rows(&self, connections: &[NavConnection]) -> Vec<Row> {
        let mut rows = Vec::new();
        for (index, connection) in connections.iter().enumerate() {
            rows.push(Row::Connection(index));
            // A connection that's been closed since it was opened has
            // nothing under it any more, whatever the expansion state says
            // -- so the tree corrects itself instead of needing the host to
            // remember to tidy up.
            if !self.open.contains(&index) || connection.tab.is_none() {
                continue;
            }
            let mut skip_children_of: Option<u8> = None;
            for (position, entry) in connection.outline.iter().enumerate() {
                // Everything below a collapsed entry is hidden until the
                // outline comes back up to that entry's own depth.
                if let Some(depth) = skip_children_of {
                    if entry.depth > depth {
                        continue;
                    }
                    skip_children_of = None;
                }
                rows.push(Row::Entry {
                    connection: index,
                    entry: position,
                });
                if entry.has_children && !self.expanded.contains(&(index, position)) {
                    skip_children_of = Some(entry.depth);
                }
            }
        }
        rows
    }

    /// `rows()` narrowed by `filter`, when one's applied. A connection row
    /// survives if its own name matches *or* any row under it does --
    /// otherwise finding a table by name would hide the very connection
    /// row you'd need to see (and click/expand) to reach it. An entry row
    /// only survives by matching itself; nothing here reaches into a
    /// collapsed subtree to reveal a match, same as `rows()` never did --
    /// filtering narrows what's already open rather than searching what
    /// isn't.
    fn visible_rows(&self, connections: &[NavConnection]) -> Vec<Row> {
        let all = self.rows(connections);
        if self.filter.is_empty() {
            return all;
        }
        let needle = self.filter.to_lowercase();
        let matches = |row: &Row| match *row {
            Row::Connection(index) => connections[index].name.to_lowercase().contains(&needle),
            Row::Entry { connection, entry } => {
                let entry = &connections[connection].outline[entry];
                entry.label.to_lowercase().contains(&needle)
                    || entry.detail.to_lowercase().contains(&needle)
            }
        };
        // Which connections have a matching entry under them, gathered in
        // one pass rather than re-scanning `all` to find each match's
        // owning connection row -- that would be quadratic in the number
        // of matches.
        let mut connections_with_a_match: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        for row in &all {
            if let Row::Entry { connection, .. } = row
                && matches(row)
            {
                connections_with_a_match.insert(*connection);
            }
        }
        all.into_iter()
            .filter(|row| match row {
                Row::Connection(index) => matches(row) || connections_with_a_match.contains(index),
                Row::Entry { .. } => matches(row),
            })
            .collect()
    }

    /// Opens the filter bar, prefilled with whatever's already applied --
    /// refining a search is "add characters", not "start over".
    pub fn open_filter(&mut self) {
        self.filter_input = Some(TextInput::new(&self.filter));
    }

    pub fn is_filtering(&self) -> bool {
        self.filter_input.is_some()
    }

    /// One key while the filter bar has the keys. `Esc` cancels -- clears
    /// the bar *and* whatever was applied, back to the unfiltered tree
    /// (matching the buffer-search bar elsewhere: `Esc` undoes, it doesn't
    /// just close). `Enter` keeps the filter and closes the bar. Anything
    /// else is text editing, applied live so the tree narrows as you type.
    pub fn filter_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let Some(input) = self.filter_input.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                self.filter_input = None;
                self.filter.clear();
            }
            KeyCode::Enter => self.filter_input = None,
            _ => {
                input.handle_key_event(code, modifiers);
                self.filter = input.text();
            }
        }
    }

    fn selected_row(&self, connections: &[NavConnection]) -> Option<Row> {
        self.visible_rows(connections).get(self.selected).copied()
    }

    pub fn apply_move(&mut self, mv: VimMove, connections: &[NavConnection]) {
        let len = self.visible_rows(connections).len();
        vim_list::apply(mv, &mut self.selected, len, self.visible_height);
    }

    /// Opens the row under the cursor: a connection's subtree, or an
    /// entry's children. Returns `Some(NavOutcome::Open)` when the
    /// connection isn't connected yet -- there's nothing to show until it
    /// is, so "expand" and "connect" are the same keystroke.
    pub fn expand(&mut self, connections: &[NavConnection]) -> Option<NavOutcome> {
        match self.selected_row(connections)? {
            Row::Connection(index) => {
                if connections[index].tab.is_none() {
                    return Some(NavOutcome::Open(index));
                }
                self.open.insert(index);
                None
            }
            Row::Entry { connection, entry } => {
                if connections[connection].outline[entry].has_children {
                    self.expanded.insert((connection, entry));
                }
                None
            }
        }
    }

    /// Closes the row under the cursor. On a row that has nothing open, it
    /// closes the parent and moves the cursor there -- otherwise the line
    /// you were on would disappear from under you.
    pub fn collapse(&mut self, connections: &[NavConnection]) {
        let Some(row) = self.selected_row(connections) else {
            return;
        };
        match row {
            Row::Connection(index) => {
                self.open.remove(&index);
            }
            Row::Entry { connection, entry } => {
                if self.expanded.remove(&(connection, entry)) {
                    return;
                }
                // Not expanded, so close whatever holds it: its parent
                // entry if it has one, else the connection.
                let depth = connections[connection].outline[entry].depth;
                let parent = (0..entry)
                    .rev()
                    .find(|i| connections[connection].outline[*i].depth < depth);
                match parent {
                    Some(parent) => {
                        self.expanded.remove(&(connection, parent));
                        self.select_row(
                            connections,
                            Row::Entry {
                                connection,
                                entry: parent,
                            },
                        );
                    }
                    None => {
                        self.open.remove(&connection);
                        self.select_row(connections, Row::Connection(connection));
                    }
                }
            }
        }
    }

    fn select_row(&mut self, connections: &[NavConnection], row: Row) {
        if let Some(index) = self
            .visible_rows(connections)
            .iter()
            .position(|r| *r == row)
        {
            self.selected = index;
        }
    }

    /// What `enter` means on the current row: connect it, jump to the tab
    /// it's already on, or put the selected name into that tab's editor.
    pub fn choose(&mut self, connections: &[NavConnection]) -> Option<NavOutcome> {
        match self.selected_row(connections)? {
            Row::Connection(index) => Some(match connections[index].tab {
                Some(tab) => NavOutcome::Focus(tab),
                None => NavOutcome::Open(index),
            }),
            Row::Entry { connection, entry } => {
                let tab = connections[connection].tab?;
                Some(NavOutcome::Insert {
                    tab,
                    name: connections[connection].outline[entry].label.clone(),
                })
            }
        }
    }

    /// What `c`/`r`/`u`/`d` mean on the current row: a CRUD snippet for
    /// the table/collection/index/key it names. Only a **depth-0** entry
    /// qualifies -- a single column (depth 1) or a connection row isn't a
    /// whole row/document/key to build a statement against.
    pub fn choose_snippet(
        &mut self,
        connections: &[NavConnection],
        op: CrudOp,
    ) -> Option<NavOutcome> {
        let Row::Entry { connection, entry } = self.selected_row(connections)? else {
            return None;
        };
        let outline_entry = &connections[connection].outline[entry];
        if outline_entry.depth != 0 {
            return None;
        }
        let tab = connections[connection].tab?;
        Some(NavOutcome::Snippet {
            tab,
            name: outline_entry.label.clone(),
            op,
        })
    }

    /// Selects whatever row was clicked. Returns whether this was a second
    /// click on that same row within the double-click window -- the host
    /// treats that the same as `Enter` (see `choose`).
    pub fn click(&mut self, column: u16, row: u16, connections: &[NavConnection]) -> bool {
        if !ui::contains(self.list_area, column, row) {
            return false;
        }
        let inner = Rect {
            x: self.list_area.x.saturating_add(1),
            y: self.list_area.y.saturating_add(1),
            width: self.list_area.width.saturating_sub(2),
            height: self.list_area.height.saturating_sub(2),
        };
        let count = self.visible_rows(connections).len();
        let Some(index) = ui::index_at(inner, self.list_state.offset(), row, count) else {
            return false;
        };
        self.selected = index;
        self.double_click.click(index)
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        ui::contains(self.list_area, column, row)
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, conns: &[NavConnection]) {
        let theme = theme();
        let (list_area, filter_bar_area) = if self.filter_input.is_some() {
            let (list_area, bar) = ui::split_bottom_bar(area, 1);
            (list_area, Some(bar))
        } else {
            (area, None)
        };

        let rows = self.visible_rows(conns);
        let items: Vec<ListItem> = rows
            .iter()
            .map(|row| match row {
                Row::Connection(index) => {
                    let connection = &conns[*index];
                    let marker = if self.open.contains(index) && connection.tab.is_some() {
                        "▾ "
                    } else {
                        "▸ "
                    };
                    let mut spans = vec![
                        Span::styled(marker.to_string(), Style::default().fg(theme.text_dim)),
                        Span::styled(
                            connection.name.clone(),
                            // A connection that isn't open yet is dimmed:
                            // there's nothing under it until you connect,
                            // and that should look like a state, not a bug.
                            match connection.tab {
                                Some(_) => {
                                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
                                }
                                None => Style::default().fg(theme.text_dim),
                            },
                        ),
                    ];
                    // Quiet when alive (a healthy connection needs no
                    // badge), called out when it's dropped -- the whole
                    // point is noticing that without running a query first.
                    if connection.alive == Some(false) {
                        spans.push(Span::styled(
                            "  ✗ disconnected",
                            Style::default().fg(theme.error),
                        ));
                    }
                    if let Some(error) = &connection.error {
                        spans.push(Span::styled(
                            format!("  {error}"),
                            Style::default().fg(theme.error),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                }
                Row::Entry { connection, entry } => {
                    let entry_index = *entry;
                    let entry = &conns[*connection].outline[entry_index];
                    let marker = match (
                        entry.has_children,
                        self.expanded.contains(&(*connection, entry_index)),
                    ) {
                        (false, _) => "  ",
                        (true, true) => "▾ ",
                        (true, false) => "▸ ",
                    };
                    let indent = "  ".repeat(entry.depth as usize + 1);
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{indent}{marker}"),
                            Style::default().fg(theme.text_dim),
                        ),
                        Span::styled(entry.label.clone(), Style::default().fg(theme.text)),
                        Span::styled(
                            if entry.detail.is_empty() {
                                String::new()
                            } else {
                                format!("  {}", entry.detail)
                            },
                            Style::default().fg(theme.text_dim),
                        ),
                    ]))
                }
            })
            .collect();

        if rows.is_empty() {
            self.list_state.select(None);
        } else {
            self.selected = self.selected.min(rows.len() - 1);
            self.list_state.select(Some(self.selected));
        }

        self.visible_height = list_area.height.saturating_sub(2) as usize;
        self.list_area = list_area;
        let title = if self.filter.is_empty() {
            format!("Navigator ({})", conns.len())
        } else {
            format!("Navigator ({}) — filter: {}", conns.len(), self.filter)
        };
        let list = List::new(items)
            .block(ui::panel(&title, focused))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        if let (Some(bar_area), Some(input)) = (filter_bar_area, &self.filter_input) {
            let mut spans = vec![Span::styled("/", Style::default().fg(theme.accent))];
            spans.extend(input.spans(focused));
            frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn entry(depth: u8, label: &str, has_children: bool) -> OutlineEntry {
        OutlineEntry {
            depth,
            label: label.to_string(),
            detail: String::new(),
            has_children,
        }
    }

    /// Two connections: `local` open on tab 0 with a `users` table that has
    /// an `id` column, and `remote` never connected.
    fn connections() -> Vec<NavConnection> {
        vec![
            NavConnection {
                name: "local".to_string(),
                tab: Some(0),
                outline: vec![entry(0, "users", true), entry(1, "id", false)],
                error: None,
                alive: Some(true),
            },
            NavConnection {
                name: "remote".to_string(),
                tab: None,
                outline: Vec::new(),
                error: None,
                alive: None,
            },
        ]
    }

    #[test]
    fn a_closed_connection_shows_nothing_underneath_it() {
        let navigator = NavigatorComponent::new();

        assert_eq!(navigator.rows(&connections()).len(), 2, "two connections");
    }

    #[test]
    fn opening_a_connected_connection_reveals_its_tables_but_not_their_columns() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();

        navigator.expand(&conns);

        assert_eq!(
            navigator.rows(&conns),
            vec![
                Row::Connection(0),
                Row::Entry {
                    connection: 0,
                    entry: 0
                },
                Row::Connection(1),
            ],
            "the column stays hidden until the table itself is opened"
        );
    }

    #[test]
    fn opening_a_table_reveals_its_columns() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);

        navigator.expand(&conns);

        assert_eq!(navigator.rows(&conns).len(), 4);
    }

    #[test]
    fn opening_a_connection_that_is_not_connected_asks_the_host_to_connect_it() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.apply_move(VimMove::Bottom, &conns);

        match navigator.expand(&conns) {
            Some(NavOutcome::Open(index)) => assert_eq!(index, 1),
            _ => panic!("a closed connection has nothing to expand until it's open"),
        }
    }

    #[test]
    fn choosing_a_name_reports_the_tab_it_belongs_to() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);

        match navigator.choose(&conns) {
            Some(NavOutcome::Insert { tab, name }) => {
                assert_eq!((tab, name.as_str()), (0, "users"))
            }
            _ => panic!("expected the table name, aimed at its own tab"),
        }
    }

    #[test]
    fn choosing_a_snippet_on_a_table_reports_the_tab_name_and_op() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);

        match navigator.choose_snippet(&conns, CrudOp::Read) {
            Some(NavOutcome::Snippet { tab, name, op }) => {
                assert_eq!((tab, name.as_str(), op), (0, "users", CrudOp::Read))
            }
            _ => panic!("expected a snippet request aimed at the table's own tab"),
        }
    }

    #[test]
    fn choosing_a_snippet_on_a_column_does_nothing() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);

        assert!(
            navigator.choose_snippet(&conns, CrudOp::Read).is_none(),
            "a single column isn't a whole row/document/key to build a statement against"
        );
    }

    #[test]
    fn choosing_a_snippet_on_a_connection_row_does_nothing() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();

        assert!(navigator.choose_snippet(&conns, CrudOp::Read).is_none());
    }

    #[test]
    fn choosing_an_already_open_connection_switches_to_its_tab() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();

        match navigator.choose(&conns) {
            Some(NavOutcome::Focus(tab)) => assert_eq!(tab, 0),
            _ => panic!("expected a jump to the tab it's already on"),
        }
    }

    #[test]
    fn collapsing_on_a_child_closes_its_parent_and_moves_the_cursor_there() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);
        assert_eq!(
            navigator.selected_row(&conns),
            Some(Row::Entry {
                connection: 0,
                entry: 1
            })
        );

        navigator.collapse(&conns);

        assert_eq!(
            navigator.selected_row(&conns),
            Some(Row::Entry {
                connection: 0,
                entry: 0
            }),
            "the cursor must not be left pointing at a line that just vanished"
        );
    }

    #[test]
    fn collapsing_a_top_level_table_closes_the_connection_and_lands_on_it() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);

        navigator.collapse(&conns);

        assert_eq!(navigator.selected_row(&conns), Some(Row::Connection(0)));
        assert_eq!(navigator.rows(&conns).len(), 2);
    }

    #[test]
    fn a_connection_that_gets_closed_stops_showing_a_tree() {
        let mut conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns);
        navigator.apply_move(VimMove::Down, &conns);
        navigator.expand(&conns);
        assert_eq!(navigator.rows(&conns).len(), 4);

        // The tab holding it went back to the picker.
        conns[0].tab = None;
        conns[0].outline.clear();

        assert_eq!(
            navigator.rows(&conns).len(),
            2,
            "a stale expansion must not outlive the connection it described"
        );
    }

    #[test]
    fn draw_calls_out_a_dropped_connection_and_stays_quiet_for_a_live_one() {
        let mut conns = connections();
        let mut navigator = NavigatorComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| navigator.draw(frame, frame.area(), true, &conns))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains("disconnected"),
            "a live connection needs no badge: {text}"
        );

        conns[0].alive = Some(false);
        terminal
            .draw(|frame| navigator.draw(frame, frame.area(), true, &conns))
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("disconnected"), "buffer was: {text}");
    }

    #[test]
    fn a_second_click_on_the_same_row_is_reported_as_a_double_click() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| navigator.draw(frame, frame.area(), true, &conns))
            .unwrap();

        // Row 0 is the border, row 1 is "local" (the first connection).
        assert!(!navigator.click(2, 1, &conns), "a first click only selects");
        assert!(
            navigator.click(2, 1, &conns),
            "a second click on the same row is a double-click"
        );
    }

    #[test]
    fn clicks_on_different_rows_never_report_a_double_click() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| navigator.draw(frame, frame.area(), true, &conns))
            .unwrap();

        navigator.click(2, 1, &conns); // "local"
        let double_clicked = navigator.click(2, 2, &conns); // "remote"

        assert!(!double_clicked);
        assert_eq!(navigator.selected, 1, "the second click still selects");
    }

    #[test]
    fn filtering_by_connection_name_hides_the_one_that_does_not_match() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.open_filter();
        for c in "remo".chars() {
            navigator.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert_eq!(
            navigator.visible_rows(&conns),
            vec![Row::Connection(1)],
            "case-insensitive substring match on the connection name"
        );
    }

    #[test]
    fn filtering_by_a_table_name_still_shows_its_own_connection_row() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.expand(&conns); // opens `local`'s subtree
        navigator.open_filter();
        for c in "user".chars() {
            navigator.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        assert_eq!(
            navigator.visible_rows(&conns),
            vec![
                Row::Connection(0),
                Row::Entry {
                    connection: 0,
                    entry: 0
                },
            ],
            "the table's own connection row must survive so there's something to click/expand"
        );
    }

    #[test]
    fn esc_on_the_filter_bar_clears_the_filter_entirely() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.open_filter();
        navigator.filter_key_event(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(navigator.visible_rows(&conns).len(), 0);

        navigator.filter_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(!navigator.is_filtering());
        assert_eq!(
            navigator.visible_rows(&conns),
            navigator.rows(&conns),
            "Esc must undo the filter, not just close the bar"
        );
    }

    #[test]
    fn enter_on_the_filter_bar_keeps_the_filter_and_closes_it() {
        let conns = connections();
        let mut navigator = NavigatorComponent::new();
        navigator.open_filter();
        navigator.filter_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        navigator.filter_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(!navigator.is_filtering());
        assert_eq!(navigator.visible_rows(&conns), vec![Row::Connection(1)]);
    }

    #[test]
    fn reopening_the_filter_prefills_what_was_already_applied() {
        let mut navigator = NavigatorComponent::new();
        navigator.open_filter();
        navigator.filter_key_event(KeyCode::Char('x'), KeyModifiers::NONE);
        navigator.filter_key_event(KeyCode::Enter, KeyModifiers::NONE);

        navigator.open_filter();

        assert_eq!(navigator.filter_input.as_ref().unwrap().text(), "x");
    }
}
