//! `KafkaScreen`: the bespoke `Component` a `KafkaSession` builds. Two
//! modes toggled by `Command::KafkaToggleMode` -- Topics (live-tailing
//! message table, auto-scrolling unless paused, plus a compose overlay for
//! publishing) and Groups (per-partition consumer lag for a selected
//! group) -- mirroring the `RabbitScreen`'s Queues/Exchanges toggle. See
//! "Thiết kế UI: Kafka và RabbitMQ" in docs/architecture.md and
//! `docs/backlog/kafka-groups-mode-2026-10-01.md` for the design Groups
//! mode implements.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, Wrap};

use tradar_connector_spi::Session as ConnectorSession;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, DoubleClickTracker, TextInput};
use tradar_core::vim_list::{self, VimMove};

use crate::KafkaSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KafkaMode {
    Topics,
    Groups,
}

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
    mode: KafkaMode,
    sidebar_selected: usize,
    sidebar_visible_height: usize,
    /// Where the sidebar was last drawn (after the filter bar, if any,
    /// shrank it) -- a click is hit-tested against these exact bounds,
    /// same role as `BrowseSidebarComponent::list_area`.
    sidebar_area: Rect,
    /// Persisted across frames (not rebuilt fresh in `draw_sidebar`) so
    /// `.offset()` reflects whatever scroll position the last render
    /// actually used -- a click's row has to be mapped through that same
    /// offset, same reasoning as `ConnectionPickerComponent::list_state`.
    list_state: ListState,
    double_click: DoubleClickTracker,
    /// `Some(n)` freezes the message view to the first `n` buffered
    /// messages -- `KafkaSession` keeps receiving and buffering regardless
    /// (see `docs/architecture.md`), only the *drawn* view stops advancing.
    paused_at_len: Option<usize>,
    /// The group whose lag `draw_lag` shows -- `None` until one's opened in
    /// Groups mode, same role as Rabbit's `selected_queue`/
    /// `selected_exchange`. The lag data itself lives on `session.group_lag`
    /// (populated asynchronously), not here.
    selected_group: Option<String>,
    compose: Option<ComposeState>,
    pending: Option<KeyPress>,
    /// A case-insensitive substring narrowing the sidebar (topics or
    /// groups, whichever mode is active) -- same idiom as
    /// `NavigatorComponent::filter`. Kept even while `filter_input` is
    /// closed so the title can still say what's applied and a fresh `/`
    /// prefills it rather than starting over. Reset on `toggle_mode`, same
    /// as `sidebar_selected` -- a filter typed for topic names wouldn't
    /// mean anything against group names.
    filter: String,
    /// `Some` while the filter bar has the keys -- see `open_filter`.
    filter_input: Option<TextInput>,
}

impl KafkaScreen {
    pub(crate) fn new(
        session: KafkaSession,
        action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            session,
            action_tx,
            mode: KafkaMode::Topics,
            sidebar_selected: 0,
            sidebar_visible_height: 0,
            sidebar_area: Rect::ZERO,
            list_state: ListState::default(),
            double_click: DoubleClickTracker::new(),
            paused_at_len: None,
            selected_group: None,
            compose: None,
            pending: None,
            filter: String::new(),
            filter_input: None,
        }
    }

    /// `session.topics` narrowed by `filter`, when one's applied -- same
    /// idiom as `NavigatorComponent::visible_rows`.
    fn visible_topics(&self) -> Vec<&crate::TopicInfo> {
        if self.filter.is_empty() {
            return self.session.topics.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.session
            .topics
            .iter()
            .filter(|t| t.name.to_lowercase().contains(&needle))
            .collect()
    }

    /// `session.groups` narrowed by `filter`, when one's applied -- same
    /// idiom as `visible_topics`.
    fn visible_groups(&self) -> Vec<&crate::ConsumerGroupInfo> {
        if self.filter.is_empty() {
            return self.session.groups.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.session
            .groups
            .iter()
            .filter(|g| g.name.to_lowercase().contains(&needle))
            .collect()
    }

    fn sidebar_len(&self) -> usize {
        match self.mode {
            KafkaMode::Topics => self.visible_topics().len(),
            KafkaMode::Groups => self.visible_groups().len(),
        }
    }

    fn selected_topic(&self) -> Option<String> {
        self.visible_topics()
            .get(self.sidebar_selected)
            .map(|t| t.name.clone())
    }

    fn selected_group_name(&self) -> Option<String> {
        self.visible_groups()
            .get(self.sidebar_selected)
            .map(|g| g.name.clone())
    }

    /// Switches Topics<->Groups, resetting sidebar position and filter --
    /// neither would mean anything carried over to the other mode's
    /// entirely different name space. Unlike `RabbitScreen::toggle_mode`,
    /// doesn't need to eagerly re-fetch: `KafkaSession::new` already lists
    /// both topics and groups upfront, mirroring `RabbitSession::new`
    /// fetching both queues and exchanges upfront.
    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            KafkaMode::Topics => KafkaMode::Groups,
            KafkaMode::Groups => KafkaMode::Topics,
        };
        self.sidebar_selected = 0;
        self.filter.clear();
        self.filter_input = None;
    }

    /// Re-fetches the current mode's sidebar list, and the open group's lag
    /// too if one's already shown -- same shape as `RabbitScreen::refresh`.
    fn refresh(&self) {
        match self.mode {
            KafkaMode::Topics => self.session.list_topics(),
            KafkaMode::Groups => {
                self.session.list_groups();
                if let Some(group) = &self.selected_group {
                    self.session.fetch_group_lag(group);
                }
            }
        }
    }

    /// `Command::KafkaOpen`'s handler -- dispatches by mode, same pattern
    /// as `RabbitScreen::open_selected`.
    fn open_selected(&mut self) {
        match self.mode {
            KafkaMode::Topics => self.tail_selected(false),
            KafkaMode::Groups => self.show_lag_selected(),
        }
    }

    fn show_lag_selected(&mut self) {
        let Some(name) = self.selected_group_name() else {
            return;
        };
        self.session.fetch_group_lag(&name);
        self.selected_group = Some(name);
    }

    /// Opens the filter bar, prefilled with whatever's already applied.
    fn open_filter(&mut self) {
        self.filter_input = Some(TextInput::new(&self.filter));
    }

    fn is_filtering(&self) -> bool {
        self.filter_input.is_some()
    }

    /// One key while the filter bar has the keys. `Esc` cancels -- clears
    /// the bar *and* whatever was applied. `Enter` keeps the filter and
    /// closes the bar. Anything else is text editing, applied live.
    fn filter_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let Some(input) = self.filter_input.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                self.filter_input = None;
                self.filter.clear();
                self.sidebar_selected = 0;
            }
            KeyCode::Enter => self.filter_input = None,
            _ => {
                input.handle_key_event(code, modifiers);
                self.filter = input.text();
                self.sidebar_selected = 0;
            }
        }
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
        let (area, filter_bar_area) = if self.filter_input.is_some() {
            let (list_area, bar) = ui::split_bottom_bar(area, 1);
            (list_area, Some(bar))
        } else {
            (area, None)
        };

        self.sidebar_visible_height = area.height.saturating_sub(2) as usize;
        self.sidebar_area = area;
        let label = match self.mode {
            KafkaMode::Topics => "Topics",
            KafkaMode::Groups => "Groups",
        };

        if let Some(error) = &self.session.error {
            let paragraph = Paragraph::new(error.as_str())
                .style(Style::default().fg(theme().error))
                .block(ui::panel(label, true))
                .wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
            return;
        }

        // `len` computed and dropped before the `sidebar_selected` write
        // below -- `visible_topics()`/`visible_groups()` elide to
        // borrowing all of `&self`, so holding either's `Vec<&_>` across
        // that write (as a single `let visible = ...` spanning both)
        // borrow-checker-fails despite `sidebar_selected` and
        // `topics`/`groups` being disjoint fields.
        let len = self.sidebar_len();
        self.sidebar_selected = self.sidebar_selected.min(len.saturating_sub(1));

        let items: Vec<ListItem> = match self.mode {
            KafkaMode::Topics => self
                .visible_topics()
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
                .collect(),
            KafkaMode::Groups => self
                .visible_groups()
                .iter()
                .map(|g| {
                    let shown = self.selected_group.as_deref() == Some(g.name.as_str());
                    let marker = if shown { "● " } else { "  " };
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{marker}{}", g.name),
                            Style::default().fg(theme().text),
                        ),
                        Span::styled(
                            format!("  {} ({} members)", g.state, g.members),
                            Style::default().fg(theme().text_dim),
                        ),
                    ]))
                })
                .collect(),
        };

        if len > 0 {
            self.list_state.select(Some(self.sidebar_selected));
        }
        let title = if self.filter.is_empty() {
            label.to_string()
        } else {
            format!("{label} — filter: {}", self.filter)
        };
        let list = List::new(items)
            .block(ui::panel(&title, true))
            .highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, area, &mut self.list_state);

        if let (Some(bar_area), Some(input)) = (filter_bar_area, &self.filter_input) {
            let mut spans = vec![Span::styled("/", Style::default().fg(theme().accent))];
            spans.extend(input.spans(true));
            frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
        }
    }

    fn draw_main(&mut self, frame: &mut Frame, area: Rect) {
        match self.mode {
            KafkaMode::Topics => self.draw_messages(frame, area),
            KafkaMode::Groups => self.draw_lag(frame, area),
        }
    }

    fn draw_messages(&mut self, frame: &mut Frame, area: Rect) {
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

    fn draw_lag(&mut self, frame: &mut Frame, area: Rect) {
        let Some(group) = &self.selected_group else {
            let placeholder =
                Paragraph::new("Select a consumer group and press enter to show its lag")
                    .style(Style::default().fg(theme().text_dim))
                    .block(ui::panel("Lag", false));
            frame.render_widget(placeholder, area);
            return;
        };

        let rows: Vec<Row> = self
            .session
            .group_lag
            .iter()
            .map(|r| {
                Row::new(vec![
                    Cell::from(r.topic.clone()),
                    Cell::from(r.partition.to_string()),
                    Cell::from(r.committed.to_string()),
                    Cell::from(r.high_watermark.to_string()),
                    Cell::from(r.lag().to_string()),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Min(16),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(15),
                Constraint::Length(8),
            ],
        )
        .header(
            Row::new(vec![
                "topic",
                "partition",
                "committed",
                "high-watermark",
                "lag",
            ])
            .style(Style::default().fg(theme().text_dim)),
        )
        .block(ui::panel(&format!("Lag — {group}"), false));
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
        if self.is_filtering() {
            self.filter_key_event(code, modifiers);
            return None;
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
                self.sidebar_len(),
                self.sidebar_visible_height,
            );
            self.sidebar_selected = selected;
            return None;
        }
        match command {
            Command::KafkaToggleMode => self.toggle_mode(),
            Command::KafkaRefresh => self.refresh(),
            Command::KafkaOpen => self.open_selected(),
            Command::KafkaTailEarliest if self.mode == KafkaMode::Topics => {
                self.tail_selected(true)
            }
            Command::KafkaPauseFollow if self.mode == KafkaMode::Topics => self.toggle_pause(),
            Command::KafkaPublish if self.mode == KafkaMode::Topics => self.open_compose(),
            Command::Search => self.open_filter(),
            Command::Help => return Some(Action::ShowHelp),
            Command::Back => return Some(Action::BackToPicker),
            _ => {}
        }
        None
    }

    /// Click-to-select/double-click-to-open on the sidebar, plus
    /// scroll-wheel to move the selection -- same pattern
    /// `ConnectionPickerComponent`/`BrowseSidebarComponent` already use.
    /// A no-op while the compose panel or filter bar has the keys, same
    /// reasoning `handle_key_event` applies to them.
    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        if self.compose.is_some() || self.is_filtering() {
            return None;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !ui::contains(self.sidebar_area, event.column, event.row) {
                    return None;
                }
                let inner = Rect {
                    x: self.sidebar_area.x.saturating_add(1),
                    y: self.sidebar_area.y.saturating_add(1),
                    width: self.sidebar_area.width.saturating_sub(2),
                    height: self.sidebar_area.height.saturating_sub(2),
                };
                let len = self.sidebar_len();
                if let Some(index) = ui::index_at(inner, self.list_state.offset(), event.row, len) {
                    self.sidebar_selected = index;
                    if self.double_click.click(index) {
                        self.open_selected();
                    }
                }
                None
            }
            MouseEventKind::ScrollDown => {
                let mut selected = self.sidebar_selected;
                vim_list::apply(
                    VimMove::Down,
                    &mut selected,
                    self.sidebar_len(),
                    self.sidebar_visible_height,
                );
                self.sidebar_selected = selected;
                None
            }
            MouseEventKind::ScrollUp => {
                let mut selected = self.sidebar_selected;
                vim_list::apply(
                    VimMove::Up,
                    &mut selected,
                    self.sidebar_len(),
                    self.sidebar_visible_height,
                );
                self.sidebar_selected = selected;
                None
            }
            _ => None,
        }
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
        hints.extend(ui::hint(Context::Kafka, Command::KafkaToggleMode, "mode"));
        hints.extend(ui::hint(Context::Kafka, Command::KafkaRefresh, "refresh"));
        match self.mode {
            KafkaMode::Topics => {
                hints.extend(ui::hint(Context::Kafka, Command::KafkaOpen, "tail"));
                hints.extend(ui::hint(
                    Context::Kafka,
                    Command::KafkaTailEarliest,
                    "from start",
                ));
                hints.extend(ui::hint(Context::Kafka, Command::KafkaPauseFollow, "pause"));
                hints.extend(ui::hint(Context::Kafka, Command::KafkaPublish, "publish"));
            }
            KafkaMode::Groups => {
                hints.extend(ui::hint(Context::Kafka, Command::KafkaOpen, "lag"));
            }
        }
        hints.extend(ui::hint(Context::Kafka, Command::Search, "filter"));
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

    fn draw_once(screen: &mut KafkaScreen) {
        let backend = ratatui::backend::TestBackend::new(60, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[tokio::test]
    async fn clicking_a_topic_in_the_sidebar_selects_it() {
        let mut screen = screen();
        screen.session.topics = vec![
            crate::TopicInfo {
                name: "orders".to_string(),
                partitions: 1,
            },
            crate::TopicInfo {
                name: "payments".to_string(),
                partitions: 1,
            },
        ];
        draw_once(&mut screen);

        // Row 0 is the sidebar's top border, row 2 is the second topic.
        screen.handle_mouse_event(left_click(2, 2));

        assert_eq!(screen.sidebar_selected, 1);
    }

    #[tokio::test]
    async fn double_clicking_a_topic_tails_it() {
        let mut screen = screen();
        screen.session.topics = vec![crate::TopicInfo {
            name: "orders".to_string(),
            partitions: 1,
        }];
        draw_once(&mut screen);

        screen.handle_mouse_event(left_click(2, 1));
        screen.handle_mouse_event(left_click(2, 1));

        assert_eq!(screen.session.tailing_topic.as_deref(), Some("orders"));
    }

    #[tokio::test]
    async fn a_click_outside_the_sidebar_does_nothing() {
        let mut screen = screen();
        screen.session.topics = vec![crate::TopicInfo {
            name: "orders".to_string(),
            partitions: 1,
        }];
        draw_once(&mut screen);

        screen.handle_mouse_event(left_click(50, 5));

        assert_eq!(screen.sidebar_selected, 0);
        assert_eq!(screen.session.tailing_topic, None);
    }

    #[tokio::test]
    async fn scrolling_moves_the_sidebar_selection() {
        let mut screen = screen();
        screen.session.topics = vec![
            crate::TopicInfo {
                name: "orders".to_string(),
                partitions: 1,
            },
            crate::TopicInfo {
                name: "payments".to_string(),
                partitions: 1,
            },
        ];
        draw_once(&mut screen);

        screen.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(screen.sidebar_selected, 1);

        screen.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(screen.sidebar_selected, 0);
    }

    #[tokio::test]
    async fn starts_in_topics_mode() {
        let screen = screen();

        assert_eq!(screen.mode, KafkaMode::Topics);
    }

    #[tokio::test]
    async fn toggle_mode_switches_and_resets_the_cursor_and_filter() {
        let mut screen = screen();
        screen.sidebar_selected = 3;
        screen.filter = "ord".to_string();

        screen.toggle_mode();

        assert_eq!(screen.mode, KafkaMode::Groups);
        assert_eq!(screen.sidebar_selected, 0);
        assert!(screen.filter.is_empty());

        screen.toggle_mode();
        assert_eq!(screen.mode, KafkaMode::Topics);
    }

    #[tokio::test]
    async fn filter_narrows_visible_groups_case_insensitively() {
        let mut screen = screen();
        screen.session.groups = vec![
            crate::ConsumerGroupInfo {
                name: "orders-consumer".to_string(),
                state: "Stable".to_string(),
                members: 2,
            },
            crate::ConsumerGroupInfo {
                name: "payments-consumer".to_string(),
                state: "Stable".to_string(),
                members: 1,
            },
            crate::ConsumerGroupInfo {
                name: "ORDER-REPORTING".to_string(),
                state: "Empty".to_string(),
                members: 0,
            },
        ];

        screen.filter = "order".to_string();
        let names: Vec<&str> = screen
            .visible_groups()
            .into_iter()
            .map(|g| g.name.as_str())
            .collect();
        assert_eq!(names, vec!["orders-consumer", "ORDER-REPORTING"]);
    }

    #[tokio::test]
    async fn show_lag_selected_remembers_the_highlighted_group() {
        let mut screen = screen();
        screen.mode = KafkaMode::Groups;
        screen.session.groups = vec![crate::ConsumerGroupInfo {
            name: "my-group".to_string(),
            state: "Stable".to_string(),
            members: 1,
        }];

        screen.show_lag_selected();

        assert_eq!(screen.selected_group.as_deref(), Some("my-group"));
    }

    #[tokio::test]
    async fn open_selected_tails_the_selected_topic_in_topics_mode() {
        let mut topics_screen = screen();
        topics_screen.session.topics = vec![crate::TopicInfo {
            name: "orders".to_string(),
            partitions: 1,
        }];

        topics_screen.open_selected();

        assert_eq!(
            topics_screen.session.tailing_topic.as_deref(),
            Some("orders")
        );
    }

    #[tokio::test]
    async fn open_selected_shows_lag_for_the_selected_group_in_groups_mode() {
        let mut groups_screen = screen();
        groups_screen.mode = KafkaMode::Groups;
        groups_screen.session.groups = vec![crate::ConsumerGroupInfo {
            name: "my-group".to_string(),
            state: "Stable".to_string(),
            members: 1,
        }];

        groups_screen.open_selected();

        assert_eq!(groups_screen.selected_group.as_deref(), Some("my-group"));
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

    #[tokio::test]
    async fn filter_narrows_visible_topics_case_insensitively() {
        let mut screen = screen();
        screen.session.topics = vec![
            crate::TopicInfo {
                name: "orders".to_string(),
                partitions: 3,
            },
            crate::TopicInfo {
                name: "payments".to_string(),
                partitions: 1,
            },
            crate::TopicInfo {
                name: "ORDER_EVENTS".to_string(),
                partitions: 2,
            },
        ];

        screen.filter = "order".to_string();
        let names: Vec<&str> = screen
            .visible_topics()
            .into_iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(names, vec!["orders", "ORDER_EVENTS"]);
    }

    #[tokio::test]
    async fn esc_clears_the_filter_and_resets_the_cursor() {
        let mut screen = screen();
        screen.session.topics = vec![
            crate::TopicInfo {
                name: "orders".to_string(),
                partitions: 3,
            },
            crate::TopicInfo {
                name: "payments".to_string(),
                partitions: 1,
            },
        ];

        screen.open_filter();
        assert!(screen.is_filtering());
        for c in "pay".chars() {
            screen.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        assert_eq!(screen.filter, "pay");
        assert_eq!(screen.visible_topics().len(), 1);
        screen.sidebar_selected = 5;

        screen.filter_key_event(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!screen.is_filtering());
        assert!(screen.filter.is_empty());
        assert_eq!(screen.visible_topics().len(), 2);
        assert_eq!(screen.sidebar_selected, 0);
    }

    #[tokio::test]
    async fn enter_keeps_the_filter_applied_and_closes_the_bar() {
        let mut screen = screen();
        screen.session.topics = vec![crate::TopicInfo {
            name: "orders".to_string(),
            partitions: 3,
        }];

        screen.open_filter();
        for c in "ord".chars() {
            screen.filter_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.filter_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(!screen.is_filtering());
        assert_eq!(screen.filter, "ord");
    }
}
