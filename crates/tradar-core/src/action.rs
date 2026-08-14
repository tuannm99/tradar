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
        /// `RootComponent::handle_key_event` in `tradar-app`. Carried end to
        /// end through `main.rs`'s connect task and back so a reply lands on
        /// the right tab even if the user switched tabs while it was in
        /// flight.
        tab: usize,
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
    /// Whether the schema behind `outline` failed to load, for the
    /// navigator to say so rather than showing an empty tree that looks
    /// like an empty database.
    fn outline_error(&self) -> Option<String> {
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
