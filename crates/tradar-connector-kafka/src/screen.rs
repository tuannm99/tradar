//! `KafkaScreen`: the bespoke `Component` a `KafkaSession` builds. One
//! mode -- Topics -- with a live-tailing message table (auto-scrolling
//! unless paused) plus a compose overlay for publishing. See "Thiết kế
//! UI: Kafka và RabbitMQ" in docs/architecture.md; Groups mode (consumer
//! lag) is deferred, see `docs/backlog.md`.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap};

use tradar_connector_spi::Session as ConnectorSession;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};
use tradar_core::vim_list;

use crate::KafkaSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposeField {
    Key,
    Value,
}

struct ComposeState {
    key: TextInput,
    value: TextInput,
    field: ComposeField,
}

impl ComposeState {
    fn new() -> Self {
        Self {
            key: TextInput::new(""),
            value: TextInput::new(""),
            field: ComposeField::Value,
        }
    }

    fn toggle_field(&mut self) {
        self.field = match self.field {
            ComposeField::Key => ComposeField::Value,
            ComposeField::Value => ComposeField::Key,
        };
    }

    fn active_mut(&mut self) -> &mut TextInput {
        match self.field {
            ComposeField::Key => &mut self.key,
            ComposeField::Value => &mut self.value,
        }
    }
}

pub struct KafkaScreen {
    session: KafkaSession,
    #[allow(dead_code)]
    action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    sidebar_selected: usize,
    sidebar_visible_height: usize,
    /// `Some(n)` freezes the message view to the first `n` buffered
    /// messages -- `KafkaSession` keeps receiving and buffering regardless
    /// (see `docs/architecture.md`), only the *drawn* view stops advancing.
    paused_at_len: Option<usize>,
    compose: Option<ComposeState>,
    pending: Option<KeyPress>,
}

impl KafkaScreen {
    pub(crate) fn new(
        session: KafkaSession,
        action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            session,
            action_tx,
            sidebar_selected: 0,
            sidebar_visible_height: 0,
            paused_at_len: None,
            compose: None,
            pending: None,
        }
    }

    fn selected_topic(&self) -> Option<String> {
        self.session
            .topics
            .get(self.sidebar_selected)
            .map(|t| t.name.clone())
    }

    fn tail_selected(&mut self, from_beginning: bool) {
        let Some(topic) = self.selected_topic() else {
            return;
        };
        self.session.start_tail(&topic, from_beginning);
        self.paused_at_len = None;
    }

    fn toggle_pause(&mut self) {
        self.paused_at_len = match self.paused_at_len {
            Some(_) => None,
            None => Some(self.session.messages.len()),
        };
    }

    fn open_compose(&mut self) {
        if self.selected_topic().is_none() {
            return;
        }
        self.compose = Some(ComposeState::new());
    }

    fn submit_compose(&mut self) {
        let Some(compose) = self.compose.take() else {
            return;
        };
        let Some(topic) = self.selected_topic() else {
            return;
        };
        let key = compose.key.text();
        let key = (!key.is_empty()).then_some(key);
        self.session
            .publish(&topic, key.as_deref(), &compose.value.text());
    }

    fn handle_compose_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        let pending = &mut self.pending;
        match keymap().resolve_in(&[Context::Prompt], pending, key) {
            Resolution::Command(Command::Confirm) => {
                self.submit_compose();
                None
            }
            Resolution::Command(Command::Cancel) => {
                self.compose = None;
                None
            }
            Resolution::Command(Command::NextField | Command::PrevField) => {
                if let Some(compose) = self.compose.as_mut() {
                    compose.toggle_field();
                }
                None
            }
            Resolution::Pending => None,
            _ => {
                if let Some(compose) = self.compose.as_mut() {
                    compose.active_mut().handle_key_event(code, modifiers);
                }
                None
            }
        }
    }

    /// The rows to actually draw: the tail of either the whole buffer
    /// (following) or the first `paused_at_len` messages (frozen), fit to
    /// `max` visible rows.
    fn visible_rows(&self, max: usize) -> Vec<&crate::KafkaMessageRow> {
        let source: Vec<&crate::KafkaMessageRow> = match self.paused_at_len {
            Some(len) => self.session.messages.iter().take(len).collect(),
            None => self.session.messages.iter().collect(),
        };
        let start = source.len().saturating_sub(max);
        source[start..].to_vec()
    }

    fn draw_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        self.sidebar_visible_height = area.height.saturating_sub(2) as usize;
        let len = self.session.topics.len();
        self.sidebar_selected = self.sidebar_selected.min(len.saturating_sub(1));

        if let Some(error) = &self.session.error {
            let paragraph = Paragraph::new(error.as_str())
                .style(Style::default().fg(theme().error))
                .block(ui::panel("Topics", true))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        let items: Vec<ListItem> = self
            .session
            .topics
            .iter()
            .map(|t| {
                let tailing = self.session.tailing_topic.as_deref() == Some(t.name.as_str());
                let marker = if tailing { "● " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{marker}{}", t.name),
                        Style::default().fg(theme().text),
                    ),
                    Span::styled(
                        format!("  {} partitions", t.partitions),
                        Style::default().fg(theme().text_dim),
                    ),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        if len > 0 {
            state.select(Some(self.sidebar_selected));
        }
        let list = List::new(items)
            .block(ui::panel("Topics", true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn draw_main(&mut self, frame: &mut Frame, area: Rect) {
        let Some(topic) = &self.session.tailing_topic else {
            let placeholder =
                Paragraph::new("Select a topic and press enter to tail (b: from the beginning)")
                    .style(Style::default().fg(theme().text_dim))
                    .block(ui::panel("Messages", false));
            frame.render_widget(placeholder, area);
            return;
        };

        let visible = area.height.saturating_sub(3) as usize;
        let rows: Vec<Row> = self
            .visible_rows(visible)
            .into_iter()
            .map(|m| {
                Row::new(vec![
                    Cell::from(m.partition.to_string()),
                    Cell::from(m.offset.to_string()),
                    Cell::from(m.key.clone().unwrap_or_default()),
                    Cell::from(m.value.clone()),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(10),
                Constraint::Length(16),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(vec!["partition", "offset", "key", "value"])
                .style(Style::default().fg(theme().text_dim)),
        )
        .block(ui::panel(
            &format!(
                "Messages — {topic}{}",
                if self.paused_at_len.is_some() {
                    " (paused)"
                } else {
                    ""
                }
            ),
            false,
        ));
        frame.render_widget(table, area);
    }

    fn draw_compose(&mut self, frame: &mut Frame, area: Rect) {
        let Some(compose) = &self.compose else {
            return;
        };
        let popup = ui::centered_rect(60, 24, area);
        frame.render_widget(Clear, popup);
        let block = ui::panel(
            "Publish message — tab: switch field, enter: send, esc: cancel",
            true,
        );
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let field_line = |label: &str, input: &TextInput, focused: bool| {
            let mut spans = vec![Span::styled(
                format!("{label}: "),
                Style::default().fg(theme().text_dim),
            )];
            spans.extend(input.spans(focused));
            Line::from(spans)
        };

        frame.render_widget(
            Paragraph::new(field_line(
                "key (optional)",
                &compose.key,
                compose.field == ComposeField::Key,
            )),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(field_line(
                "value         ",
                &compose.value,
                compose.field == ComposeField::Value,
            )),
            rows[1],
        );
    }
}

impl Component for KafkaScreen {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if self.compose.is_some() {
            return self.handle_compose_key(code, modifiers);
        }

        let key = KeyPress::new(code, modifiers);
        let resolution =
            keymap().resolve_in(&[Context::Kafka, Context::List], &mut self.pending, key);
        let command = match resolution {
            Resolution::Command(command) => command,
            _ => return None,
        };
        if let Some(mv) = command.as_vim_move() {
            let mut selected = self.sidebar_selected;
            vim_list::apply(
                mv,
                &mut selected,
                self.session.topics.len(),
                self.sidebar_visible_height,
            );
            self.sidebar_selected = selected;
            return None;
        }
        match command {
            Command::KafkaRefresh => self.session.list_topics(),
            Command::KafkaTailLatest => self.tail_selected(false),
            Command::KafkaTailEarliest => self.tail_selected(true),
            Command::KafkaPauseFollow => self.toggle_pause(),
            Command::KafkaPublish => self.open_compose(),
            Command::Help => return Some(Action::ShowHelp),
            Command::Back => return Some(Action::BackToPicker),
            _ => {}
        }
        None
    }

    fn update(&mut self, _action: Action) -> Option<Action> {
        None
    }

    fn tick(&mut self) -> bool {
        self.session.tick()
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(20)])
            .split(area);
        self.draw_sidebar(frame, columns[0]);
        self.draw_main(frame, columns[1]);
        if self.compose.is_some() {
            self.draw_compose(frame, area);
        }
    }

    fn connection_alive(&self) -> Option<bool> {
        Some(self.session.error.is_none())
    }

    fn status_hints(&self) -> Vec<ui::Hint> {
        let mut hints = Vec::new();
        hints.extend(ui::hint(Context::Kafka, Command::KafkaRefresh, "refresh"));
        hints.extend(ui::hint(Context::Kafka, Command::KafkaTailLatest, "tail"));
        hints.extend(ui::hint(Context::Kafka, Command::KafkaPauseFollow, "pause"));
        hints.extend(ui::hint(Context::Kafka, Command::KafkaPublish, "publish"));
        hints.extend(ui::hint(Context::Kafka, Command::Back, "back"));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> KafkaScreen {
        let mut config = rdkafka::config::ClientConfig::new();
        config.set("bootstrap.servers", "127.0.0.1:1");
        let metadata_client: rdkafka::consumer::BaseConsumer =
            config.create().expect("client config must build offline");
        let producer: rdkafka::producer::FutureProducer =
            config.create().expect("producer config must build offline");
        let session = KafkaSession::new(
            "127.0.0.1:1".to_string(),
            producer,
            std::sync::Arc::new(metadata_client),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        KafkaScreen::new(session, tx)
    }

    #[tokio::test]
    async fn starts_with_nothing_tailing_and_not_paused() {
        let screen = screen();

        assert_eq!(screen.session.tailing_topic, None);
        assert_eq!(screen.paused_at_len, None);
    }

    #[tokio::test]
    async fn pausing_freezes_the_buffer_length_and_resuming_clears_it() {
        let mut screen = screen();
        screen.session.messages.push_back(crate::KafkaMessageRow {
            partition: 0,
            offset: 0,
            key: None,
            value: "a".to_string(),
        });

        screen.toggle_pause();
        assert_eq!(screen.paused_at_len, Some(1));

        screen.toggle_pause();
        assert_eq!(screen.paused_at_len, None);
    }

    #[test]
    fn compose_state_toggles_between_its_two_fields() {
        let mut compose = ComposeState::new();
        assert_eq!(compose.field, ComposeField::Value);

        compose.toggle_field();
        assert_eq!(compose.field, ComposeField::Key);
        compose.toggle_field();
        assert_eq!(compose.field, ComposeField::Value);
    }
}
