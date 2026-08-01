//! ratatui views, widgets, and input handling. Renders whatever state
//! `app` hands it; contains no driver-specific or business logic.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{App, Screen};
use crate::drivers::QueryResult;

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::ConnectionPicker => draw_connection_picker(frame, app),
        Screen::Query => draw_query_screen(frame, app),
    }
}

fn draw_connection_picker(frame: &mut Frame, app: &App) {
    let items: Vec<ListItem> = app
        .connections
        .iter()
        .enumerate()
        .map(|(i, connection)| {
            let item = ListItem::new(connection.name.clone());
            if i == app.selected {
                item.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Connections"),
    );
    frame.render_widget(list, frame.area());
}

fn draw_query_screen(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(frame.area());

    let connection_name = app.active_connection.as_deref().unwrap_or("");
    let input = Paragraph::new(app.query_input.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Query — {connection_name}")),
    );
    frame.render_widget(input, chunks[0]);

    let body_text = if let Some(error) = &app.last_error {
        error.clone()
    } else if let Some(result) = &app.last_result {
        match result {
            QueryResult::Table { columns, rows } => {
                let header = columns.join(" | ");
                let rows = rows
                    .iter()
                    .map(|row| row.join(" | "))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{header}\n{rows}")
            }
            QueryResult::Documents(docs) => docs
                .iter()
                .map(|doc| serde_json::to_string_pretty(doc).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    } else {
        String::new()
    };
    let body = Paragraph::new(body_text)
        .block(Block::default().borders(Borders::ALL).title("Results"));
    frame.render_widget(body, chunks[1]);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;
    use crate::app::App;
    use crate::storage::{DriverKind, SavedConnection};

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn connection_picker_lists_saved_connection_names() {
        let app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_active_connection_and_input() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.push_char('x');
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_the_last_result() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_result(crate::drivers::QueryResult::Table {
            columns: vec!["id".to_string()],
            rows: vec![vec!["42".to_string()]],
        });
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("42"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_documents_pretty_printed() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_result(crate::drivers::QueryResult::Documents(vec![
            serde_json::json!({"name": "Ada"}),
        ]));
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Ada"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_the_last_error() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_error("syntax error".to_string());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("syntax error"), "buffer was: {text}");
    }
}
