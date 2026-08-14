//! Key bindings, resolved by *context* instead of matched inline. A
//! component asks "what command did the user just invoke here?" rather than
//! matching `KeyCode::Char('j')` itself, which is what lets
//! `~/.config/tradar/config.toml` rebind anything without touching
//! component code -- and lets the help overlay list the bindings actually
//! in effect rather than a hand-maintained cheatsheet that drifts.
//!
//! Scope note: the vim keys *inside* the query editor (`i`/`a`/`o`/`x`/`dd`/
//! `hjkl`) are deliberately **not** remappable -- they're standard vim, and
//! leaving them fixed keeps the config small and un-footgunny. Everything
//! else (tabs, quit, run, save/open, history, yank, focus, and list
//! navigation) goes through here.

use std::collections::HashMap;
use std::sync::OnceLock;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::vim_list::VimMove;

static KEYMAP: OnceLock<Keymap> = OnceLock::new();

/// The active key bindings -- the built-in defaults until `set_keymap` runs.
pub fn keymap() -> &'static Keymap {
    KEYMAP.get_or_init(Keymap::default)
}

/// Installs `keymap` process-wide. Only the first call wins; the app calls
/// this once at startup.
pub fn set_keymap(keymap: Keymap) {
    let _ = KEYMAP.set(keymap);
}

/// Where a key was pressed. The same physical key can mean different things
/// in different contexts (`enter` opens a connection in the picker, inserts
/// a schema name on the query screen), so every lookup names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// Checked before anything else, on every screen.
    Global,
    Picker,
    QueryScreen,
    /// Only while the schema sidebar has focus. Split out from
    /// `QueryScreen` so the same key can mean different things per pane
    /// (`l` expands a table here, scrolls the table right in `Results`)
    /// without the two bindings colliding -- the screen passes only the
    /// focused pane's context to `resolve_in`.
    Sidebar,
    /// Only while the results pane has focus.
    Results,
    /// Shared by every selectable list: the connection picker, the schema
    /// sidebar, the results pane, and the history overlay.
    List,
    /// The file-path prompt and the history overlay.
    Prompt,
    /// Only while the autocomplete popup is showing. Checked before the
    /// editor's own keys, so `tab` accepts a suggestion when one is on
    /// screen and cycles panes when none is.
    Completion,
}

impl Context {
    pub fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Picker => "picker",
            Self::QueryScreen => "query-screen",
            Self::Sidebar => "sidebar",
            Self::Results => "results",
            Self::List => "list",
            Self::Prompt => "prompt",
            Self::Completion => "completion",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "global" => Self::Global,
            "picker" => Self::Picker,
            "query-screen" => Self::QueryScreen,
            "sidebar" => Self::Sidebar,
            "results" => Self::Results,
            "list" => Self::List,
            "prompt" => Self::Prompt,
            "completion" => Self::Completion,
            _ => return None,
        })
    }

    /// Every context, in the order the help overlay lists them.
    pub fn all() -> [Self; 8] {
        [
            Self::Global,
            Self::Picker,
            Self::QueryScreen,
            Self::Sidebar,
            Self::Results,
            Self::List,
            Self::Prompt,
            Self::Completion,
        ]
    }
}

/// Everything a key can be bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    // Global
    Quit,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    // Picker
    Open,
    NewConnection,
    EditConnection,
    DeleteConnection,
    // Query screen
    Back,
    RunQuery,
    /// Abandon the query that's currently running.
    CancelQuery,
    /// Run every statement in the buffer, not just the one at the cursor.
    RunAll,
    CycleFocus,
    SaveFile,
    OpenFile,
    History,
    ExportCurl,
    Yank,
    InsertName,
    Help,
    /// Move the cell cursor sideways in the results grid. The table
    /// scrolls to follow it, so this covers horizontal scrolling too.
    PrevColumn,
    NextColumn,
    /// Change the selected cell's value, by generating and running the
    /// statement that does it.
    EditCell,
    /// Delete the selected row, the same way.
    DeleteRow,
    /// Show/hide a table's columns in the schema sidebar.
    Expand,
    Collapse,
    // Lists
    MoveDown,
    MoveUp,
    MoveTop,
    MoveBottom,
    HalfPageDown,
    HalfPageUp,
    // Overlays
    Confirm,
    Cancel,
    /// Move between fields of a multi-field form.
    NextField,
    PrevField,
    /// Take the highlighted autocomplete suggestion.
    AcceptCompletion,
    NextCompletion,
    PrevCompletion,
}

impl Command {
    pub fn name(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::NewTab => "new-tab",
            Self::CloseTab => "close-tab",
            Self::NextTab => "next-tab",
            Self::PrevTab => "prev-tab",
            Self::Open => "open",
            Self::NewConnection => "new-connection",
            Self::EditConnection => "edit-connection",
            Self::DeleteConnection => "delete-connection",
            Self::Back => "back",
            Self::RunQuery => "run-query",
            Self::CancelQuery => "cancel-query",
            Self::RunAll => "run-all",
            Self::CycleFocus => "cycle-focus",
            Self::SaveFile => "save-file",
            Self::OpenFile => "open-file",
            Self::History => "history",
            Self::ExportCurl => "export-curl",
            Self::Yank => "yank",
            Self::InsertName => "insert-name",
            Self::Help => "help",
            Self::PrevColumn => "prev-column",
            Self::NextColumn => "next-column",
            Self::EditCell => "edit-cell",
            Self::DeleteRow => "delete-row",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::MoveDown => "move-down",
            Self::MoveUp => "move-up",
            Self::MoveTop => "move-top",
            Self::MoveBottom => "move-bottom",
            Self::HalfPageDown => "half-page-down",
            Self::HalfPageUp => "half-page-up",
            Self::Confirm => "confirm",
            Self::Cancel => "cancel",
            Self::NextField => "next-field",
            Self::PrevField => "prev-field",
            Self::AcceptCompletion => "accept-completion",
            Self::NextCompletion => "next-completion",
            Self::PrevCompletion => "prev-completion",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }

    const ALL: [Self; 40] = [
        Self::Quit,
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PrevTab,
        Self::Open,
        Self::NewConnection,
        Self::EditConnection,
        Self::DeleteConnection,
        Self::Back,
        Self::RunQuery,
        Self::CancelQuery,
        Self::RunAll,
        Self::CycleFocus,
        Self::SaveFile,
        Self::OpenFile,
        Self::History,
        Self::ExportCurl,
        Self::Yank,
        Self::InsertName,
        Self::Help,
        Self::PrevColumn,
        Self::NextColumn,
        Self::EditCell,
        Self::DeleteRow,
        Self::Expand,
        Self::Collapse,
        Self::MoveDown,
        Self::MoveUp,
        Self::MoveTop,
        Self::MoveBottom,
        Self::HalfPageDown,
        Self::HalfPageUp,
        Self::Confirm,
        Self::Cancel,
        Self::NextField,
        Self::PrevField,
        Self::AcceptCompletion,
        Self::NextCompletion,
        Self::PrevCompletion,
    ];

    /// One-line description, shown next to the binding in the help overlay.
    pub fn description(self) -> &'static str {
        match self {
            Self::Quit => "Quit tradar",
            Self::NewTab => "Open a new tab",
            Self::CloseTab => "Close the current tab",
            Self::NextTab => "Go to the next tab",
            Self::PrevTab => "Go to the previous tab",
            Self::Open => "Connect to the selected connection",
            Self::NewConnection => "Add a connection",
            Self::EditConnection => "Edit the selected connection",
            Self::DeleteConnection => "Delete the selected connection",
            Self::Back => "Back to the connection picker",
            Self::RunQuery => "Run the statement at the cursor",
            Self::CancelQuery => "Cancel the running query",
            Self::RunAll => "Run every statement in the buffer",
            Self::CycleFocus => "Cycle focus: editor / results / schema",
            Self::SaveFile => "Save the query to a file",
            Self::OpenFile => "Load a query from a file",
            Self::History => "Browse query history",
            Self::ExportCurl => "Export the request as curl (Elasticsearch)",
            Self::Yank => "Copy the selected row/document",
            Self::InsertName => "Insert the selected schema name",
            Self::Help => "Show this help",
            Self::PrevColumn => "Move to the previous column",
            Self::NextColumn => "Move to the next column",
            Self::EditCell => "Edit the selected cell",
            Self::DeleteRow => "Delete the selected row",
            Self::Expand => "Show a table's columns",
            Self::Collapse => "Hide a table's columns",
            Self::MoveDown => "Move down",
            Self::MoveUp => "Move up",
            Self::MoveTop => "Jump to the top",
            Self::MoveBottom => "Jump to the bottom",
            Self::HalfPageDown => "Scroll half a page down",
            Self::HalfPageUp => "Scroll half a page up",
            Self::Confirm => "Confirm",
            Self::Cancel => "Cancel",
            Self::NextField => "Next field",
            Self::PrevField => "Previous field",
            Self::AcceptCompletion => "Accept the suggestion",
            Self::NextCompletion => "Next suggestion",
            Self::PrevCompletion => "Previous suggestion",
        }
    }

    /// The list movement this command means, for the four components that
    /// render a selectable list. `None` for everything else.
    pub fn as_vim_move(self) -> Option<VimMove> {
        Some(match self {
            Self::MoveDown => VimMove::Down,
            Self::MoveUp => VimMove::Up,
            Self::MoveTop => VimMove::Top,
            Self::MoveBottom => VimMove::Bottom,
            Self::HalfPageDown => VimMove::HalfPageDown,
            Self::HalfPageUp => VimMove::HalfPageUp,
            _ => return None,
        })
    }
}

/// A single key press: a key plus its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyPress {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        // A capital letter arrives as `Char('J')` with SHIFT set on some
        // terminals and without it on others; the character itself already
        // carries the shift, so normalize it away rather than needing two
        // bindings for `G`.
        let modifiers = match code {
            KeyCode::Char(_) => modifiers.difference(KeyModifiers::SHIFT),
            _ => modifiers,
        };
        Self { code, modifiers }
    }

    /// Renders back to config syntax (`"ctrl-d"`, `"enter"`, `"j"`) for the
    /// help overlay.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            out.push_str("ctrl-");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            out.push_str("alt-");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            out.push_str("shift-");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("space"),
            KeyCode::Char(c) => out.push(c),
            KeyCode::F(n) => out.push_str(&format!("f{n}")),
            KeyCode::Enter => out.push_str("enter"),
            KeyCode::Esc => out.push_str("esc"),
            KeyCode::Tab => out.push_str("tab"),
            KeyCode::BackTab => out.push_str("backtab"),
            KeyCode::Backspace => out.push_str("backspace"),
            KeyCode::Delete => out.push_str("delete"),
            KeyCode::Insert => out.push_str("insert"),
            KeyCode::Home => out.push_str("home"),
            KeyCode::End => out.push_str("end"),
            KeyCode::PageUp => out.push_str("pageup"),
            KeyCode::PageDown => out.push_str("pagedown"),
            KeyCode::Up => out.push_str("up"),
            KeyCode::Down => out.push_str("down"),
            KeyCode::Left => out.push_str("left"),
            KeyCode::Right => out.push_str("right"),
            other => out.push_str(&format!("{other:?}").to_lowercase()),
        }
        out
    }
}

/// One binding: one key press, or two pressed in sequence (`gg`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding(Vec<KeyPress>);

impl Binding {
    pub fn display(&self) -> String {
        self.0.iter().map(|k| k.display()).collect::<String>()
    }
}

/// What a key press meant in a given context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Command(Command),
    /// The key started a two-key binding (the first `g` of `gg`); nothing
    /// has happened yet.
    Pending,
    /// Not bound here -- the caller should keep looking (a different
    /// context, or its own handling).
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    /// Per context, in lookup order. A `Vec` rather than a map because
    /// several bindings can share one command (`j` and `down`), and the
    /// help overlay wants them in a stable declared order.
    bindings: HashMap<Context, Vec<(Binding, Command)>>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(
            Context::Global,
            parse_defaults(&[
                ("ctrl-q", Command::Quit),
                ("ctrl-t", Command::NewTab),
                ("ctrl-w", Command::CloseTab),
                ("ctrl-right", Command::NextTab),
                ("ctrl-left", Command::PrevTab),
            ]),
        );
        bindings.insert(
            Context::Picker,
            parse_defaults(&[
                ("q", Command::Quit),
                ("enter", Command::Open),
                ("a", Command::NewConnection),
                ("e", Command::EditConnection),
                ("d", Command::DeleteConnection),
                ("?", Command::Help),
            ]),
        );
        bindings.insert(
            Context::QueryScreen,
            parse_defaults(&[
                ("esc", Command::Back),
                ("f5", Command::RunQuery),
                ("ctrl-enter", Command::RunQuery),
                ("ctrl-c", Command::CancelQuery),
                ("ctrl-a", Command::RunAll),
                ("tab", Command::CycleFocus),
                ("ctrl-s", Command::SaveFile),
                ("ctrl-o", Command::OpenFile),
                ("ctrl-r", Command::History),
                ("ctrl-y", Command::ExportCurl),
                ("?", Command::Help),
            ]),
        );
        bindings.insert(
            Context::Sidebar,
            parse_defaults(&[
                ("enter", Command::InsertName),
                ("l", Command::Expand),
                ("right", Command::Expand),
                ("h", Command::Collapse),
                ("left", Command::Collapse),
            ]),
        );
        bindings.insert(
            Context::Results,
            parse_defaults(&[
                ("y", Command::Yank),
                ("h", Command::PrevColumn),
                ("left", Command::PrevColumn),
                ("l", Command::NextColumn),
                ("right", Command::NextColumn),
                ("enter", Command::EditCell),
                ("d", Command::DeleteRow),
            ]),
        );
        bindings.insert(
            Context::List,
            parse_defaults(&[
                ("j", Command::MoveDown),
                ("down", Command::MoveDown),
                ("k", Command::MoveUp),
                ("up", Command::MoveUp),
                ("gg", Command::MoveTop),
                ("G", Command::MoveBottom),
                ("ctrl-d", Command::HalfPageDown),
                ("ctrl-u", Command::HalfPageUp),
            ]),
        );
        bindings.insert(
            Context::Completion,
            parse_defaults(&[
                ("tab", Command::AcceptCompletion),
                ("ctrl-n", Command::NextCompletion),
                ("down", Command::NextCompletion),
                ("ctrl-p", Command::PrevCompletion),
                ("up", Command::PrevCompletion),
            ]),
        );
        bindings.insert(
            Context::Prompt,
            parse_defaults(&[
                ("enter", Command::Confirm),
                ("esc", Command::Cancel),
                ("tab", Command::NextField),
                ("backtab", Command::PrevField),
            ]),
        );
        Self { bindings }
    }
}

fn parse_defaults(pairs: &[(&str, Command)]) -> Vec<(Binding, Command)> {
    pairs
        .iter()
        .map(|(keys, command)| {
            let binding = parse_binding(keys)
                .unwrap_or_else(|e| panic!("built-in binding '{keys}' must parse: {e}"));
            (binding, *command)
        })
        .collect()
}

impl Keymap {
    /// Resolves `key` in `context`, threading the caller's own `pending`
    /// slot for two-key bindings. Any key that doesn't complete a pending
    /// sequence cancels it and is then matched on its own, which is what
    /// vim does (`g` then `k` moves up).
    pub fn resolve(
        &self,
        context: Context,
        pending: &mut Option<KeyPress>,
        key: KeyPress,
    ) -> Resolution {
        self.resolve_in(&[context], pending, key)
    }

    /// Resolves against several contexts at once, earlier ones winning --
    /// how a screen checks its own bindings before falling back to the
    /// shared list navigation. Sharing one `pending` slot across them is
    /// the point: a two-key sequence started in one context still completes
    /// even if another context also binds its first key.
    pub fn resolve_in(
        &self,
        contexts: &[Context],
        pending: &mut Option<KeyPress>,
        key: KeyPress,
    ) -> Resolution {
        if let Some(previous) = pending.take() {
            for context in contexts {
                if let Some(command) = self.lookup(*context, &[previous, key]) {
                    return Resolution::Command(command);
                }
            }
        }
        for context in contexts {
            if let Some(command) = self.lookup(*context, &[key]) {
                return Resolution::Command(command);
            }
        }
        if contexts
            .iter()
            .any(|context| self.starts_a_sequence(*context, key))
        {
            *pending = Some(key);
            return Resolution::Pending;
        }
        Resolution::None
    }

    fn lookup(&self, context: Context, keys: &[KeyPress]) -> Option<Command> {
        self.bindings
            .get(&context)?
            .iter()
            .find(|(binding, _)| binding.0 == keys)
            .map(|(_, command)| *command)
    }

    fn starts_a_sequence(&self, context: Context, key: KeyPress) -> bool {
        self.bindings.get(&context).is_some_and(|bindings| {
            bindings
                .iter()
                .any(|(binding, _)| binding.0.len() > 1 && binding.0[0] == key)
        })
    }

    /// Every binding in `context`, in declaration order -- what the help
    /// overlay renders.
    pub fn bindings(&self, context: Context) -> &[(Binding, Command)] {
        self.bindings
            .get(&context)
            .map_or(&[], |bindings| bindings.as_slice())
    }

    /// The first binding for `command` in `context`, for inline hints in
    /// the status bar. `None` if the user unbound it.
    pub fn binding_for(&self, context: Context, command: Command) -> Option<String> {
        self.bindings
            .get(&context)?
            .iter()
            .find(|(_, c)| *c == command)
            .map(|(binding, _)| binding.display())
    }

    /// Replaces the bindings for the commands named in `overrides` (from
    /// `[keymap.<context>]` in `config.toml`). Only the commands mentioned
    /// change: everything else keeps its default, so a two-line config
    /// doesn't silently unbind the rest of the app. Binding a command to an
    /// empty list unbinds it.
    pub fn apply_overrides(
        &mut self,
        overrides: &HashMap<String, HashMap<String, Vec<String>>>,
    ) -> anyhow::Result<()> {
        for (context_name, commands) in overrides {
            let context = Context::from_name(context_name)
                .ok_or_else(|| anyhow::anyhow!("keymap.{context_name}: unknown context"))?;
            for (command_name, keys) in commands {
                let command = Command::from_name(command_name).ok_or_else(|| {
                    anyhow::anyhow!("keymap.{context_name}.{command_name}: unknown command")
                })?;
                let mut parsed = Vec::new();
                for key in keys {
                    let binding = parse_binding(key).map_err(|e| {
                        anyhow::anyhow!("keymap.{context_name}.{command_name}: {e}")
                    })?;
                    parsed.push((binding, command));
                }
                let bindings = self.bindings.entry(context).or_default();
                bindings.retain(|(_, c)| *c != command);
                bindings.extend(parsed);
            }
        }
        Ok(())
    }
}

/// Parses config syntax into a binding: `"ctrl-d"`, `"enter"`, `"j"`, or a
/// two-character sequence like `"gg"`.
fn parse_binding(spec: &str) -> anyhow::Result<Binding> {
    if spec.is_empty() {
        anyhow::bail!("empty key binding");
    }
    if let Ok(press) = parse_key_press(spec) {
        return Ok(Binding(vec![press]));
    }
    // Not a single key: the only other shape is a two-key sequence of plain
    // characters, written bare (`gg`). Anything longer -- or containing a
    // `-`, which means the author meant a modifier -- is a typo, and
    // reporting it beats silently binding a 13-key sequence.
    let chars: Vec<char> = spec.chars().collect();
    if chars.len() == 2 && !chars.contains(&'-') {
        let presses: Result<Vec<_>, _> = chars
            .iter()
            .map(|c| parse_key_press(&c.to_string()))
            .collect();
        return Ok(Binding(presses?));
    }
    anyhow::bail!("'{spec}' is not a known key")
}

fn parse_key_press(spec: &str) -> anyhow::Result<KeyPress> {
    let mut modifiers = KeyModifiers::NONE;
    let mut rest = spec;
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(stripped) = lower.strip_prefix("ctrl-") {
            modifiers |= KeyModifiers::CONTROL;
            rest = &rest[rest.len() - stripped.len()..];
        } else if let Some(stripped) = lower.strip_prefix("alt-") {
            modifiers |= KeyModifiers::ALT;
            rest = &rest[rest.len() - stripped.len()..];
        } else if let Some(stripped) = lower.strip_prefix("shift-") {
            modifiers |= KeyModifiers::SHIFT;
            rest = &rest[rest.len() - stripped.len()..];
        } else {
            break;
        }
    }

    let code = match rest.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        other => {
            if let Some(n) = other.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                KeyCode::F(n)
            } else {
                // Case matters for plain characters (`G` is not `g`), so
                // take it from `rest`, not the lowercased copy.
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => anyhow::bail!("'{spec}' is not a known key"),
                }
            }
        }
    };
    Ok(KeyPress::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyPress {
        KeyPress::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyPress {
        KeyPress::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn resolves_a_single_key_binding() {
        let keymap = Keymap::default();
        let mut pending = None;

        let resolution = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('j')));

        assert_eq!(resolution, Resolution::Command(Command::MoveDown));
    }

    #[test]
    fn the_same_key_means_different_things_in_different_contexts() {
        let keymap = Keymap::default();
        let mut pending = None;

        let in_picker = keymap.resolve(Context::Picker, &mut pending, press(KeyCode::Enter));
        let in_sidebar = keymap.resolve(Context::Sidebar, &mut pending, press(KeyCode::Enter));

        assert_eq!(in_picker, Resolution::Command(Command::Open));
        assert_eq!(in_sidebar, Resolution::Command(Command::InsertName));
    }

    #[test]
    fn a_two_key_sequence_needs_both_keys() {
        let keymap = Keymap::default();
        let mut pending = None;

        let first = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('g')));
        assert_eq!(first, Resolution::Pending);
        assert!(pending.is_some());

        let second = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('g')));
        assert_eq!(second, Resolution::Command(Command::MoveTop));
        assert!(pending.is_none(), "the sequence must be consumed");
    }

    #[test]
    fn a_key_that_does_not_complete_a_sequence_cancels_it_and_still_acts() {
        let keymap = Keymap::default();
        let mut pending = None;
        keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('g')));

        let resolution = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('k')));

        assert_eq!(
            resolution,
            Resolution::Command(Command::MoveUp),
            "vim behaviour: the second key still does its own job"
        );
        assert!(pending.is_none());
    }

    #[test]
    fn resolve_in_prefers_the_earlier_context() {
        let keymap = Keymap::default();
        let mut pending = None;

        // `enter` is bound in both, and Sidebar is listed first.
        let resolution = keymap.resolve_in(
            &[Context::Sidebar, Context::Picker],
            &mut pending,
            press(KeyCode::Enter),
        );

        assert_eq!(resolution, Resolution::Command(Command::InsertName));
    }

    #[test]
    fn resolve_in_falls_back_to_a_later_context() {
        let keymap = Keymap::default();
        let mut pending = None;

        // `j` isn't a query-screen binding, so the shared list one wins.
        let resolution = keymap.resolve_in(
            &[Context::QueryScreen, Context::List],
            &mut pending,
            press(KeyCode::Char('j')),
        );

        assert_eq!(resolution, Resolution::Command(Command::MoveDown));
    }

    #[test]
    fn the_same_key_does_different_things_in_the_sidebar_and_the_results_pane() {
        let keymap = Keymap::default();
        let mut pending = None;

        let in_sidebar = keymap.resolve(Context::Sidebar, &mut pending, press(KeyCode::Char('l')));
        let in_results = keymap.resolve(Context::Results, &mut pending, press(KeyCode::Char('l')));

        assert_eq!(in_sidebar, Resolution::Command(Command::Expand));
        assert_eq!(in_results, Resolution::Command(Command::NextColumn));
    }

    #[test]
    fn an_unbound_key_resolves_to_none() {
        let keymap = Keymap::default();
        let mut pending = None;

        let resolution = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('z')));

        assert_eq!(resolution, Resolution::None);
    }

    #[test]
    fn a_capital_letter_matches_with_or_without_the_shift_modifier() {
        let keymap = Keymap::default();
        let mut pending = None;

        let bare = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('G')));
        let shifted = keymap.resolve(
            Context::List,
            &mut pending,
            KeyPress::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
        );

        assert_eq!(bare, Resolution::Command(Command::MoveBottom));
        assert_eq!(shifted, Resolution::Command(Command::MoveBottom));
    }

    #[test]
    fn several_keys_can_share_one_command() {
        let keymap = Keymap::default();
        let mut pending = None;

        let letter = keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('j')));
        let arrow = keymap.resolve(Context::List, &mut pending, press(KeyCode::Down));

        assert_eq!(letter, Resolution::Command(Command::MoveDown));
        assert_eq!(arrow, Resolution::Command(Command::MoveDown));
    }

    fn overrides(
        context: &str,
        command: &str,
        keys: &[&str],
    ) -> HashMap<String, HashMap<String, Vec<String>>> {
        let mut commands = HashMap::new();
        commands.insert(
            command.to_string(),
            keys.iter().map(|k| k.to_string()).collect(),
        );
        let mut contexts = HashMap::new();
        contexts.insert(context.to_string(), commands);
        contexts
    }

    #[test]
    fn an_override_replaces_the_default_binding_for_that_command_only() {
        let mut keymap = Keymap::default();

        keymap
            .apply_overrides(&overrides("list", "move-down", &["n"]))
            .unwrap();

        let mut pending = None;
        assert_eq!(
            keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('n'))),
            Resolution::Command(Command::MoveDown)
        );
        assert_eq!(
            keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('j'))),
            Resolution::None,
            "the default binding is replaced, not added to"
        );
        assert_eq!(
            keymap.resolve(Context::List, &mut pending, press(KeyCode::Char('k'))),
            Resolution::Command(Command::MoveUp),
            "other commands keep their defaults"
        );
    }

    #[test]
    fn a_command_can_be_bound_to_an_empty_list_to_unbind_it() {
        let mut keymap = Keymap::default();

        keymap
            .apply_overrides(&overrides("global", "close-tab", &[]))
            .unwrap();

        let mut pending = None;
        assert_eq!(
            keymap.resolve(Context::Global, &mut pending, ctrl('w')),
            Resolution::None
        );
    }

    #[test]
    fn an_unknown_context_or_command_is_an_error() {
        let mut keymap = Keymap::default();

        let bad_context = keymap
            .apply_overrides(&overrides("lists", "move-down", &["n"]))
            .unwrap_err();
        let bad_command = keymap
            .apply_overrides(&overrides("list", "move-downwards", &["n"]))
            .unwrap_err();

        assert!(bad_context.to_string().contains("unknown context"));
        assert!(bad_command.to_string().contains("unknown command"));
    }

    #[test]
    fn an_unparseable_key_is_an_error_naming_the_command() {
        let mut keymap = Keymap::default();

        let err = keymap
            .apply_overrides(&overrides("list", "move-down", &["ctrl-nonsense"]))
            .unwrap_err();

        assert!(err.to_string().contains("keymap.list.move-down"), "{err}");
        assert!(err.to_string().contains("not a known key"), "{err}");
    }

    #[test]
    fn parses_modifiers_named_keys_function_keys_and_sequences() {
        assert_eq!(parse_binding("ctrl-d").unwrap(), Binding(vec![ctrl('d')]));
        assert_eq!(
            parse_binding("enter").unwrap(),
            Binding(vec![press(KeyCode::Enter)])
        );
        assert_eq!(
            parse_binding("f5").unwrap(),
            Binding(vec![press(KeyCode::F(5))])
        );
        assert_eq!(
            parse_binding("gg").unwrap(),
            Binding(vec![press(KeyCode::Char('g')), press(KeyCode::Char('g'))])
        );
        assert_eq!(
            parse_binding("ctrl-right").unwrap(),
            Binding(vec![KeyPress::new(KeyCode::Right, KeyModifiers::CONTROL)])
        );
    }

    #[test]
    fn a_typo_is_rejected_rather_than_read_as_a_long_key_sequence() {
        for spec in ["ctrl-nonsense", "entr", "ggg"] {
            assert!(
                parse_binding(spec).is_err(),
                "'{spec}' should not parse as a binding"
            );
        }
    }

    #[test]
    fn parsing_is_case_sensitive_for_plain_characters() {
        assert_eq!(
            parse_binding("G").unwrap(),
            Binding(vec![press(KeyCode::Char('G'))])
        );
        assert_ne!(parse_binding("G").unwrap(), parse_binding("g").unwrap());
    }

    #[test]
    fn display_round_trips_through_the_parser() {
        for spec in ["ctrl-d", "enter", "f5", "gg", "j", "G", "ctrl-right"] {
            assert_eq!(parse_binding(spec).unwrap().display(), spec);
        }
    }

    #[test]
    fn binding_for_returns_the_first_binding_of_a_command() {
        let keymap = Keymap::default();

        assert_eq!(
            keymap
                .binding_for(Context::List, Command::MoveDown)
                .as_deref(),
            Some("j")
        );
        assert_eq!(
            keymap
                .binding_for(Context::Global, Command::Quit)
                .as_deref(),
            Some("ctrl-q")
        );
    }

    #[test]
    fn every_command_has_a_default_binding_somewhere() {
        let keymap = Keymap::default();

        for command in Command::ALL {
            let bound = Context::all()
                .iter()
                .any(|ctx| keymap.binding_for(*ctx, command).is_some());
            assert!(bound, "{} has no default binding", command.name());
        }
    }
}
