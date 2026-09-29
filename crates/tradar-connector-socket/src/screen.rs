//! `SocketScreen`: the bespoke `Component` a `SocketSession` builds. No
//! sidebar (there's no "topic"/"queue" concept -- the connection itself is
//! the one thing there is to look at): a scrolling transcript on top, one
//! always-focused input line at the bottom. See "Thiết kế UI: HTTP, gRPC,
//! Socket" in docs/architecture.md and
//! `docs/backlog/socket-connector-2026-09-29.md`.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction as LayoutDirection, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use tradar_connector_spi::Session as ConnectorSession;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};

use crate::{Direction, LogEntry, SocketSession};

/// How many bytes of a non-text entry to show before truncating -- a full
/// multi-line `xxd`-style dump (as `docs/architecture.md`'s design
/// describes) would need each entry to expand into several scrollable
/// lines, complicating the scroll-offset math below for a case that's
/// mostly "confirms binary data arrived" rather than something read byte
/// by byte in this grid -- see "Sai khác khi triển khai thật" in
/// `docs/backlog/socket-connector-2026-09-29.md`.
const HEX_PREVIEW_BYTES: usize = 32;

/// One log line as it's actually drawn: `[+12.34s] > hello` /
/// `[+12.34s] < world`. Text when the bytes are valid UTF-8 with nothing
/// but ordinary whitespace control characters in them, a hex preview
/// otherwise -- same "decode when possible, don't lose binary data
/// silently" idea the design calls for, just single-line (see
/// `HEX_PREVIEW_BYTES`'s doc comment for why not a full dump).
fn format_entry(entry: &LogEntry) -> Line<'static> {
    let marker = match entry.direction {
        Direction::Sent => "›",
        Direction::Received => "‹",
    };
    let marker_color = match entry.direction {
        Direction::Sent => theme().accent,
        Direction::Received => theme().text,
    };
    let looks_like_text = std::str::from_utf8(&entry.bytes).is_ok_and(|s| {
        s.chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
    });
    let body = if looks_like_text {
        String::from_utf8_lossy(&entry.bytes).into_owned()
    } else {
        hex_preview(&entry.bytes)
    };
    Line::from(vec![
        Span::styled(
            format!("[{:>7.2}s] ", entry.at.as_secs_f64()),
            Style::default().fg(theme().text_dim),
        ),
        Span::styled(format!("{marker} "), Style::default().fg(marker_color)),
        Span::styled(body, Style::default().fg(theme().text)),
    ])
}

fn hex_preview(bytes: &[u8]) -> String {
    let shown = &bytes[..bytes.len().min(HEX_PREVIEW_BYTES)];
    let mut out: String = shown
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > HEX_PREVIEW_BYTES {
        out.push_str(&format!(" … ({} bytes)", bytes.len()));
    }
    out
}

pub struct SocketScreen {
    session: SocketSession,
    #[allow(dead_code)]
    action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    input: TextInput,
    /// On by default -- matches most line-based text protocols (RESP
    /// inline, SMTP, IRC...), see `Command::SocketToggleAppendNewline`.
    append_newline: bool,
    /// Lines scrolled back from the very latest entry; `0` stays pinned to
    /// the bottom (a live tail), same idea as `HttpScreen::response_scroll`
    /// just counted from the opposite end so new data arriving doesn't
    /// have to renumber anything already on screen.
    scroll_offset: usize,
    visible_height: usize,
    pending: Option<KeyPress>,
}

impl SocketScreen {
    pub(crate) fn new(
        session: SocketSession,
        action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            session,
            action_tx,
            input: TextInput::new(""),
            append_newline: true,
            scroll_offset: 0,
            visible_height: 0,
            pending: None,
        }
    }

    fn send_input(&mut self) {
        let text = self.input.text();
        self.session.send(&text, self.append_newline);
        self.input = TextInput::new("");
        self.scroll_offset = 0;
    }

    fn scroll(&mut self, delta: isize) {
        let max_offset = self
            .session
            .entries
            .len()
            .saturating_sub(self.visible_height);
        self.scroll_offset = self
            .scroll_offset
            .saturating_add_signed(delta)
            .min(max_offset);
    }

    fn draw_log(&mut self, frame: &mut Frame, area: Rect) {
        let title = format!(
            "Socket — {}{}",
            self.session.target,
            if self.session.connected {
                String::new()
            } else {
                format!(
                    " (disconnected{})",
                    self.session
                        .error
                        .as_deref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                )
            }
        );
        let block = ui::panel(&title, false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.visible_height = inner.height as usize;

        let len = self.session.entries.len();
        let max_offset = len.saturating_sub(self.visible_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);
        let end = len.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(self.visible_height);

        let items: Vec<ListItem> = self
            .session
            .entries
            .iter()
            .skip(start)
            .take(end - start)
            .map(|entry| ListItem::new(format_entry(entry)))
            .collect();
        frame.render_stateful_widget(List::new(items), inner, &mut ListState::default());
    }

    fn draw_input(&mut self, frame: &mut Frame, area: Rect) {
        let title = format!(
            "Send{} — enter: send, ctrl-r: reconnect, f2: toggle \\n",
            if self.append_newline { " (+\\n)" } else { "" }
        );
        let block = ui::panel(&title, true);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(Line::from(self.input.spans(true))), inner);
    }
}

impl Component for SocketScreen {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        let command = match keymap().resolve_in(&[Context::Socket], &mut self.pending, key) {
            Resolution::Command(command) => command,
            Resolution::Pending => return None,
            Resolution::None => {
                self.input.handle_key_event(code, modifiers);
                return None;
            }
        };
        match command {
            Command::MoveUp => self.scroll(1),
            Command::MoveDown => self.scroll(-1),
            Command::HalfPageUp => self.scroll((self.visible_height / 2).max(1) as isize),
            Command::HalfPageDown => self.scroll(-((self.visible_height / 2).max(1) as isize)),
            Command::SocketSend => self.send_input(),
            Command::SocketReconnect => self.session.reconnect(),
            Command::SocketToggleAppendNewline => self.append_newline = !self.append_newline,
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
        let rows = Layout::default()
            .direction(LayoutDirection::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);
        self.draw_log(frame, rows[0]);
        self.draw_input(frame, rows[1]);
    }

    fn connection_alive(&self) -> Option<bool> {
        Some(self.session.connected)
    }

    fn status_hints(&self) -> Vec<ui::Hint> {
        let mut hints = Vec::new();
        hints.extend(ui::hint(Context::Socket, Command::SocketSend, "send"));
        hints.extend(ui::hint(
            Context::Socket,
            Command::SocketReconnect,
            "reconnect",
        ));
        hints.extend(ui::hint(
            Context::Socket,
            Command::SocketToggleAppendNewline,
            "toggle \\n",
        ));
        hints.extend(ui::hint(Context::Socket, Command::Back, "back"));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn screen() -> SocketScreen {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let session = SocketSession::new(addr.to_string(), stream);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        SocketScreen::new(session, tx)
    }

    #[tokio::test]
    async fn starts_with_append_newline_on_and_nothing_typed() {
        let screen = screen().await;

        assert!(screen.append_newline);
        assert_eq!(screen.input.text(), "");
    }

    #[tokio::test]
    async fn typing_a_letter_lands_in_the_input_not_as_a_command() {
        let mut screen = screen().await;

        // 'r' would be `SocketReconnect` if bound bare -- it's `ctrl-r`
        // instead precisely so this must land as text.
        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::NONE);

        assert_eq!(screen.input.text(), "r");
    }

    #[tokio::test]
    async fn enter_sends_and_clears_the_input() {
        let mut screen = screen().await;
        for c in "hello".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(screen.input.text(), "");
        assert_eq!(screen.session.entries.len(), 1);
        assert_eq!(screen.session.entries[0].bytes, b"hello\n");
    }

    #[tokio::test]
    async fn f2_toggles_the_append_newline_flag() {
        let mut screen = screen().await;

        screen.handle_key_event(KeyCode::F(2), KeyModifiers::NONE);
        assert!(!screen.append_newline);

        screen.handle_key_event(KeyCode::F(2), KeyModifiers::NONE);
        assert!(screen.append_newline);
    }

    #[tokio::test]
    async fn ctrl_r_reconnects() {
        let mut screen = screen().await;

        screen.handle_key_event(KeyCode::Char('r'), KeyModifiers::CONTROL);

        // Reconnecting drops the old read task -- `connected` flips to
        // `false` until the new dial finishes, proving the key actually
        // triggered `reconnect()` rather than being swallowed as text.
        assert!(!screen.session.connected);
        assert_eq!(screen.input.text(), "", "must not have been typed");
    }

    #[tokio::test]
    async fn esc_backs_out_to_the_picker() {
        let mut screen = screen().await;

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    #[tokio::test]
    async fn up_and_down_scroll_without_touching_the_input() {
        let mut screen = screen().await;
        for i in 0..5 {
            screen
                .session
                .push_entry(Direction::Received, vec![b'0' + i]);
        }
        screen.visible_height = 2;

        screen.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(screen.scroll_offset, 1);
        screen.handle_key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(screen.scroll_offset, 2);
        screen.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(screen.scroll_offset, 1);
        assert_eq!(screen.input.text(), "");
    }

    #[test]
    fn hex_preview_truncates_past_the_cap_and_says_so() {
        let bytes: Vec<u8> = (0..40).collect();

        let preview = hex_preview(&bytes);

        assert!(preview.starts_with("00 01 02"));
        assert!(preview.contains("40 bytes"));
    }

    #[test]
    fn format_entry_shows_binary_data_as_hex_not_mangled_text() {
        let entry = LogEntry {
            direction: Direction::Received,
            bytes: vec![0x00, 0x01, 0xff, 0xfe],
            at: std::time::Duration::from_secs(1),
        };

        let line = format_entry(&entry);
        let text: String = line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("");

        assert!(text.contains("00 01 ff fe"));
    }
}
