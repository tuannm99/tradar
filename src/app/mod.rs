//! Application state, event loop, and command dispatch. Depends only on
//! the `Driver` trait, never on a specific driver implementation.

use crate::drivers::{QueryResult, SchemaInfo};
use crate::storage::SavedConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    ConnectionPicker,
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Sidebar,
}

pub struct App {
    pub screen: Screen,
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub active_connection: Option<SavedConnection>,
    pub query_input: String,
    pub should_quit: bool,
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub schema: Vec<SchemaInfo>,
    pub schema_selected: usize,
    pub schema_error: Option<String>,
    pub focus: Focus,
}

impl App {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            screen: Screen::ConnectionPicker,
            connections,
            selected: 0,
            active_connection: None,
            query_input: String::new(),
            should_quit: false,
            last_result: None,
            last_error: None,
            schema: Vec::new(),
            schema_selected: 0,
            schema_error: None,
            focus: Focus::Editor,
        }
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.last_result = Some(result);
        self.last_error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.last_result = None;
    }

    pub fn push_char(&mut self, c: char) {
        self.query_input.push(c);
    }

    pub fn backspace(&mut self) {
        self.query_input.pop();
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn move_selection_down(&mut self) {
        if self.selected + 1 < self.connections.len() {
            self.selected += 1;
        }
    }

    pub fn move_selection_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn set_schema(&mut self, schema: Vec<SchemaInfo>) {
        self.schema = schema;
        self.schema_selected = 0;
        self.schema_error = None;
    }

    pub fn set_schema_error(&mut self, error: String) {
        self.schema_error = Some(error);
    }

    pub fn schema_move_down(&mut self) {
        if self.schema_selected + 1 < self.schema.len() {
            self.schema_selected += 1;
        }
    }

    pub fn schema_move_up(&mut self) {
        self.schema_selected = self.schema_selected.saturating_sub(1);
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Editor => Focus::Sidebar,
            Focus::Sidebar => Focus::Editor,
        };
    }

    pub fn insert_schema_selection(&mut self) {
        let Some(item) = self.schema.get(self.schema_selected) else {
            return;
        };
        self.query_input.push_str(&item.name);
        self.focus = Focus::Editor;
    }

    pub fn connect_to_selected(&mut self) {
        self.active_connection = self.connections.get(self.selected).cloned();
        self.screen = Screen::Query;
    }

    pub fn back_to_picker(&mut self) {
        self.active_connection = None;
        self.screen = Screen::ConnectionPicker;
        self.schema = Vec::new();
        self.schema_selected = 0;
        self.schema_error = None;
        self.focus = Focus::Editor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{DriverKind, SavedConnection};

    fn connections() -> Vec<SavedConnection> {
        vec![
            SavedConnection {
                name: "local-sqlite".to_string(),
                driver: DriverKind::Sqlite,
                target: "test.db".to_string(),
            },
            SavedConnection {
                name: "local-postgres".to_string(),
                driver: DriverKind::Postgres,
                target: "postgres://localhost/test".to_string(),
            },
        ]
    }

    fn schema() -> Vec<SchemaInfo> {
        vec![
            SchemaInfo {
                name: "users".to_string(),
            },
            SchemaInfo {
                name: "orders".to_string(),
            },
        ]
    }

    #[test]
    fn new_app_starts_on_the_connection_picker_with_nothing_selected() {
        let app = App::new(connections());

        assert_eq!(app.screen, Screen::ConnectionPicker);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn move_selection_down_advances_and_stops_at_the_last_connection() {
        let mut app = App::new(connections());

        app.move_selection_down();
        assert_eq!(app.selected, 1);

        app.move_selection_down();
        assert_eq!(
            app.selected, 1,
            "should stop at the last connection, not wrap"
        );
    }

    #[test]
    fn move_selection_up_retreats_and_stops_at_zero() {
        let mut app = App::new(connections());
        app.move_selection_down();

        app.move_selection_up();
        assert_eq!(app.selected, 0);

        app.move_selection_up();
        assert_eq!(app.selected, 0, "should stop at zero, not go negative");
    }

    #[test]
    fn connect_to_selected_switches_to_the_query_screen() {
        let mut app = App::new(connections());
        app.move_selection_down();

        app.connect_to_selected();

        assert_eq!(app.screen, Screen::Query);
        assert_eq!(
            app.active_connection.as_ref().map(|c| c.name.as_str()),
            Some("local-postgres")
        );
    }

    #[test]
    fn back_to_picker_returns_to_the_connection_picker() {
        let mut app = App::new(connections());
        app.connect_to_selected();

        app.back_to_picker();

        assert_eq!(app.screen, Screen::ConnectionPicker);
        assert_eq!(app.active_connection, None);
    }

    #[test]
    fn push_char_and_backspace_edit_the_query_input() {
        let mut app = App::new(connections());

        app.push_char('a');
        app.push_char('b');
        assert_eq!(app.query_input, "ab");

        app.backspace();
        assert_eq!(app.query_input, "a");
    }

    #[test]
    fn backspace_on_empty_input_does_nothing() {
        let mut app = App::new(connections());

        app.backspace();

        assert_eq!(app.query_input, "");
    }

    #[test]
    fn quit_sets_should_quit() {
        let mut app = App::new(connections());

        assert!(!app.should_quit);
        app.quit();

        assert!(app.should_quit);
    }

    #[test]
    fn set_result_replaces_any_previous_error() {
        let mut app = App::new(connections());
        app.set_error("boom".to_string());

        app.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
        });

        assert!(app.last_error.is_none());
        assert_eq!(
            app.last_result,
            Some(QueryResult::Table {
                columns: vec!["id".to_string()],
                rows: vec![vec!["1".to_string()]],
            })
        );
    }

    #[test]
    fn set_result_preserves_the_query_input() {
        let mut app = App::new(connections());
        app.push_char('x');

        app.set_result(QueryResult::Table {
            columns: vec![],
            rows: vec![],
        });

        // The editor is multi-line and Ctrl+Y exports the current
        // query_input as curl, so a successful query must not wipe it.
        assert_eq!(app.query_input, "x");
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut app = App::new(connections());
        app.set_result(QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![],
        });

        app.set_error("boom".to_string());

        assert!(app.last_result.is_none());
        assert_eq!(app.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn set_error_keeps_the_query_input_so_it_can_be_fixed() {
        let mut app = App::new(connections());
        app.push_char('x');

        app.set_error("boom".to_string());

        assert_eq!(app.query_input, "x");
    }

    #[test]
    fn set_schema_replaces_the_schema_and_resets_selection_and_error() {
        let mut app = App::new(connections());
        app.set_schema_error("boom".to_string());
        app.schema_selected = 1;

        app.set_schema(schema());

        assert_eq!(app.schema, schema());
        assert_eq!(app.schema_selected, 0);
        assert!(app.schema_error.is_none());
    }

    #[test]
    fn schema_move_down_advances_and_stops_at_the_last_item() {
        let mut app = App::new(connections());
        app.set_schema(schema());

        app.schema_move_down();
        assert_eq!(app.schema_selected, 1);

        app.schema_move_down();
        assert_eq!(
            app.schema_selected, 1,
            "should stop at the last item, not wrap"
        );
    }

    #[test]
    fn schema_move_up_retreats_and_stops_at_zero() {
        let mut app = App::new(connections());
        app.set_schema(schema());
        app.schema_move_down();

        app.schema_move_up();
        assert_eq!(app.schema_selected, 0);

        app.schema_move_up();
        assert_eq!(
            app.schema_selected, 0,
            "should stop at zero, not go negative"
        );
    }

    #[test]
    fn toggle_focus_flips_between_editor_and_sidebar() {
        let mut app = App::new(connections());
        assert_eq!(app.focus, Focus::Editor);

        app.toggle_focus();
        assert_eq!(app.focus, Focus::Sidebar);

        app.toggle_focus();
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn insert_schema_selection_appends_the_selected_name_and_returns_focus_to_editor() {
        let mut app = App::new(connections());
        app.set_schema(schema());
        app.schema_move_down();
        app.toggle_focus();
        app.push_char('x');

        app.insert_schema_selection();

        assert_eq!(app.query_input, "xorders");
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn insert_schema_selection_is_a_no_op_when_schema_is_empty() {
        let mut app = App::new(connections());
        app.toggle_focus();

        app.insert_schema_selection();

        assert_eq!(app.query_input, "");
        assert_eq!(
            app.focus,
            Focus::Sidebar,
            "no-op must not change focus either"
        );
    }

    #[test]
    fn back_to_picker_clears_schema_state() {
        let mut app = App::new(connections());
        app.connect_to_selected();
        app.set_schema(schema());
        app.set_schema_error("boom".to_string());
        app.toggle_focus();

        app.back_to_picker();

        assert_eq!(app.schema, Vec::new());
        assert_eq!(app.schema_selected, 0);
        assert!(app.schema_error.is_none());
        assert_eq!(app.focus, Focus::Editor);
    }
}
