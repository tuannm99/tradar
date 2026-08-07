use crossterm::event::KeyEvent;
use edtui::actions::InsertChar;
use edtui::{EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Lines};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

pub struct QueryEditorComponent {
    pub state: EditorState,
    event_handler: EditorEventHandler,
}

impl Default for QueryEditorComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryEditorComponent {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(Lines::default()),
            event_handler: EditorEventHandler::default(),
        }
    }

    pub fn text(&self) -> String {
        self.state
            .lines
            .iter_row()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\n")
    }

    pub fn insert_at_cursor(&mut self, text: &str) {
        for c in text.chars() {
            self.state.execute(InsertChar(c));
        }
        self.state.mode = EditorMode::Insert;
    }

    pub fn forward_key(&mut self, key: KeyEvent) {
        self.event_handler.on_key_event(key, &mut self.state);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Query — {connection_name}"));
        let view = EditorView::new(&mut self.state)
            .theme(EditorTheme::default().block(block))
            .wrap(true);
        frame.render_widget(view, area);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_in_insert_mode_updates_the_text() {
        let mut editor = QueryEditorComponent::new();

        editor.forward_key(key(KeyCode::Char('i')));
        editor.forward_key(key(KeyCode::Char('a')));
        editor.forward_key(key(KeyCode::Char('b')));

        assert_eq!(editor.text(), "ab");
    }

    #[test]
    fn backspace_in_insert_mode_removes_the_last_character() {
        let mut editor = QueryEditorComponent::new();
        editor.forward_key(key(KeyCode::Char('i')));
        editor.forward_key(key(KeyCode::Char('a')));

        editor.forward_key(key(KeyCode::Backspace));

        assert_eq!(editor.text(), "");
    }

    #[test]
    fn insert_at_cursor_inserts_text_and_switches_to_insert_mode() {
        let mut editor = QueryEditorComponent::new();

        editor.insert_at_cursor("users");

        assert_eq!(editor.text(), "users");
        assert_eq!(editor.state.mode, EditorMode::Insert);
    }

    #[test]
    fn draw_shows_the_connection_name_and_input() {
        let mut editor = QueryEditorComponent::new();
        editor.insert_at_cursor("x");
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| editor.draw(frame, Rect::new(0, 0, 40, 10), "local-sqlite"))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("local-sqlite"), "buffer was: {text}");
        assert!(text.contains('x'), "buffer was: {text}");
    }
}
