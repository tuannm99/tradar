//! The application-level `Action` message and the `Component` trait every
//! `Screen` implements. This enum is closed and stays closed: no connector
//! adds a variant to it. Everything specific to one connector's own key
//! handling/state travels as a direct synchronous method call on that
//! connector's `Session`, never through this type -- see "Screen không bao
//! giờ làm IO" in docs/architecture.md.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::storage::SavedConnection;

pub enum Action {
    Quit,
    OpenRequested {
        connection: SavedConnection,
        epoch: u64,
    },
    Opened {
        connection: SavedConnection,
        screen: Box<dyn Component>,
        epoch: u64,
    },
    OpenFailed {
        error: String,
        epoch: u64,
    },
    BackToPicker,
}

pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn tick(&mut self) {}
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
