//! `RabbitScreen`: the bespoke `Component` a `RabbitSession` builds. Two
//! modes toggled by `Command::ToggleRabbitMode` -- Queues (peek messages,
//! non-destructively) and Exchanges (browse bindings) -- plus a compose
//! overlay for publishing. See "Thiết kế UI: Kafka và RabbitMQ" in
//! docs/architecture.md for the design this implements.

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

use crate::RabbitSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RabbitMode {
    Queues,
    Exchanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposeField {
    Exchange,
    RoutingKey,
    Payload,
}

struct ComposeState {
    exchange: TextInput,
    routing_key: TextInput,
    payload: TextInput,
    field: ComposeField,
}

impl ComposeState {
    fn new(default_exchange: &str) -> Self {
        Self {
            exchange: TextInput::new(default_exchange),
            routing_key: TextInput::new(""),
            payload: TextInput::new(""),
            field: ComposeField::RoutingKey,
        }
    }

    fn next_field(&mut self) {
        self.field = match self.field {
            ComposeField::Exchange => ComposeField::RoutingKey,
            ComposeField::RoutingKey => ComposeField::Payload,
            ComposeField::Payload => ComposeField::Exchange,
        };
    }

    fn prev_field(&mut self) {
        self.field = match self.field {
            ComposeField::Exchange => ComposeField::Payload,
            ComposeField::RoutingKey => ComposeField::Exchange,
            ComposeField::Payload => ComposeField::RoutingKey,
        };
    }

    fn active_mut(&mut self) -> &mut TextInput {
        match self.field {
            ComposeField::Exchange => &mut self.exchange,
            ComposeField::RoutingKey => &mut self.routing_key,
            ComposeField::Payload => &mut self.payload,
        }
    }
}

pub struct RabbitScreen {
    session: RabbitSession,
    #[allow(dead_code)]
    action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    mode: RabbitMode,
    sidebar_selected: usize,
    sidebar_visible_height: usize,
    selected_queue: Option<String>,
    selected_exchange: Option<String>,
    compose: Option<ComposeState>,
    pending: Option<KeyPress>,
}

impl RabbitScreen {
    pub(crate) fn new(
        session: RabbitSession,
        action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            session,
            action_tx,
            mode: RabbitMode::Queues,
            sidebar_selected: 0,
            sidebar_visible_height: 0,
            selected_queue: None,
            selected_exchange: None,
            compose: None,
            pending: None,
        }
    }

    fn sidebar_len(&self) -> usize {
        match self.mode {
            RabbitMode::Queues => self.session.queues.len(),
            RabbitMode::Exchanges => self.session.exchanges.len(),
        }
    }

    fn selected_name(&self) -> Option<String> {
        match self.mode {
            RabbitMode::Queues => self
                .session
                .queues
                .get(self.sidebar_selected)
                .map(|q| q.name.clone()),
            RabbitMode::Exchanges => self
                .session
                .exchanges
                .get(self.sidebar_selected)
                .map(|e| e.name.clone()),
        }
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            RabbitMode::Queues => RabbitMode::Exchanges,
            RabbitMode::Exchanges => RabbitMode::Queues,
        };
        self.sidebar_selected = 0;
    }

    fn refresh(&self) {
        match self.mode {
            RabbitMode::Queues => {
                self.session.refresh_queues();
                if let Some(queue) = &self.selected_queue {
                    self.session.peek_messages(queue);
                }
            }
            RabbitMode::Exchanges => {
                self.session.refresh_exchanges();
                if let Some(exchange) = &self.selected_exchange {
                    self.session.list_bindings(exchange);
                }
            }
        }
    }

    fn open_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        match self.mode {
            RabbitMode::Queues => {
                self.session.peek_messages(&name);
                self.selected_queue = Some(name);
            }
            RabbitMode::Exchanges => {
                self.session.list_bindings(&name);
                self.selected_exchange = Some(name);
            }
        }
    }

    fn open_compose(&mut self) {
        let default_exchange = self.selected_exchange.clone().unwrap_or_default();
        self.compose = Some(ComposeState::new(&default_exchange));
    }

    fn submit_compose(&mut self) {
        let Some(compose) = self.compose.take() else {
            return;
        };
        self.session.publish(
            &compose.exchange.text(),
            &compose.routing_key.text(),
            &compose.payload.text(),
        );
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
            Resolution::Command(Command::NextField) => {
                if let Some(compose) = self.compose.as_mut() {
                    compose.next_field();
                }
                None
            }
            Resolution::Command(Command::PrevField) => {
                if let Some(compose) = self.compose.as_mut() {
                    compose.prev_field();
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

    fn draw_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        self.sidebar_visible_height = area.height.saturating_sub(2) as usize;
        let len = self.sidebar_len();
        self.sidebar_selected = self.sidebar_selected.min(len.saturating_sub(1));

        if let Some(error) = &self.session.error {
            let paragraph = Paragraph::new(error.as_str())
                .style(Style::default().fg(theme().error))
                .block(ui::panel(self.mode_title(), true))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        let items: Vec<ListItem> = match self.mode {
            RabbitMode::Queues => self
                .session
                .queues
                .iter()
                .map(|q| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {}", q.name), Style::default().fg(theme().text)),
                        Span::styled(
                            format!(
                                "  {} ready, {} unacked, {} consumers",
                                q.messages_ready, q.messages_unacknowledged, q.consumers
                            ),
                            Style::default().fg(theme().text_dim),
                        ),
                    ]))
                })
                .collect(),
            RabbitMode::Exchanges => self
                .session
                .exchanges
                .iter()
                .map(|e| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {}", e.name), Style::default().fg(theme().text)),
                        Span::styled(
                            format!("  {}", e.kind),
                            Style::default().fg(theme().text_dim),
                        ),
                    ]))
                })
                .collect(),
        };

        let mut state = ListState::default();
        if len > 0 {
            state.select(Some(self.sidebar_selected));
        }
        let list = List::new(items)
            .block(ui::panel(self.mode_title(), true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn mode_title(&self) -> &'static str {
        match self.mode {
            RabbitMode::Queues => "Queues",
            RabbitMode::Exchanges => "Exchanges",
        }
    }

    fn draw_main(&mut self, frame: &mut Frame, area: Rect) {
        match self.mode {
            RabbitMode::Queues => {
                let Some(queue) = &self.selected_queue else {
                    let placeholder = Paragraph::new("Select a queue and press enter to peek")
                        .style(Style::default().fg(theme().text_dim))
                        .block(ui::panel("Messages", false));
                    frame.render_widget(placeholder, area);
                    return;
                };
                let rows: Vec<Row> = self
                    .session
                    .messages
                    .iter()
                    .map(|m| {
                        Row::new(vec![
                            Cell::from(m.routing_key.clone()),
                            Cell::from(m.exchange.clone()),
                            Cell::from(if m.redelivered { "yes" } else { "no" }),
                            Cell::from(m.payload.clone()),
                        ])
                    })
                    .collect();
                let table = Table::new(
                    rows,
                    [
                        Constraint::Length(16),
                        Constraint::Length(16),
                        Constraint::Length(10),
                        Constraint::Min(20),
                    ],
                )
                .header(
                    Row::new(vec!["routing key", "exchange", "redelivered", "payload"])
                        .style(Style::default().fg(theme().text_dim)),
                )
                .block(ui::panel(&format!("Messages — {queue}"), false));
                frame.render_widget(table, area);
            }
            RabbitMode::Exchanges => {
                let Some(exchange) = &self.selected_exchange else {
                    let placeholder =
                        Paragraph::new("Select an exchange and press enter to see bindings")
                            .style(Style::default().fg(theme().text_dim))
                            .block(ui::panel("Bindings", false));
                    frame.render_widget(placeholder, area);
                    return;
                };
                let rows: Vec<Row> = self
                    .session
                    .bindings
                    .iter()
                    .map(|b| {
                        Row::new(vec![
                            Cell::from(b.destination.clone()),
                            Cell::from(b.routing_key.clone()),
                        ])
                    })
                    .collect();
                let table = Table::new(rows, [Constraint::Length(24), Constraint::Min(16)])
                    .header(
                        Row::new(vec!["destination queue", "routing key"])
                            .style(Style::default().fg(theme().text_dim)),
                    )
                    .block(ui::panel(&format!("Bindings — {exchange}"), false));
                frame.render_widget(table, area);
            }
        }
    }

    fn draw_compose(&mut self, frame: &mut Frame, area: Rect) {
        let Some(compose) = &self.compose else {
            return;
        };
        let popup = ui::centered_rect(60, 30, area);
        frame.render_widget(Clear, popup);
        let block = ui::panel(
            "Publish message — tab: next field, enter: send, esc: cancel",
            true,
        );
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
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
                "exchange   ",
                &compose.exchange,
                compose.field == ComposeField::Exchange,
            )),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(field_line(
                "routing key",
                &compose.routing_key,
                compose.field == ComposeField::RoutingKey,
            )),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(field_line(
                "payload    ",
                &compose.payload,
                compose.field == ComposeField::Payload,
            )),
            rows[3],
        );
    }
}

impl Component for RabbitScreen {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if self.compose.is_some() {
            return self.handle_compose_key(code, modifiers);
        }

        let key = KeyPress::new(code, modifiers);
        let resolution =
            keymap().resolve_in(&[Context::Rabbit, Context::List], &mut self.pending, key);
        let command = match resolution {
            Resolution::Command(command) => command,
            _ => return None,
        };
        if let Some(mv) = command.as_vim_move() {
            let mut selected = self.sidebar_selected;
            vim_list::apply(
                mv,
                &mut selected,
                self.sidebar_len(),
                self.sidebar_visible_height,
            );
            self.sidebar_selected = selected;
            return None;
        }
        match command {
            Command::ToggleRabbitMode => self.toggle_mode(),
            Command::RabbitRefresh => self.refresh(),
            Command::RabbitOpen => self.open_selected(),
            Command::RabbitPublish => self.open_compose(),
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
        hints.extend(ui::hint(Context::Rabbit, Command::ToggleRabbitMode, "mode"));
        hints.extend(ui::hint(Context::Rabbit, Command::RabbitOpen, "open"));
        hints.extend(ui::hint(Context::Rabbit, Command::RabbitPublish, "publish"));
        hints.extend(ui::hint(Context::Rabbit, Command::Back, "back"));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn screen() -> RabbitScreen {
        let target = crate::parse_target("http://user:pass@localhost:15672").unwrap();
        let session = RabbitSession::new(reqwest::Client::new(), target);
        let (tx, _rx) = mpsc::unbounded_channel();
        RabbitScreen::new(session, tx)
    }

    // `RabbitSession::new` fires off its initial refresh via `tokio::spawn`
    // (see "Screen không bao giờ làm IO" in docs/architecture.md), which
    // needs a running runtime even though these tests never await anything
    // themselves -- hence `#[tokio::test]` rather than plain `#[test]`.

    #[tokio::test]
    async fn starts_in_queues_mode_with_nothing_selected() {
        let screen = screen();

        assert_eq!(screen.mode, RabbitMode::Queues);
        assert_eq!(screen.selected_queue, None);
        assert_eq!(screen.selected_exchange, None);
    }

    #[tokio::test]
    async fn toggle_mode_switches_and_resets_the_cursor() {
        let mut screen = screen();
        screen.sidebar_selected = 3;

        screen.toggle_mode();
        assert_eq!(screen.mode, RabbitMode::Exchanges);
        assert_eq!(screen.sidebar_selected, 0);

        screen.toggle_mode();
        assert_eq!(screen.mode, RabbitMode::Queues);
    }

    #[test]
    fn compose_state_cycles_through_all_three_fields() {
        let mut compose = ComposeState::new("");
        assert_eq!(compose.field, ComposeField::RoutingKey);

        compose.next_field();
        assert_eq!(compose.field, ComposeField::Payload);
        compose.next_field();
        assert_eq!(compose.field, ComposeField::Exchange);
        compose.next_field();
        assert_eq!(compose.field, ComposeField::RoutingKey);

        compose.prev_field();
        assert_eq!(compose.field, ComposeField::Exchange);
    }
}
