//! The `Action` message type and the `Component` trait every top-level
//! screen implements. Depends only on the `Driver` trait's associated
//! types and `QueryEngine` — never a concrete driver module, so this
//! stays safe for every `components/` file to depend on.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::drivers::{QueryResult, SchemaInfo};
use crate::query_engine::QueryEngine;
use crate::storage::SavedConnection;

pub enum Action {
    Quit,
    ConnectRequested {
        connection: SavedConnection,
        epoch: u64,
    },
    Connected {
        connection: SavedConnection,
        engine: QueryEngine,
        schema: Result<Vec<SchemaInfo>, String>,
        epoch: u64,
    },
    ConnectFailed {
        error: String,
        epoch: u64,
    },
    ToggleFocus,
    SchemaMoveUp,
    SchemaMoveDown,
    SchemaMoveTop,
    SchemaMoveBottom,
    SchemaHalfPageUp,
    SchemaHalfPageDown,
    InsertSchemaSelection,
    SubmitQuery,
    QueryCompleted {
        engine: QueryEngine,
        result: QueryResult,
        epoch: u64,
    },
    QueryFailed {
        engine: QueryEngine,
        error: String,
        epoch: u64,
    },
    ExportCurl {
        connection: SavedConnection,
        query: String,
    },
    BackToPicker,
}

pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
