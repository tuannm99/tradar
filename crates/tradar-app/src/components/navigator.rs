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

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};

use tradar_core::action::OutlineEntry;
use tradar_core::theme::theme;
use tradar_core::ui;
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

    fn selected_row(&self, connections: &[NavConnection]) -> Option<Row> {
        self.rows(connections).get(self.selected).copied()
    }

    pub fn apply_move(&mut self, mv: VimMove, connections: &[NavConnection]) {
        let len = self.rows(connections).len();
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
        if let Some(index) = self.rows(connections).iter().position(|r| *r == row) {
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

    /// Selects whatever row was clicked. Returns whether the click landed
    /// in this panel at all.
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
        let count = self.rows(connections).len();
        if let Some(index) = ui::index_at(inner, self.list_state.offset(), row, count) {
            self.selected = index;
        }
        true
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        ui::contains(self.list_area, column, row)
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, conns: &[NavConnection]) {
        let theme = theme();
        let rows = self.rows(conns);
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

        self.visible_height = area.height.saturating_sub(2) as usize;
        self.list_area = area;
        let list = List::new(items)
            .block(ui::panel(&format!("Navigator ({})", conns.len()), focused))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut self.list_state);
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
}
