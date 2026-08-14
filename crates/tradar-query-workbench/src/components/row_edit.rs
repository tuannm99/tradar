//! The overlay behind "change this cell" / "delete this row" in the
//! results grid. Deliberately not a direct write: it types a value, shows
//! the exact statement that would run, and only then runs it. Editing a
//! database from a grid is a write you can't undo, so the statement is
//! never hidden -- what you approve is what executes, and it lands in the
//! query history like anything else you ran.
//!
//! Not a `Component`: like `FilePromptComponent`, it's driven directly by
//! `QueryScreenComponent`, which owns the schema and the driver and is
//! therefore the only place that can build the statement.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};

pub enum RowEditOutcome {
    Cancelled,
    /// A new value was typed. The host builds the statement from it and
    /// hands it back with `show_statement` -- this component knows nothing
    /// about SQL, or about which table the row came from.
    ValueEntered(String),
    /// The user approved the statement: run it.
    Confirmed(String),
}

enum Stage {
    /// Typing a replacement value.
    Value(TextInput),
    /// Showing the statement, waiting for a yes.
    Confirm(String),
    /// Nothing can run, and why. Dismiss only.
    Blocked(String),
}

pub struct RowEditComponent {
    title: String,
    stage: Stage,
}

impl RowEditComponent {
    /// Asks for a new value for `column`, prefilled with what's there now.
    pub fn value(column: &str, current: &str) -> Self {
        Self {
            title: format!("Set {column}"),
            stage: Stage::Value(TextInput::new(current)),
        }
    }

    /// Skips straight to approving `statement` -- a delete has no value to
    /// type.
    pub fn confirm(title: &str, statement: String) -> Self {
        Self {
            title: title.to_string(),
            stage: Stage::Confirm(statement),
        }
    }

    /// Explains why this result can't be edited. A refusal with no reason
    /// reads as a broken key.
    pub fn blocked(title: &str, reason: String) -> Self {
        Self {
            title: title.to_string(),
            stage: Stage::Blocked(reason),
        }
    }

    /// Moves from typing a value to approving the statement built from it.
    pub fn show_statement(&mut self, statement: String) {
        self.stage = Stage::Confirm(statement);
    }

    /// Reports a failure to build the statement, in place, rather than
    /// closing and leaving no sign of what happened.
    pub fn show_problem(&mut self, reason: String) {
        self.stage = Stage::Blocked(reason);
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<RowEditOutcome> {
        if code == KeyCode::Esc {
            return Some(RowEditOutcome::Cancelled);
        }
        match &mut self.stage {
            Stage::Value(input) => {
                if code == KeyCode::Enter {
                    return Some(RowEditOutcome::ValueEntered(input.text()));
                }
                input.handle_key_event(code, modifiers);
                None
            }
            // A plain `y`, the same answer the connection picker's delete
            // asks for -- and anything else cancels, so a stray key can
            // never write to the database.
            Stage::Confirm(statement) => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(RowEditOutcome::Confirmed(statement.clone()))
                }
                _ => Some(RowEditOutcome::Cancelled),
            },
            Stage::Blocked(_) => match code {
                KeyCode::Enter | KeyCode::Char(_) => Some(RowEditOutcome::Cancelled),
                _ => None,
            },
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let block = ui::panel(&self.title, true);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = match &self.stage {
            Stage::Value(input) => vec![
                Line::from(input.spans(true)),
                Line::from(""),
                Line::from(Span::styled(
                    "enter to build the statement, esc to cancel",
                    Style::default().fg(theme.text_dim),
                )),
            ],
            Stage::Confirm(statement) => vec![
                Line::from(Span::styled(
                    "This will run:",
                    Style::default().fg(theme.text_dim),
                )),
                Line::from(Span::styled(
                    statement.clone(),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "y to run, any other key to cancel",
                    Style::default().fg(theme.warning),
                )),
            ],
            Stage::Blocked(reason) => vec![
                Line::from(Span::styled(
                    reason.clone(),
                    Style::default().fg(theme.error),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "esc to dismiss",
                    Style::default().fg(theme.text_dim),
                )),
            ],
        };

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn draw(component: &RowEditComponent) -> String {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| component.draw(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn typing_a_value_and_confirming_hands_it_back_for_the_host_to_build() {
        let mut component = RowEditComponent::value("name", "Ada");

        component.handle_key_event(KeyCode::Char('!'), KeyModifiers::NONE);
        let outcome = component.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        match outcome {
            Some(RowEditOutcome::ValueEntered(value)) => assert_eq!(value, "Ada!"),
            _ => panic!("expected the typed value back"),
        }
    }

    #[test]
    fn the_statement_has_to_be_approved_with_y_before_it_runs() {
        let mut component =
            RowEditComponent::confirm("Delete row", "DELETE FROM t WHERE id = '1'".to_string());

        let text = draw(&component);
        assert!(
            text.contains("DELETE FROM t WHERE id = '1'"),
            "the statement must be visible before it runs: {text}"
        );

        match component.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE) {
            Some(RowEditOutcome::Confirmed(sql)) => {
                assert_eq!(sql, "DELETE FROM t WHERE id = '1'")
            }
            _ => panic!("expected the statement to be confirmed"),
        }
    }

    #[test]
    fn any_other_key_cancels_rather_than_running_the_statement() {
        let mut component = RowEditComponent::confirm("Delete row", "DELETE FROM t".to_string());

        assert!(matches!(
            component.handle_key_event(KeyCode::Char('n'), KeyModifiers::NONE),
            Some(RowEditOutcome::Cancelled)
        ));
    }

    #[test]
    fn esc_cancels_from_the_value_stage_too() {
        let mut component = RowEditComponent::value("name", "Ada");

        assert!(matches!(
            component.handle_key_event(KeyCode::Esc, KeyModifiers::NONE),
            Some(RowEditOutcome::Cancelled)
        ));
    }

    #[test]
    fn a_blocked_edit_says_why() {
        let component = RowEditComponent::blocked(
            "Edit cell",
            "users has no primary key: there is no WHERE that names one row".to_string(),
        );

        let text = draw(&component);

        assert!(text.contains("no primary key"), "buffer was: {text}");
    }
}
