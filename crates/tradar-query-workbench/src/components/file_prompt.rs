//! A single-line text input overlay used to ask for a file path, e.g. for
//! `Ctrl+S`/`Ctrl+O` in `QueryScreenComponent`. Not a `Component` -- it's
//! driven by `QueryScreenComponent::handle_key_event` directly, the same way
//! `QueryEditorComponent`/`ResultsComponent`/`SchemaSidebarComponent` are.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};

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
    input: TextInput,
    pub error: Option<String>,
}

impl FilePromptComponent {
    pub fn new(kind: PromptKind, initial: &str) -> Self {
        Self {
            kind,
            input: TextInput::new(initial),
            error: None,
        }
    }

    pub fn text(&self) -> String {
        self.input.text()
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

        // Anything the field itself handles is text editing; typing
        // clears a stale error so the prompt stops shouting at you while
        // you fix the path.
        if self.input.handle_key_event(code, modifiers) {
            self.error = None;
        }
        None
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

        let spans = self.input.spans(true);

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
