//! The add/edit-connection form, an overlay over the connection picker.
//! Before this, the only way to add a connection was to hand-edit
//! `connections.toml` and restart.
//!
//! Not a `Component`: like the query screen's file prompt, it's driven
//! directly by the screen hosting it (`ConnectionPickerComponent`), which
//! owns whether it's open.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedConnection;
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMode {
    Add,
    /// Editing the connection at this index in the picker's list.
    Edit(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormOutcome {
    Saved {
        connection: SavedConnection,
        mode: FormMode,
    },
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Driver,
    Target,
}

impl Field {
    const ORDER: [Self; 3] = [Self::Name, Self::Driver, Self::Target];

    fn next(self) -> Self {
        match self {
            Self::Name => Self::Driver,
            Self::Driver => Self::Target,
            Self::Target => Self::Name,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Name => Self::Target,
            Self::Driver => Self::Name,
            Self::Target => Self::Driver,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Driver => "Driver",
            Self::Target => "Target",
        }
    }
}

pub struct ConnectionFormComponent {
    pub mode: FormMode,
    field: Field,
    name: TextInput,
    target: TextInput,
    /// Every connector id compiled into this build, and which one is
    /// selected. A picker rather than a text field: a typo'd driver id only
    /// fails later, at connect time, with "unknown connector".
    drivers: Vec<String>,
    driver: usize,
    pub error: Option<String>,
}

impl ConnectionFormComponent {
    /// `drivers` comes from the connector registry in `main.rs` -- the one
    /// place that knows what's actually compiled in.
    pub fn new(mode: FormMode, drivers: Vec<String>, existing: Option<&SavedConnection>) -> Self {
        let driver = existing
            .and_then(|c| drivers.iter().position(|d| *d == c.driver))
            .unwrap_or(0);
        Self {
            mode,
            field: Field::Name,
            name: TextInput::new(existing.map_or("", |c| c.name.as_str())),
            target: TextInput::new(existing.map_or("", |c| c.target.as_str())),
            drivers,
            driver,
            error: None,
        }
    }

    fn driver_id(&self) -> String {
        self.drivers.get(self.driver).cloned().unwrap_or_default()
    }

    fn confirm(&mut self) -> Option<FormOutcome> {
        // Validate here rather than on save: a connection with no name is
        // invisible in the picker, and one with no target can't connect --
        // both are worth catching before they reach the file.
        if self.name.is_empty() {
            self.error = Some("name must not be empty".to_string());
            self.field = Field::Name;
            return None;
        }
        if self.target.is_empty() {
            self.error = Some("target must not be empty".to_string());
            self.field = Field::Target;
            return None;
        }
        if self.drivers.is_empty() {
            self.error = Some("no connectors are compiled into this build".to_string());
            return None;
        }
        Some(FormOutcome::Saved {
            connection: SavedConnection {
                name: self.name.text().trim().to_string(),
                driver: self.driver_id(),
                target: self.target.text().trim().to_string(),
            },
            mode: self.mode,
        })
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<FormOutcome> {
        let key = KeyPress::new(code, modifiers);
        let mut pending = None;
        if let Resolution::Command(command) = keymap().resolve(Context::Prompt, &mut pending, key) {
            match command {
                Command::Cancel => return Some(FormOutcome::Cancelled),
                Command::Confirm => return self.confirm(),
                Command::NextField => {
                    self.field = self.field.next();
                    return None;
                }
                Command::PrevField => {
                    self.field = self.field.prev();
                    return None;
                }
                _ => {}
            }
        }

        // The driver field is a picker, so left/right cycle it instead of
        // moving a text cursor that isn't there.
        if self.field == Field::Driver {
            match code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.driver = self.driver.saturating_sub(1);
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.driver = (self.driver + 1).min(self.drivers.len().saturating_sub(1));
                }
                _ => {}
            }
            return None;
        }

        let input = match self.field {
            Field::Name => &mut self.name,
            Field::Target => &mut self.target,
            Field::Driver => unreachable!("handled above"),
        };
        if input.handle_key_event(code, modifiers) {
            self.error = None;
        }
        None
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let title = match self.mode {
            FormMode::Add => "New connection",
            FormMode::Edit(_) => "Edit connection",
        };
        let confirm = keymap()
            .binding_for(Context::Prompt, Command::Confirm)
            .unwrap_or_default();
        let next = keymap()
            .binding_for(Context::Prompt, Command::NextField)
            .unwrap_or_default();
        let cancel = keymap()
            .binding_for(Context::Prompt, Command::Cancel)
            .unwrap_or_default();

        let mut lines: Vec<Line> = Vec::new();
        for field in Field::ORDER {
            let focused = field == self.field;
            let label = Span::styled(
                format!("{:<8}", field.label()),
                if focused {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
            );
            let mut spans = vec![label];
            match field {
                Field::Name => spans.extend(self.name.spans(focused)),
                Field::Target => spans.extend(self.target.spans(focused)),
                Field::Driver => {
                    // Show the whole set, marking the chosen one, so the
                    // valid values are visible instead of guessable.
                    for (i, driver) in self.drivers.iter().enumerate() {
                        let selected = i == self.driver;
                        let style = if selected && focused {
                            ui::selection_style()
                        } else if selected {
                            Style::default()
                                .fg(theme.accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.text_dim)
                        };
                        spans.push(Span::styled(format!(" {driver} "), style));
                    }
                }
            }
            lines.push(Line::from(spans));
        }

        lines.push(Line::from(""));
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                error.clone(),
                Style::default().fg(theme.error),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("{next} next field · ←/→ pick driver · {confirm} save · {cancel} cancel"),
                Style::default().fg(theme.text_dim),
            )));
        }

        let mut block = ui::panel(title, true);
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
    use ratatui::buffer::Buffer;

    use super::*;

    fn drivers() -> Vec<String> {
        vec!["postgres".to_string(), "sqlite".to_string()]
    }

    fn form() -> ConnectionFormComponent {
        ConnectionFormComponent::new(FormMode::Add, drivers(), None)
    }

    fn type_str(form: &mut ConnectionFormComponent, text: &str) {
        for c in text.chars() {
            form.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn filling_every_field_and_confirming_yields_a_connection() {
        let mut form = form();

        type_str(&mut form, "local pg");
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE); // -> driver
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE); // -> target
        type_str(&mut form, "postgres://localhost/db");
        let outcome = form.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(FormOutcome::Saved {
                connection: SavedConnection {
                    name: "local pg".to_string(),
                    driver: "postgres".to_string(),
                    target: "postgres://localhost/db".to_string(),
                },
                mode: FormMode::Add,
            })
        );
    }

    #[test]
    fn the_driver_field_cycles_through_the_compiled_in_connectors() {
        let mut form = form();
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);

        form.handle_key_event(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(form.driver_id(), "sqlite");

        form.handle_key_event(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(form.driver_id(), "sqlite", "must stop at the last driver");

        form.handle_key_event(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(form.driver_id(), "postgres");
        form.handle_key_event(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(form.driver_id(), "postgres", "must stop at the first");
    }

    #[test]
    fn typing_in_the_driver_field_does_not_edit_text() {
        let mut form = form();
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);

        type_str(&mut form, "zzz");

        // `h`/`l` are the picker's own keys there, and nothing lands in a
        // text field.
        assert_eq!(form.name.text(), "");
        assert_eq!(form.target.text(), "");
    }

    #[test]
    fn an_empty_name_is_rejected_and_focuses_the_offending_field() {
        let mut form = form();
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        form.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
        type_str(&mut form, "test.db");

        let outcome = form.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None);
        assert_eq!(form.field, Field::Name);
        assert!(form.error.as_deref().unwrap().contains("name"));
    }

    #[test]
    fn an_empty_target_is_rejected() {
        let mut form = form();
        type_str(&mut form, "no target");

        let outcome = form.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None);
        assert_eq!(form.field, Field::Target);
        assert!(form.error.as_deref().unwrap().contains("target"));
    }

    #[test]
    fn typing_clears_a_previous_error() {
        let mut form = form();
        form.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);
        assert!(form.error.is_some());

        type_str(&mut form, "x");

        assert_eq!(form.error, None);
    }

    #[test]
    fn esc_cancels() {
        let mut form = form();

        let outcome = form.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(outcome, Some(FormOutcome::Cancelled));
    }

    #[test]
    fn editing_starts_from_the_existing_connection() {
        let existing = SavedConnection {
            name: "local sqlite".to_string(),
            driver: "sqlite".to_string(),
            target: "test.db".to_string(),
        };

        let mut form = ConnectionFormComponent::new(FormMode::Edit(2), drivers(), Some(&existing));
        let outcome = form.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(FormOutcome::Saved {
                connection: existing,
                mode: FormMode::Edit(2),
            }),
            "confirming an untouched edit round-trips the connection unchanged"
        );
    }

    #[test]
    fn draw_shows_every_field_and_the_available_drivers() {
        let form = form();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| form.draw(frame, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("New connection"), "buffer was: {text}");
        assert!(text.contains("Name"), "buffer was: {text}");
        assert!(text.contains("Driver"), "buffer was: {text}");
        assert!(text.contains("Target"), "buffer was: {text}");
        assert!(text.contains("postgres"), "buffer was: {text}");
        assert!(text.contains("sqlite"), "buffer was: {text}");
    }
}
