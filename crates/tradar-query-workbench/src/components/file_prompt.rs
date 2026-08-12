//! A single-line text input overlay used to ask for a file path, e.g. for
//! `Ctrl+S`/`Ctrl+O` in `QueryScreenComponent`. Not a `Component` -- it's
//! driven by `QueryScreenComponent::handle_key_event` directly, the same way
//! `QueryEditorComponent`/`ResultsComponent`/`SchemaSidebarComponent` are.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui;

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
        // Confirm/cancel come from the keymap; everything else is text
        // editing, which is fixed (this is a path field, not a vim buffer).
        let key = KeyPress::new(code, modifiers);
        let mut pending = None;
        if let Resolution::Command(command) = keymap().resolve(Context::Prompt, &mut pending, key) {
            match command {
                Command::Cancel => return Some(PromptOutcome::Cancelled),
                Command::Confirm => return Some(PromptOutcome::Confirmed(self.text())),
                _ => {}
            }
        }

        match code {
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
        let theme = theme();
        let confirm = keymap()
            .binding_for(Context::Prompt, Command::Confirm)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Prompt, Command::Cancel)
            .unwrap_or_default();
        let verb = match self.kind {
            PromptKind::Save => "Save query to",
            PromptKind::Open => "Open query from",
        };
        let title = format!("{verb} — {confirm} confirm, {cancel} cancel");

        // The typed path, with a block cursor so the caret is visible in a
        // terminal that hides the real one.
        let input: Vec<char> = self.input.clone();
        let mut spans: Vec<Span> = input
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let style = Style::default().fg(theme.text);
                if i == self.cursor {
                    Span::styled(c.to_string(), style.add_modifier(Modifier::REVERSED))
                } else {
                    Span::styled(c.to_string(), style)
                }
            })
            .collect();
        if self.cursor >= input.len() {
            spans.push(Span::styled(
                " ",
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }

        let mut lines = vec![Line::from(spans)];
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                error.clone(),
                Style::default().fg(theme.error),
            )));
        }

        let mut block = ui::panel(&title, true);
        if self.error.is_some() {
            block = block.border_style(Style::default().fg(theme.error));
        }
        frame.render_widget(Paragraph::new(lines).block(block), area);
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
