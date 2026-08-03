//! The query input box. Not a `Component` — driven entirely by
//! `QueryScreenComponent`. `query_input` will change representation
//! (to a vim-modal `edtui` editor) in a later sub-project; this
//! sub-project keeps it an unmodified `String`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Default)]
pub struct QueryEditorComponent {
    pub query_input: String,
}

impl QueryEditorComponent {
    pub fn new() -> Self {
        Self {
            query_input: String::new(),
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.query_input.push(c);
    }

    pub fn backspace(&mut self) {
        self.query_input.pop();
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, connection_name: &str) {
        let input = Paragraph::new(self.query_input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Query — {connection_name}")),
        );
        frame.render_widget(input, area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn push_char_and_backspace_edit_the_query_input() {
        let mut editor = QueryEditorComponent::new();

        editor.push_char('a');
        editor.push_char('b');
        assert_eq!(editor.query_input, "ab");

        editor.backspace();
        assert_eq!(editor.query_input, "a");
    }

    #[test]
    fn backspace_on_empty_input_does_nothing() {
        let mut editor = QueryEditorComponent::new();

        editor.backspace();

        assert_eq!(editor.query_input, "");
    }

    #[test]
    fn draw_shows_the_connection_name_and_input() {
        let mut editor = QueryEditorComponent::new();
        editor.push_char('x');
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
