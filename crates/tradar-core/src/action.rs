//! The application-level `Action` message and the `Component` trait every
//! `Screen` implements. This enum is closed and stays closed: no connector
//! adds a variant to it. Everything specific to one connector's own key
//! handling/state travels as a direct synchronous method call on that
//! connector's `Session`, never through this type -- see "Screen không bao
//! giờ làm IO" in docs/architecture.md.

use crossterm::event::{KeyCode, KeyModifiers, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::storage::SavedConnection;

pub enum Action {
    Quit,
    OpenRequested {
        connection: SavedConnection,
        epoch: u64,
        /// Which tab issued this request. Set by the originating
        /// `ConnectionPickerComponent` to a placeholder and overwritten by
        /// `RootComponent` with the real tab index -- see
        /// `RootComponent::route_open_requested` in `tradar-app`. Carried end
        /// to end through `main.rs`'s connect task and back so a reply lands
        /// on the right tab even if the user switched tabs while it was in
        /// flight.
        tab: usize,
        /// `false` (the normal case: `Enter`, a single click via double-click)
        /// lets `RootComponent` redirect this into switching to a tab that
        /// already has `connection` open, instead of dialing a second,
        /// independent session to it. `true` (`Ctrl+Enter`, or the context
        /// menu's "Open new session") skips that redirect -- the user
        /// explicitly asked for a parallel session, same idea as
        /// Ctrl+clicking a link opens a new browser tab instead of switching
        /// to one that's already open on it.
        force_new: bool,
    },
    Opened {
        connection: SavedConnection,
        screen: Box<dyn Component>,
        epoch: u64,
        tab: usize,
    },
    OpenFailed {
        error: String,
        epoch: u64,
        tab: usize,
    },
    BackToPicker,
    /// Raise the key-bindings overlay. Like `Quit`, this is a property of
    /// the app shell rather than of any connector -- `RootComponent` owns
    /// the overlay, so a screen that wants it just says so.
    ShowHelp,
}

/// One line of what a screen has to browse -- a table, a collection, an
/// index, a column of one. The navigator in the app shell draws these
/// without knowing what any of them *are*: a screen flattens its own
/// structure into labels and depths, and the shell only has to lay them
/// out. Same contract as `restore_state`: opaque to everyone but the
/// screen that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Nesting level: 0 for a top-level entry, 1 for its children.
    pub depth: u8,
    pub label: String,
    /// Dimmed text after the label -- a column's type, say. Empty for none.
    pub detail: String,
    /// Whether this entry has children below it, so the navigator knows
    /// whether to draw an expand marker. An entry with nothing under it
    /// must not look like it opens.
    pub has_children: bool,
}

/// Which CRUD statement to generate a snippet for -- see
/// `Component::crud_snippet`. Lives here rather than in
/// `tradar-query-workbench` because `Component` (this trait) can't depend
/// on a crate that depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrudOp {
    Create,
    Read,
    Update,
    Delete,
}

pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    /// Handles a click or scroll. Defaults to ignoring it: a component
    /// that hasn't been taught where it was drawn can't hit-test, and
    /// silently doing nothing is better than acting on a guess.
    fn handle_mouse_event(&mut self, _event: MouseEvent) -> Option<Action> {
        None
    }
    fn update(&mut self, action: Action) -> Option<Action>;
    /// Returns whether anything changed that's worth a redraw. The default
    /// (no-op, nothing changed) covers every `Component` that only ever
    /// changes state in response to a key press or an `Action`.
    fn tick(&mut self) -> bool {
        false
    }
    /// What this screen would want handed back if the app restarted --
    /// for a query screen, the text in its editor. Opaque to everyone but
    /// the screen itself: the app just persists the string and passes it
    /// to `Session::build_screen` next time. `None` for a screen with
    /// nothing worth restoring, which is the default.
    fn restore_state(&self) -> Option<String> {
        None
    }
    /// What this screen offers the navigator to browse -- see
    /// `OutlineEntry`. Empty by default: a connector with nothing tree-
    /// shaped to show simply doesn't appear expandable.
    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }
    /// Puts `text` wherever this screen accepts typed input (the query
    /// editor, for a query screen), which is how choosing a name in the
    /// navigator gets it into a query. A screen with nowhere to put it
    /// ignores it, the default.
    fn insert_text(&mut self, _text: &str) {}
    /// A skeleton statement for `op` against the outline entry named
    /// `name` (a table, collection, index, or key -- whatever `outline`'s
    /// depth-0 entries are for this screen), in this screen's own query
    /// language. `None` -- the default -- means this screen has no notion
    /// of CRUD statements, or doesn't recognize `name`; the navigator then
    /// does nothing rather than inserting a blank line.
    fn crud_snippet(&self, _name: &str, _op: CrudOp) -> Option<String> {
        None
    }
    /// Whether the schema behind `outline` failed to load, for the
    /// navigator to say so rather than showing an empty tree that looks
    /// like an empty database.
    fn outline_error(&self) -> Option<String> {
        None
    }
    /// Whether this screen's own connection is currently reachable. `None`
    /// -- the default -- means "not applicable" (the connection picker, an
    /// overlay, anything with no live connection of its own): the navigator
    /// only draws a status marker for a screen that actually reports one.
    /// A query screen reports its `QueryEngine`'s periodic ping result.
    fn connection_alive(&self) -> Option<bool> {
        None
    }
    /// The key hints this screen wants in the app shell's bottom status
    /// bar, alongside the always-present navigator/tab/help ones --
    /// `RootComponent` used to hardcode `QueryScreenComponent`'s own
    /// (run/focus/history/back) here for every active screen, which was
    /// silently wrong the moment a non-query `Screen` (RabbitMQ, ...)
    /// existed. Empty by default: a screen with nothing to hint just shows
    /// the shared ones.
    fn status_hints(&self) -> Vec<crate::ui::Hint> {
        Vec::new()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
