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
    /// `Ctrl+E`: export the current result to a CSV/JSON file. Unlike
    /// `Save`/`Open`, a bare name is a literal relative path -- there's no
    /// queries-directory concept for export output.
    Export,
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

    /// `queries_dir`: `Some` only for `Save`/`Open` when a queries
    /// directory is actually configured -- lets the prompt show exactly
    /// where the typed name resolves to, updated on every keystroke. `None`
    /// for `Export` (there's no queries-directory concept for it, see
    /// `PromptKind::Export`'s own doc comment) or when none is configured
    /// (tests, mainly) -- the preview line just doesn't appear then.
    pub fn draw(&self, frame: &mut Frame, area: Rect, queries_dir: Option<&std::path::Path>) {
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
            PromptKind::Export => "Export result to (.csv/.json)",
        };
        let title = format!("{verb} — {confirm} confirm, {cancel} cancel");

        let spans = self.input.spans(true);

        let mut lines = vec![Line::from(spans)];
        if let (PromptKind::Save | PromptKind::Open, Some(dir)) = (self.kind, queries_dir) {
            let resolved = tradar_core::storage::resolve_query_path(&self.text(), dir);
            lines.push(Line::from(Span::styled(
                format!("→ {}", resolved.display()),
                Style::default().fg(theme.text_dim),
            )));
        }
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn draw_text(prompt: &FilePromptComponent, queries_dir: Option<&std::path::Path>) -> String {
        let backend = TestBackend::new(60, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| prompt.draw(frame, frame.area(), queries_dir))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn the_preview_line_resolves_a_bare_name_into_the_queries_directory() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = FilePromptComponent::new(PromptKind::Save, "report");

        let text = draw_text(&prompt, Some(dir.path()));

        let expected = dir.path().join("report.sql");
        assert!(
            text.contains(&expected.display().to_string()),
            "buffer was: {text}"
        );
    }

    #[test]
    fn the_preview_line_joins_a_relative_subfolder_into_the_queries_directory() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = FilePromptComponent::new(PromptKind::Open, "sub/report.sql");

        let text = draw_text(&prompt, Some(dir.path()));

        let expected = dir.path().join("sub/report.sql");
        assert!(
            text.contains(&expected.display().to_string()),
            "buffer was: {text}"
        );
    }

    #[test]
    fn the_preview_line_uses_an_absolute_path_as_typed() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = FilePromptComponent::new(PromptKind::Open, "/tmp/elsewhere.sql");

        let text = draw_text(&prompt, Some(dir.path()));

        assert!(text.contains("/tmp/elsewhere.sql"), "buffer was: {text}");
    }

    #[test]
    fn the_preview_line_is_absent_without_a_queries_directory() {
        let prompt = FilePromptComponent::new(PromptKind::Save, "report");

        let text = draw_text(&prompt, None);

        assert!(!text.contains("report.sql"), "buffer was: {text}");
    }

    #[test]
    fn export_never_shows_a_preview_line() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = FilePromptComponent::new(PromptKind::Export, "out");

        let text = draw_text(&prompt, Some(dir.path()));

        assert!(!text.contains("out.sql"), "buffer was: {text}");
    }

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
