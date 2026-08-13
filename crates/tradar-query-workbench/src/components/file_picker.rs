//! The open-a-query overlay: recently used files first, then whatever else
//! is in the queries directory, filtered as you type.
//!
//! Typing filters rather than replacing the list, and a filter that matches
//! nothing is still openable as a literal path -- so one widget serves both
//! "pick the file I had open yesterday" and "open this exact path", without
//! a mode to switch between them.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use tradar_core::keymap::{Command, Context, KeyPress, Resolution, keymap};
use tradar_core::theme::theme;
use tradar_core::ui::{self, TextInput};
use tradar_core::vim_list;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Chosen(PathBuf),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    /// What's shown: the bare file name for anything in the queries
    /// directory, the full path for a file from somewhere else, since the
    /// name alone wouldn't say which file it is.
    label: String,
    recent: bool,
}

pub struct FilePickerComponent {
    entries: Vec<Entry>,
    filter: TextInput,
    selected: usize,
    pending: Option<KeyPress>,
    state: ListState,
    visible_height: usize,
}

impl FilePickerComponent {
    /// `recent` is most-recent-first; `queries_dir` is listed after it.
    /// Files that no longer exist are dropped from the recent list rather
    /// than offered and then failing to open.
    pub fn new(recent: &[String], queries_dir: &Path) -> Self {
        let mut entries: Vec<Entry> = Vec::new();
        for path in recent {
            let path = PathBuf::from(path);
            if path.exists() {
                entries.push(Entry {
                    label: label_for(&path, queries_dir),
                    path,
                    recent: true,
                });
            }
        }
        if let Ok(read_dir) = std::fs::read_dir(queries_dir) {
            let mut from_dir: Vec<PathBuf> = read_dir
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| !entries.iter().any(|entry| entry.path == *p))
                .collect();
            from_dir.sort();
            entries.extend(from_dir.into_iter().map(|path| Entry {
                label: label_for(&path, queries_dir),
                path,
                recent: false,
            }));
        }
        Self {
            entries,
            filter: TextInput::new(""),
            selected: 0,
            pending: None,
            state: ListState::default(),
            visible_height: 0,
        }
    }

    fn matches(&self) -> Vec<&Entry> {
        let needle = self.filter.text().to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| needle.is_empty() || entry.label.to_ascii_lowercase().contains(&needle))
            .collect()
    }

    fn chosen(&self) -> Option<PathBuf> {
        match self.matches().get(self.selected) {
            Some(entry) => Some(entry.path.clone()),
            // Nothing matched, so treat what was typed as a path -- that's
            // the escape hatch for a file outside the queries directory.
            None => {
                let typed = self.filter.text();
                (!typed.trim().is_empty()).then(|| PathBuf::from(typed.trim()))
            }
        }
    }

    pub fn handle_key_event(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<PickerOutcome> {
        let key = KeyPress::new(code, modifiers);
        // Confirm/cancel and list movement come from the keymap; anything
        // else is typing into the filter. Only the movement keys that
        // aren't plain characters are taken, or you couldn't type a name
        // containing `j`.
        if let Resolution::Command(command) =
            keymap().resolve_in(&[Context::Prompt], &mut self.pending, key)
        {
            match command {
                Command::Cancel => return Some(PickerOutcome::Cancelled),
                Command::Confirm => return self.chosen().map(PickerOutcome::Chosen),
                _ => {}
            }
        }
        let movement = match code {
            KeyCode::Down => Some(vim_list::VimMove::Down),
            KeyCode::Up => Some(vim_list::VimMove::Up),
            _ => None,
        };
        if let Some(mv) = movement {
            let len = self.matches().len();
            vim_list::apply(mv, &mut self.selected, len, self.visible_height);
            return None;
        }

        if self.filter.handle_key_event(code, modifiers) {
            // The list shrinks as you type, so keep the selection inside it.
            self.selected = self.selected.min(self.matches().len().saturating_sub(1));
        }
        None
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let theme = theme();
        let matches: Vec<Entry> = self.matches().into_iter().cloned().collect();

        let block = ui::panel(
            "Open query — type to filter, enter opens, esc cancels",
            true,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);
        self.visible_height = chunks[1].height as usize;

        let mut filter_line = vec![Span::styled("> ", Style::default().fg(theme.accent))];
        filter_line.extend(self.filter.spans(true));
        frame.render_widget(Paragraph::new(Line::from(filter_line)), chunks[0]);

        if matches.is_empty() {
            let hint = if self.filter.is_empty() {
                "no saved queries yet — save one with ctrl-s".to_string()
            } else {
                format!("no match — enter opens '{}' as a path", self.filter.text())
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(theme.text_dim))),
                chunks[1],
            );
            return;
        }

        let items: Vec<ListItem> = matches
            .iter()
            .map(|entry| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if entry.recent { " ● " } else { "   " }.to_string(),
                        Style::default().fg(theme.accent),
                    ),
                    Span::styled(entry.label.clone(), Style::default().fg(theme.text)),
                ]))
            })
            .collect();
        self.state
            .select(Some(self.selected.min(matches.len() - 1)));
        frame.render_stateful_widget(
            List::new(items).highlight_style(ui::selection_style()),
            chunks[1],
            &mut self.state,
        );
    }
}

fn label_for(path: &Path, queries_dir: &Path) -> String {
    match path.strip_prefix(queries_dir) {
        Ok(relative) => relative.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn dir_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in files {
            std::fs::write(dir.path().join(name), "select 1").unwrap();
        }
        dir
    }

    fn type_str(picker: &mut FilePickerComponent, text: &str) {
        for c in text.chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[test]
    fn lists_the_queries_directory_by_bare_name() {
        let dir = dir_with(&["a.sql", "b.sql"]);

        let picker = FilePickerComponent::new(&[], dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["a.sql", "b.sql"]);
    }

    #[test]
    fn recent_files_come_first_and_are_not_listed_twice() {
        let dir = dir_with(&["a.sql", "b.sql"]);
        let recent = vec![dir.path().join("b.sql").to_string_lossy().to_string()];

        let picker = FilePickerComponent::new(&recent, dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["b.sql", "a.sql"]);
        assert!(picker.entries[0].recent);
        assert!(!picker.entries[1].recent);
    }

    #[test]
    fn a_recent_file_that_no_longer_exists_is_dropped() {
        let dir = dir_with(&["a.sql"]);
        let recent = vec!["/nowhere/gone.sql".to_string()];

        let picker = FilePickerComponent::new(&recent, dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["a.sql"], "offering it would just fail to open");
    }

    #[test]
    fn a_file_outside_the_queries_directory_is_shown_by_full_path() {
        let dir = dir_with(&[]);
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("other.sql");
        std::fs::write(&outside, "select 1").unwrap();
        let recent = vec![outside.to_string_lossy().to_string()];

        let picker = FilePickerComponent::new(&recent, dir.path());

        assert_eq!(picker.entries[0].label, outside.to_string_lossy());
    }

    #[test]
    fn typing_filters_the_list() {
        let dir = dir_with(&["orders.sql", "users.sql"]);
        let mut picker = FilePickerComponent::new(&[], dir.path());

        type_str(&mut picker, "ord");

        let labels: Vec<&str> = picker.matches().iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["orders.sql"]);
    }

    #[test]
    fn enter_opens_the_selected_file() {
        let dir = dir_with(&["a.sql", "b.sql"]);
        let mut picker = FilePickerComponent::new(&[], dir.path());

        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(PickerOutcome::Chosen(dir.path().join("b.sql")))
        );
    }

    #[test]
    fn a_filter_matching_nothing_is_opened_as_a_path() {
        let dir = dir_with(&["a.sql"]);
        let mut picker = FilePickerComponent::new(&[], dir.path());

        type_str(&mut picker, "/tmp/elsewhere.sql");
        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(PickerOutcome::Chosen(PathBuf::from("/tmp/elsewhere.sql"))),
            "the typed path is the escape hatch for files outside the directory"
        );
    }

    #[test]
    fn esc_cancels() {
        let dir = dir_with(&["a.sql"]);
        let mut picker = FilePickerComponent::new(&[], dir.path());

        assert_eq!(
            picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE),
            Some(PickerOutcome::Cancelled)
        );
    }

    #[test]
    fn filtering_keeps_the_selection_inside_the_shortened_list() {
        let dir = dir_with(&["a.sql", "b.sql", "orders.sql"]);
        let mut picker = FilePickerComponent::new(&[], dir.path());
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected, 2);

        type_str(&mut picker, "ord");

        assert_eq!(picker.selected, 0, "one match left, so it must be selected");
        assert_eq!(
            picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE),
            Some(PickerOutcome::Chosen(dir.path().join("orders.sql")))
        );
    }

    #[test]
    fn draw_says_so_when_there_is_nothing_saved_yet() {
        let dir = dir_with(&[]);
        let mut picker = FilePickerComponent::new(&[], dir.path());
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
            .unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("no saved queries yet"), "buffer was: {text}");
    }
}
