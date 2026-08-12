//! A single-line text input overlay used to ask for a file path, e.g. for
//! `Ctrl+S`/`Ctrl+O` in `QueryScreenComponent`. Not a `Component` -- it's
//! driven by `QueryScreenComponent::handle_key_event` directly, the same way
//! `QueryEditorComponent`/`ResultsComponent`/`SchemaSidebarComponent` are.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Save,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOutcome {
    Confirmed(String),
    Cancelled,
}

pub struct FilePromptComponent {
    pub kind: PromptKind,
    input: Vec<char>,
    cursor: usize,
    pub error: Option<String>,
}

impl FilePromptComponent {
    pub fn new(kind: PromptKind, initial: &str) -> Self {
        let input: Vec<char> = initial.chars().collect();
        let cursor = input.len();
        Self {
            kind,
            input,
            cursor,
            error: None,
        }
    }

    pub fn text(&self) -> String {
        self.input.iter().collect()
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<PromptOutcome> {
        match code {
            KeyCode::Esc => Some(PromptOutcome::Cancelled),
            KeyCode::Enter => Some(PromptOutcome::Confirmed(self.text())),
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                self.error = None;
                None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                    self.error = None;
                }
                None
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                    self.error = None;
                }
                None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.input.len());
                None
            }
            KeyCode::Home => {
                self.cursor = 0;
                None
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                None
            }
            _ => None,
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let title = match self.kind {
            PromptKind::Save => "Save query to file (Enter=save, Esc=cancel)",
            PromptKind::Open => "Open query from file (Enter=open, Esc=cancel)",
        };
        let border_color = if self.error.is_some() {
            Color::Red
        } else {
            Color::Yellow
        };
        let text = match &self.error {
            Some(err) => format!("{}\n{err}", self.text()),
            None => self.text(),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color));
        frame.render_widget(Paragraph::new(text).block(block), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_appends_to_the_initial_text() {
        let mut prompt = FilePromptComponent::new(PromptKind::Save, "foo");

        prompt.handle_key_event(KeyCode::Char('.'), KeyModifiers::NONE);
        prompt.handle_key_event(KeyCode::Char('s'), KeyModifiers::NONE);
        prompt.handle_key_event(KeyCode::Char('q'), KeyModifiers::NONE);
        prompt.handle_key_event(KeyCode::Char('l'), KeyModifiers::NONE);

        assert_eq!(prompt.text(), "foo.sql");
    }

    #[test]
    fn backspace_removes_before_the_cursor() {
        let mut prompt = FilePromptComponent::new(PromptKind::Save, "foo");

        prompt.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(prompt.text(), "fo");
    }

    #[test]
    fn left_then_backspace_removes_from_the_middle() {
        let mut prompt = FilePromptComponent::new(PromptKind::Save, "foo");

        prompt.handle_key_event(KeyCode::Left, KeyModifiers::NONE);
        prompt.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(prompt.text(), "fo");
    }

    #[test]
    fn enter_confirms_with_the_current_text() {
        let mut prompt = FilePromptComponent::new(PromptKind::Open, "foo.sql");

        let outcome = prompt.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(PromptOutcome::Confirmed("foo.sql".to_string()))
        );
    }

    #[test]
    fn esc_cancels() {
        let mut prompt = FilePromptComponent::new(PromptKind::Open, "foo.sql");

        let outcome = prompt.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(outcome, Some(PromptOutcome::Cancelled));
    }

    #[test]
    fn typing_clears_a_previous_error() {
        let mut prompt = FilePromptComponent::new(PromptKind::Save, "foo");
        prompt.error = Some("boom".to_string());

        prompt.handle_key_event(KeyCode::Char('x'), KeyModifiers::NONE);

        assert_eq!(prompt.error, None);
    }
}
