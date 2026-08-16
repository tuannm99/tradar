//! `HttpScreen`: the bespoke Postman-style `Component` an `HttpSession`
//! builds -- method + URL + headers + body on top, response below. See
//! "Thiết kế UI: HTTP, gRPC, Socket" in docs/architecture.md for the design
//! this implements.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};

use tradar_connector_spi::Session as ConnectorSession;
use tradar_core::action::{Action, Component};
use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::storage::SavedHttpRequest;
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextArea, TextInput};
use tradar_core::vim_list;

use crate::HttpSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    const ALL: [Self; 7] = [
        Self::Get,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Head,
        Self::Options,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    fn from_str(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(s))
            .unwrap_or(Self::Get)
    }

    fn cycle(self, delta: isize) -> Self {
        let idx = Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as isize;
        let len = Self::ALL.len() as isize;
        let next = (idx + delta).rem_euclid(len);
        Self::ALL[next as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Url,
    Headers,
    Body,
    Response,
}

const FOCUS_ORDER: [Focus; 4] = [Focus::Url, Focus::Headers, Focus::Body, Focus::Response];

struct RequestPicker {
    entries: Vec<SavedHttpRequest>,
    selected: usize,
    visible_height: usize,
}

pub struct HttpScreen {
    session: HttpSession,
    #[allow(dead_code)]
    action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    method: HttpMethod,
    url: TextInput,
    headers: TextArea,
    body: TextArea,
    focus: Focus,
    response_scroll: usize,
    response_visible_height: usize,
    name_prompt: Option<TextInput>,
    request_picker: Option<RequestPicker>,
    pending: Option<KeyPress>,
    /// The request-builder/response split -- stacked or side by side, and
    /// how much space each pane gets. Same widget and same `F6`/
    /// `Ctrl+Up`/`Ctrl+Down` bindings as the query screen's editor/results
    /// split -- see `tradar_core::ui::SplitPane`.
    split: ui::SplitPane,
    /// Where each field/pane was last drawn, so a click there can focus it
    /// or hit-test a right-click -- same idea as `QueryScreenComponent`'s
    /// `editor_area`.
    url_area: Rect,
    headers_area: Rect,
    body_area: Rect,
    response_area: Rect,
    /// The full area this screen was last drawn into -- context-menu
    /// clicks are hit-tested against the exact bounds it was drawn with.
    screen_area: Rect,
    /// Open after a right-click on the response pane.
    context_menu: Option<ui::ContextMenu>,
}

impl HttpScreen {
    pub(crate) fn new(
        session: HttpSession,
        action_tx: tokio::sync::mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            session,
            action_tx,
            method: HttpMethod::Get,
            url: TextInput::new(""),
            headers: TextArea::new(""),
            body: TextArea::new(""),
            focus: Focus::Url,
            response_scroll: 0,
            response_visible_height: 0,
            name_prompt: None,
            request_picker: None,
            pending: None,
            split: ui::SplitPane::default(),
            url_area: Rect::ZERO,
            headers_area: Rect::ZERO,
            body_area: Rect::ZERO,
            response_area: Rect::ZERO,
            screen_area: Rect::ZERO,
            context_menu: None,
        }
    }

    fn cycle_focus(&mut self, delta: isize) {
        let idx = FOCUS_ORDER
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0) as isize;
        let len = FOCUS_ORDER.len() as isize;
        let next = (idx + delta).rem_euclid(len);
        self.focus = FOCUS_ORDER[next as usize];
    }

    fn send(&mut self) {
        self.session.send(
            self.method.as_str(),
            &self.url.text(),
            &self.headers.text(),
            &self.body.text(),
        );
    }

    fn response_line_count(&self) -> usize {
        self.session
            .response
            .as_ref()
            .map(|r| r.body.lines().count())
            .unwrap_or(0)
    }

    fn open_save_prompt(&mut self) {
        self.name_prompt = Some(TextInput::new(""));
    }

    fn save_request(&mut self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(store) = tradar_core::storage::http_requests() {
            store.save(SavedHttpRequest {
                name,
                method: self.method.as_str().to_string(),
                url: self.url.text(),
                headers: self.headers.text(),
                body: self.body.text(),
            });
        }
    }

    fn open_request_picker(&mut self) {
        let entries = tradar_core::storage::http_requests()
            .map(|s| s.all())
            .unwrap_or_default();
        self.request_picker = Some(RequestPicker {
            entries,
            selected: 0,
            visible_height: 0,
        });
    }

    fn load_selected_request(&mut self) {
        let Some(picker) = self.request_picker.take() else {
            return;
        };
        let Some(entry) = picker.entries.get(picker.selected) else {
            return;
        };
        self.method = HttpMethod::from_str(&entry.method);
        self.url = TextInput::new(&entry.url);
        self.headers.set_text(&entry.headers);
        self.body.set_text(&entry.body);
    }

    fn delete_selected_request(&mut self) {
        let Some(picker) = self.request_picker.as_mut() else {
            return;
        };
        let Some(entry) = picker.entries.get(picker.selected).cloned() else {
            return;
        };
        if let Some(store) = tradar_core::storage::http_requests() {
            store.delete(&entry.name);
        }
        picker.entries.remove(picker.selected);
        if picker.selected >= picker.entries.len() {
            picker.selected = picker.entries.len().saturating_sub(1);
        }
    }

    fn handle_save_prompt_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        match keymap().resolve(Context::Prompt, &mut self.pending, key) {
            Resolution::Command(Command::Confirm) => {
                if let Some(prompt) = self.name_prompt.take() {
                    self.save_request(prompt.text());
                }
            }
            Resolution::Command(Command::Cancel) => self.name_prompt = None,
            Resolution::Pending => {}
            _ => {
                if let Some(prompt) = self.name_prompt.as_mut() {
                    prompt.handle_key_event(code, modifiers);
                }
            }
        }
        None
    }

    fn handle_request_picker_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<Action> {
        let key = KeyPress::new(code, modifiers);
        let resolution = keymap().resolve_in(
            &[Context::HttpRequests, Context::List],
            &mut self.pending,
            key,
        );
        let command = match resolution {
            Resolution::Command(command) => command,
            _ => return None,
        };
        if let Some(mv) = command.as_vim_move() {
            if let Some(picker) = self.request_picker.as_mut() {
                let len = picker.entries.len();
                let visible = picker.visible_height;
                vim_list::apply(mv, &mut picker.selected, len, visible);
            }
            return None;
        }
        match command {
            Command::Confirm => self.load_selected_request(),
            Command::Cancel => self.request_picker = None,
            Command::HttpDeleteRequest => self.delete_selected_request(),
            _ => {}
        }
        None
    }

    fn forward_to_field(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        match self.focus {
            Focus::Url => {
                self.url.handle_key_event(code, modifiers);
            }
            Focus::Headers => {
                self.headers.handle_key_event(code, modifiers);
            }
            Focus::Body => {
                self.body.handle_key_event(code, modifiers);
            }
            Focus::Response => {}
        }
        None
    }

    fn yank_response(&self) {
        if let Some(response) = &self.session.response {
            ui::yank_to_clipboard(&response.body);
        }
    }

    /// Runs whatever `command` means for this screen -- shared by keyboard
    /// dispatch and a right-click context menu's confirmed choice, so a
    /// menu item runs through the exact same code a keyboard shortcut for
    /// it would.
    fn dispatch_command(&mut self, command: Command) -> Option<Action> {
        match command {
            Command::NextField => self.cycle_focus(1),
            Command::PrevField => self.cycle_focus(-1),
            Command::HttpSend => self.send(),
            Command::HttpNextMethod => self.method = self.method.cycle(1),
            Command::HttpPrevMethod => self.method = self.method.cycle(-1),
            Command::HttpSaveRequest => self.open_save_prompt(),
            Command::HttpOpenRequests => self.open_request_picker(),
            Command::Yank if self.focus == Focus::Response => self.yank_response(),
            Command::ToggleSplitOrientation => self.split.toggle_orientation(),
            Command::ZoomIn => self.split.zoom_in(self.focus != Focus::Response),
            Command::ZoomOut => self.split.zoom_out(self.focus != Focus::Response),
            Command::Help => return Some(Action::ShowHelp),
            Command::Back => return Some(Action::BackToPicker),
            _ => {}
        }
        None
    }

    fn draw_method_url(&mut self, frame: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(11), Constraint::Min(10)])
            .split(area);

        let method_block = ui::panel("Method", false);
        let inner = method_block.inner(cols[0]);
        frame.render_widget(method_block, cols[0]);
        frame.render_widget(
            Paragraph::new(self.method.as_str()).style(
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            inner,
        );

        let url_focused = self.focus == Focus::Url;
        let url_block = ui::panel("URL", url_focused);
        let inner = url_block.inner(cols[1]);
        frame.render_widget(url_block, cols[1]);
        frame.render_widget(
            Paragraph::new(Line::from(self.url.spans(url_focused))),
            inner,
        );
    }

    fn draw_headers(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Headers;
        let block = ui::panel("Headers (Key: Value, one per line)", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let scroll = self.headers.scroll_offset(inner.height as usize);
        frame.render_widget(
            Paragraph::new(self.headers.styled_lines(focused)).scroll((scroll, 0)),
            inner,
        );
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Body;
        let block = ui::panel("Body", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let scroll = self.body.scroll_offset(inner.height as usize);
        frame.render_widget(
            Paragraph::new(self.body.styled_lines(focused)).scroll((scroll, 0)),
            inner,
        );
    }

    fn draw_response(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Response;
        let title = match (
            &self.session.error,
            self.session.sending,
            &self.session.response,
        ) {
            (Some(error), _, _) => format!("Response — error: {error}"),
            (None, true, _) => "Response — sending…".to_string(),
            (None, false, Some(response)) => format!(
                "Response — {} {} · {} headers · {}ms",
                response.status,
                response.status_text,
                response.headers.len(),
                response.elapsed_ms
            ),
            (None, false, None) => "Response".to_string(),
        };
        let block = ui::panel(&title, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.response_visible_height = inner.height as usize;

        let Some(response) = &self.session.response else {
            let placeholder = Paragraph::new(if self.session.sending {
                "sending…"
            } else {
                "ctrl-enter / f5 to send"
            })
            .style(Style::default().fg(theme().text_dim))
            .wrap(Wrap { trim: true });
            frame.render_widget(placeholder, inner);
            return;
        };

        let lines: Vec<&str> = response.body.lines().collect();
        let len = lines.len();
        self.response_scroll = self.response_scroll.min(len.saturating_sub(1));
        let items: Vec<ListItem> = lines
            .into_iter()
            .skip(self.response_scroll)
            .take(inner.height as usize)
            .map(ListItem::new)
            .collect();
        frame.render_stateful_widget(List::new(items), inner, &mut ListState::default());
    }

    fn draw_save_prompt(&self, frame: &mut Frame, area: Rect) {
        let Some(prompt) = &self.name_prompt else {
            return;
        };
        let popup = ui::centered_rect(50, 15, area);
        frame.render_widget(Clear, popup);
        let block = ui::panel("Save request as — enter: confirm, esc: cancel", true);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        frame.render_widget(Paragraph::new(Line::from(prompt.spans(true))), inner);
    }

    fn draw_request_picker(&mut self, frame: &mut Frame, area: Rect) {
        let Some(picker) = self.request_picker.as_mut() else {
            return;
        };
        let popup = ui::centered_rect(60, 50, area);
        frame.render_widget(Clear, popup);
        let block = ui::panel("Saved requests — enter: load, d: delete, esc: close", true);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        picker.visible_height = inner.height as usize;

        let items: Vec<ListItem> = picker
            .entries
            .iter()
            .map(|r| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {:<7}", r.method),
                        Style::default().fg(theme().accent),
                    ),
                    Span::styled(format!("{}  ", r.name), Style::default().fg(theme().text)),
                    Span::styled(r.url.clone(), Style::default().fg(theme().text_dim)),
                ]))
            })
            .collect();

        let mut state = ListState::default();
        if !picker.entries.is_empty() {
            state.select(Some(picker.selected));
        }
        let list = List::new(items).highlight_style(ui::selection_style());
        frame.render_stateful_widget(list, inner, &mut state);
    }
}

impl Component for HttpScreen {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        if let Some(menu) = self.context_menu.as_mut() {
            match menu.handle_key_event(code) {
                ui::ContextMenuOutcome::Open => {}
                ui::ContextMenuOutcome::Closed => self.context_menu = None,
                ui::ContextMenuOutcome::Confirmed(command) => {
                    self.context_menu = None;
                    return self.dispatch_command(command);
                }
            }
            return None;
        }
        if self.request_picker.is_some() {
            return self.handle_request_picker_key(code, modifiers);
        }
        if self.name_prompt.is_some() {
            return self.handle_save_prompt_key(code, modifiers);
        }

        let contexts: &[Context] = match self.focus {
            Focus::Response => &[Context::Http, Context::HttpResponse, Context::List],
            _ => &[Context::Http],
        };
        let key = KeyPress::new(code, modifiers);
        let command = match keymap().resolve_in(contexts, &mut self.pending, key) {
            Resolution::Command(command) => command,
            Resolution::Pending => return None,
            Resolution::None => return self.forward_to_field(code, modifiers),
        };
        if let Some(mv) = command.as_vim_move() {
            if self.focus == Focus::Response {
                let len = self.response_line_count();
                let visible = self.response_visible_height;
                vim_list::apply(mv, &mut self.response_scroll, len, visible);
            }
            return None;
        }
        self.dispatch_command(command)
    }

    fn handle_mouse_event(&mut self, event: MouseEvent) -> Option<Action> {
        if self.request_picker.is_some() || self.name_prompt.is_some() {
            return None;
        }

        if let Some(menu) = self.context_menu.take() {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(command) = menu.click(self.screen_area, event.column, event.row)
            {
                return self.dispatch_command(command);
            }
            return None;
        }

        let point = (event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if ui::contains(self.url_area, point.0, point.1) {
                    self.focus = Focus::Url;
                } else if ui::contains(self.headers_area, point.0, point.1) {
                    self.focus = Focus::Headers;
                } else if ui::contains(self.body_area, point.0, point.1) {
                    self.focus = Focus::Body;
                } else if ui::contains(self.response_area, point.0, point.1) {
                    self.focus = Focus::Response;
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if ui::contains(self.response_area, point.0, point.1)
                    && self.session.response.is_some()
                {
                    self.focus = Focus::Response;
                    self.context_menu = Some(ui::ContextMenu::new(
                        point,
                        vec![("Yank body".to_string(), Command::Yank)],
                    ));
                }
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                // X11 middle-click-pastes convention -- whichever field is
                // under the click gets focused (same as a left click
                // would), and gets the paste when a clipboard is actually
                // available (headless/SSH with no display forwarding
                // commonly has none -- focusing still happens, the paste
                // just quietly doesn't).
                let text = ui::paste_from_clipboard();
                if ui::contains(self.url_area, point.0, point.1) {
                    self.focus = Focus::Url;
                    if let Some(text) = &text {
                        self.url.insert_str(text);
                    }
                } else if ui::contains(self.headers_area, point.0, point.1) {
                    self.focus = Focus::Headers;
                    if let Some(text) = &text {
                        self.headers.insert_str(text);
                    }
                } else if ui::contains(self.body_area, point.0, point.1) {
                    self.focus = Focus::Body;
                    if let Some(text) = &text {
                        self.body.insert_str(text);
                    }
                }
            }
            MouseEventKind::ScrollDown if ui::contains(self.response_area, point.0, point.1) => {
                let len = self.response_line_count();
                vim_list::apply(
                    vim_list::VimMove::Down,
                    &mut self.response_scroll,
                    len,
                    self.response_visible_height,
                );
            }
            MouseEventKind::ScrollUp if ui::contains(self.response_area, point.0, point.1) => {
                let len = self.response_line_count();
                vim_list::apply(
                    vim_list::VimMove::Up,
                    &mut self.response_scroll,
                    len,
                    self.response_visible_height,
                );
            }
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
        self.screen_area = area;
        // Stacked or side by side, and how much of the split the request
        // builder gets, per `self.split` -- same `F6`/`Ctrl+Up`/
        // `Ctrl+Down` bindings as the query screen's editor/results split.
        let (form_area, response_area) = self.split.split(area);
        let form = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Min(3),
            ])
            .split(form_area);

        self.url_area = form[0];
        self.headers_area = form[1];
        self.body_area = form[2];
        self.response_area = response_area;

        self.draw_method_url(frame, form[0]);
        self.draw_headers(frame, form[1]);
        self.draw_body(frame, form[2]);
        self.draw_response(frame, response_area);

        self.draw_save_prompt(frame, area);
        self.draw_request_picker(frame, area);

        if let Some(menu) = &self.context_menu {
            menu.draw(frame, area);
        }
    }

    fn connection_alive(&self) -> Option<bool> {
        Some(self.session.error.is_none())
    }

    fn status_hints(&self) -> Vec<ui::Hint> {
        let mut hints = Vec::new();
        hints.extend(ui::hint(Context::Http, Command::NextField, "field"));
        hints.extend(ui::hint(Context::Http, Command::HttpSend, "send"));
        hints.extend(ui::hint(Context::Http, Command::HttpNextMethod, "method"));
        hints.extend(ui::hint(Context::Http, Command::HttpSaveRequest, "save"));
        hints.extend(ui::hint(Context::Http, Command::HttpOpenRequests, "open"));
        hints.extend(ui::hint(Context::Http, Command::Back, "back"));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> HttpScreen {
        let session = HttpSession::new(
            reqwest::Client::new(),
            "https://api.example.com".to_string(),
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        HttpScreen::new(session, tx)
    }

    #[test]
    fn starts_focused_on_the_url_field_with_method_get() {
        let screen = screen();

        assert_eq!(screen.focus, Focus::Url);
        assert_eq!(screen.method, HttpMethod::Get);
    }

    #[test]
    fn tab_cycles_focus_forward_through_all_four_panes_and_wraps() {
        let mut screen = screen();

        for expected in [Focus::Headers, Focus::Body, Focus::Response, Focus::Url] {
            screen.handle_key_event(KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(screen.focus, expected);
        }
    }

    #[test]
    fn backtab_cycles_focus_backward() {
        let mut screen = screen();

        screen.handle_key_event(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(screen.focus, Focus::Response);
    }

    #[test]
    fn typing_a_letter_while_url_is_focused_is_text_not_a_command() {
        let mut screen = screen();

        // 'y' is bound to Yank in Context::HttpResponse, but that context
        // is only offered when Response has focus -- typed here, it must
        // land in the URL field instead of being swallowed as a command.
        screen.handle_key_event(KeyCode::Char('y'), KeyModifiers::NONE);

        assert_eq!(screen.url.text(), "y");
    }

    #[test]
    fn ctrl_left_and_ctrl_right_cycle_the_method_regardless_of_focus() {
        let mut screen = screen();

        screen.handle_key_event(KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(screen.method, HttpMethod::Post);

        screen.handle_key_event(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(screen.method, HttpMethod::Get);
    }

    #[test]
    fn plain_left_right_in_the_url_field_move_the_cursor_not_the_method() {
        let mut screen = screen();
        for c in "abc".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }

        screen.handle_key_event(KeyCode::Left, KeyModifiers::NONE);
        screen.handle_key_event(KeyCode::Char('X'), KeyModifiers::NONE);

        assert_eq!(screen.url.text(), "abXc");
        assert_eq!(screen.method, HttpMethod::Get, "method must be untouched");
    }

    #[test]
    fn ctrl_k_opens_a_save_prompt_and_enter_closes_it() {
        // Deliberately doesn't touch `tradar_core::storage`'s process-global
        // `HttpRequests` singleton -- it's a `OnceLock`, so a second test in
        // this binary calling `init_http_requests` would silently no-op and
        // leave two tests sharing one store. `save_request` already
        // no-ops gracefully when the store was never initialized (`if let
        // Some(store) = ...`), which is exactly the state under test here;
        // the persistence round trip itself is covered directly in
        // `tradar-core`'s own `storage` tests.
        let mut screen = screen();

        screen.handle_key_event(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert!(screen.name_prompt.is_some());
        for c in "list users".chars() {
            screen.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.name_prompt.is_none());
    }

    #[test]
    fn ctrl_l_opens_an_empty_picker_when_nothing_is_saved() {
        let mut screen = screen();

        screen.handle_key_event(KeyCode::Char('l'), KeyModifiers::CONTROL);

        assert!(screen.request_picker.is_some());
    }

    #[test]
    fn enter_in_the_request_picker_loads_the_selected_entry_into_the_fields() {
        let mut screen = screen();
        screen.request_picker = Some(RequestPicker {
            entries: vec![SavedHttpRequest {
                name: "list users".to_string(),
                method: "POST".to_string(),
                url: "/users".to_string(),
                headers: "Accept: json".to_string(),
                body: "{}".to_string(),
            }],
            selected: 0,
            visible_height: 0,
        });

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.request_picker.is_none());
        assert_eq!(screen.method, HttpMethod::Post);
        assert_eq!(screen.url.text(), "/users");
        assert_eq!(screen.headers.text(), "Accept: json");
        assert_eq!(screen.body.text(), "{}");
    }

    #[test]
    fn d_in_the_request_picker_removes_the_selected_entry_from_the_list() {
        let mut screen = screen();
        screen.request_picker = Some(RequestPicker {
            entries: vec![
                SavedHttpRequest {
                    name: "a".to_string(),
                    method: "GET".to_string(),
                    url: "/a".to_string(),
                    headers: String::new(),
                    body: String::new(),
                },
                SavedHttpRequest {
                    name: "b".to_string(),
                    method: "GET".to_string(),
                    url: "/b".to_string(),
                    headers: String::new(),
                    body: String::new(),
                },
            ],
            selected: 0,
            visible_height: 0,
        });

        screen.handle_key_event(KeyCode::Char('d'), KeyModifiers::NONE);

        let picker = screen.request_picker.as_ref().unwrap();
        assert_eq!(picker.entries.len(), 1);
        assert_eq!(picker.entries[0].name, "b");
    }

    #[test]
    fn esc_backs_out_of_the_screen_when_no_overlay_is_open() {
        let mut screen = screen();

        let action = screen.handle_key_event(KeyCode::Esc, KeyModifiers::NONE);

        assert!(matches!(action, Some(Action::BackToPicker)));
    }

    fn drawn(screen: &mut HttpScreen) {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| screen.draw(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn f6_toggles_the_split_between_stacked_and_side_by_side() {
        let mut screen = screen();
        drawn(&mut screen);
        let stacked_response_area = screen.response_area;

        screen.handle_key_event(KeyCode::F(6), KeyModifiers::NONE);
        drawn(&mut screen);

        assert_eq!(stacked_response_area.width, 80, "stacked: full width");
        assert!(
            screen.response_area.width < 80,
            "side by side: response is now only part of the width"
        );
    }

    #[test]
    fn ctrl_up_grows_the_focused_response_pane() {
        let mut screen = screen();
        screen.focus = Focus::Response;
        drawn(&mut screen);
        let before = screen.response_area.height;

        screen.handle_key_event(KeyCode::Up, KeyModifiers::CONTROL);
        drawn(&mut screen);

        assert!(
            screen.response_area.height > before,
            "zooming in on the focused response pane should grow it"
        );
    }

    #[test]
    fn left_click_focuses_the_field_under_the_cursor() {
        let mut screen = screen();
        drawn(&mut screen);
        let point = (screen.headers_area.x + 1, screen.headers_area.y + 1);

        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(screen.focus, Focus::Headers);
    }

    #[test]
    fn right_click_on_the_response_pane_opens_a_yank_menu_only_when_there_is_a_response() {
        let mut screen = screen();
        drawn(&mut screen);
        let point = (screen.response_area.x + 1, screen.response_area.y + 1);

        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::NONE,
        });
        assert!(
            screen.context_menu.is_none(),
            "nothing to yank yet -- no menu"
        );

        screen.session.response = Some(crate::HttpResponseData {
            status: 200,
            status_text: "OK".to_string(),
            headers: vec![],
            body: "{}".to_string(),
            elapsed_ms: 1,
        });
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::NONE,
        });

        assert!(screen.context_menu.is_some());
        assert_eq!(screen.focus, Focus::Response);
    }

    #[test]
    fn confirming_the_yank_menu_item_closes_it() {
        let mut screen = screen();
        drawn(&mut screen);
        screen.session.response = Some(crate::HttpResponseData {
            status: 200,
            status_text: "OK".to_string(),
            headers: vec![],
            body: "{}".to_string(),
            elapsed_ms: 1,
        });
        let point = (screen.response_area.x + 1, screen.response_area.y + 1);
        screen.handle_mouse_event(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::NONE,
        });
        assert!(screen.context_menu.is_some());

        screen.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert!(screen.context_menu.is_none());
    }

    #[test]
    fn http_method_cycle_wraps_around_in_both_directions() {
        assert_eq!(HttpMethod::Options.cycle(1), HttpMethod::Get);
        assert_eq!(HttpMethod::Get.cycle(-1), HttpMethod::Options);
    }

    #[test]
    fn http_method_from_str_is_case_insensitive_and_falls_back_to_get() {
        assert_eq!(HttpMethod::from_str("post"), HttpMethod::Post);
        assert_eq!(HttpMethod::from_str("nonsense"), HttpMethod::Get);
    }
}
