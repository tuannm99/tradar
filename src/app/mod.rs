//! Application state, event loop, and command dispatch. Depends only on
//! the `Driver` trait, never on a specific driver implementation.

use crate::drivers::QueryResult;
use crate::storage::SavedConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    ConnectionPicker,
    Query,
}

pub struct App {
    pub screen: Screen,
    pub connections: Vec<SavedConnection>,
    pub selected: usize,
    pub active_connection: Option<String>,
    pub query_input: String,
    pub should_quit: bool,
    pub last_result: Option<QueryResult>,
    pub last_error: Option<String>,
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
        }
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.last_result = Some(result);
        self.last_error = None;
        self.query_input.clear();
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

    pub fn connect_to_selected(&mut self) {
        self.active_connection = self.connections.get(self.selected).map(|c| c.name.clone());
        self.screen = Screen::Query;
    }

    pub fn back_to_picker(&mut self) {
        self.active_connection = None;
        self.screen = Screen::ConnectionPicker;
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
        assert_eq!(app.selected, 1, "should stop at the last connection, not wrap");
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
        assert_eq!(app.active_connection.as_deref(), Some("local-postgres"));
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

        app.set_result(QueryResult {
            columns: vec!["id".to_string()],
            rows: vec![vec!["1".to_string()]],
        });

        assert!(app.last_error.is_none());
        assert_eq!(app.last_result.as_ref().unwrap().columns, vec!["id"]);
    }

    #[test]
    fn set_result_clears_the_query_input() {
        let mut app = App::new(connections());
        app.push_char('x');

        app.set_result(QueryResult {
            columns: vec![],
            rows: vec![],
        });

        assert_eq!(app.query_input, "");
    }

    #[test]
    fn set_error_replaces_any_previous_result() {
        let mut app = App::new(connections());
        app.set_result(QueryResult {
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
}
