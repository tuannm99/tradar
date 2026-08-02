//! ratatui views, widgets, and input handling. Renders whatever state
//! `app` hands it; contains no driver-specific or business logic.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Focus, Screen};
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

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Connections"));

    let Some(error) = &app.last_error else {
        frame.render_widget(list, frame.area());
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(frame.area());
    frame.render_widget(list, chunks[0]);

    let error_box =
        Paragraph::new(error.as_str()).block(Block::default().borders(Borders::ALL).title("Error"));
    frame.render_widget(error_box, chunks[1]);
}

fn draw_query_screen(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(1)])
        .split(frame.area());

    draw_schema_sidebar(frame, app, outer[0]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(outer[1]);

    let connection_name = app
        .active_connection
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("");
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
    let body =
        Paragraph::new(body_text).block(Block::default().borders(Borders::ALL).title("Results"));
    frame.render_widget(body, chunks[1]);
}

fn draw_schema_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .schema
        .iter()
        .map(|entry| ListItem::new(entry.name.clone()))
        .collect();

    let mut state = ListState::default();
    if !app.schema.is_empty() {
        state.select(Some(app.schema_selected));
    }

    let title = if app.focus == Focus::Sidebar {
        "Schema [focused]"
    } else {
        "Schema"
    };

    let Some(error) = &app.schema_error else {
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, area, &mut state);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(7)])
        .split(area);

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let error_box = Paragraph::new(error.as_str())
        .block(Block::default().borders(Borders::ALL).title("Error"))
        .wrap(Wrap { trim: true });
    frame.render_widget(error_box, chunks[1]);
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
        let backend = TestBackend::new(64, 10);
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
        let backend = TestBackend::new(64, 10);
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
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Ada"), "buffer was: {text}");
    }

    #[test]
    fn connection_picker_shows_a_connection_error() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.set_error("connection refused".to_string());
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("connection refused"), "buffer was: {text}");
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
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("syntax error"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_schema_items_in_the_sidebar() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_schema(vec![crate::drivers::SchemaInfo {
            name: "users".to_string(),
        }]);
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("users"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_marks_the_sidebar_as_focused_in_its_title() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.toggle_focus();
        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Schema [focused]"), "buffer was: {text}");
    }

    #[test]
    fn query_screen_shows_a_schema_error_in_the_sidebar() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        let message =
            "failed to run SCAN against redis at 10.0.0.5:6379: connection timed out after 5s"
                .to_string();
        app.set_schema_error(message.clone());
        let term_height = 10u16;
        let backend = TestBackend::new(64, term_height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        // The error box is a Length(7) region at the bottom of the 24-wide
        // sidebar; its inner (post-border) text area is 22 columns wide and
        // 5 rows tall. Extract just that region, word-wrapped across rows,
        // to confirm the whole message survived wrapping instead of being
        // clipped to a single line.
        let inner_error_area = Rect::new(1, term_height - 7 + 1, 22, 5);
        let wrapped = sidebar_text_in(terminal.backend().buffer(), inner_error_area);
        assert_eq!(wrapped, message, "buffer region was: {wrapped:?}");
    }

    /// Extracts text from a rectangular region of the buffer, treating each
    /// row as independently word-wrapped: trims each row and rejoins
    /// non-empty rows with a single space, reconstructing the original
    /// (unwrapped) sentence.
    fn sidebar_text_in(buffer: &Buffer, region: Rect) -> String {
        let mut rows = Vec::new();
        for y in region.y..region.y + region.height {
            let mut row = String::new();
            for x in region.x..region.x + region.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    row.push_str(cell.symbol());
                }
            }
            let trimmed = row.trim().to_string();
            if !trimmed.is_empty() {
                rows.push(trimmed);
            }
        }
        rows.join(" ")
    }

    #[test]
    fn schema_sidebar_selection_highlight_tracks_schema_selected() {
        let mut app = App::new(vec![SavedConnection {
            name: "local-sqlite".to_string(),
            driver: DriverKind::Sqlite,
            target: "test.db".to_string(),
        }]);
        app.connect_to_selected();
        app.set_schema(vec![
            crate::drivers::SchemaInfo {
                name: "aaa".to_string(),
            },
            crate::drivers::SchemaInfo {
                name: "bbb".to_string(),
            },
            crate::drivers::SchemaInfo {
                name: "ccc".to_string(),
            },
        ]);
        app.schema_move_down();
        assert_eq!(app.schema_selected, 1);

        let backend = TestBackend::new(64, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        // Sidebar area starts at (0, 0); the top border occupies row 0, so
        // list item 0 ("aaa") renders at row 1 and item 1 ("bbb") at row 2.
        // Column 1 is the first text column inside the left border.
        let unselected_cell = buffer.cell((1, 1)).unwrap();
        let selected_cell = buffer.cell((1, 2)).unwrap();

        assert!(
            selected_cell.modifier.contains(Modifier::REVERSED),
            "expected the selected row (schema_selected = 1) to carry the REVERSED modifier"
        );
        assert!(
            !unselected_cell.modifier.contains(Modifier::REVERSED),
            "expected an unselected row to not carry the REVERSED modifier"
        );
    }
}
