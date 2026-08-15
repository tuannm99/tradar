//! The open-a-query overlay: a real directory browser scoped to the
//! queries directory and its subfolders, recently used files shown first
//! (at the root only -- they're absolute paths, not tied to one folder),
//! filtered as you type.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// Shown only while browsing `queries_dir` itself, marked `●`.
    Recent,
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    /// What's shown: for `Recent`, the path relative to the queries
    /// directory (or the full path for a file from somewhere else) since
    /// it could be nested anywhere; for `Dir`/`File`, just the bare name
    /// -- the breadcrumb above the list already says which folder they're
    /// in.
    label: String,
    kind: EntryKind,
}

pub struct FilePickerComponent {
    queries_dir: PathBuf,
    current_dir: PathBuf,
    /// Kept around (not just consumed in `new()`) so returning to the root
    /// via `Backspace` can rebuild the recent-files section without the
    /// caller having to hand it over again.
    recent: Vec<String>,
    entries: Vec<Entry>,
    filter: TextInput,
    selected: usize,
    pending: Option<KeyPress>,
    state: ListState,
    visible_height: usize,
}

impl FilePickerComponent {
    /// `recent` is most-recent-first. `start_dir` is where the picker
    /// opens -- normally the folder of the most recently saved/opened
    /// file (see `QueryScreenComponent::open_file_picker`), falling back
    /// to `queries_dir` itself when there's no recent file yet.
    pub fn new(recent: &[String], queries_dir: &Path, start_dir: &Path) -> Self {
        let mut picker = Self {
            queries_dir: queries_dir.to_path_buf(),
            current_dir: start_dir.to_path_buf(),
            recent: recent.to_vec(),
            entries: Vec::new(),
            filter: TextInput::new(""),
            selected: 0,
            pending: None,
            state: ListState::default(),
            visible_height: 0,
        };
        picker.list_dir();
        picker
    }

    /// Rebuilds `entries` for `current_dir`: recent files first (root
    /// only), then subdirectories, then files, each group sorted by name.
    /// Files that no longer exist are dropped from the recent list rather
    /// than offered and then failing to open. Also resets the filter and
    /// selection -- a stale filter from the folder you just left wouldn't
    /// mean anything in this one.
    fn list_dir(&mut self) {
        let mut entries = Vec::new();
        if self.current_dir == self.queries_dir {
            for path in &self.recent {
                let path = PathBuf::from(path);
                if path.exists() {
                    entries.push(Entry {
                        label: label_for(&path, &self.queries_dir),
                        path,
                        kind: EntryKind::Recent,
                    });
                }
            }
        }
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            for dir_entry in read_dir.filter_map(Result::ok) {
                let path = dir_entry.path();
                if entries.iter().any(|entry| entry.path == path) {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(path);
                } else if path.is_file() {
                    files.push(path);
                }
            }
        }
        dirs.sort();
        files.sort();
        entries.extend(dirs.into_iter().map(|path| Entry {
            label: file_name(&path),
            path,
            kind: EntryKind::Dir,
        }));
        entries.extend(files.into_iter().map(|path| Entry {
            label: file_name(&path),
            path,
            kind: EntryKind::File,
        }));
        self.entries = entries;
        self.selected = 0;
        self.filter = TextInput::new("");
    }

    fn matches(&self) -> Vec<&Entry> {
        let needle = self.filter.text().to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| needle.is_empty() || entry.label.to_ascii_lowercase().contains(&needle))
            .collect()
    }

    /// `Enter`'s meaning depends on what's selected: a directory is
    /// navigated into (the picker stays open), a file is chosen, and no
    /// match at all falls back to the typed text as a literal path -- the
    /// escape hatch for anything outside `queries_dir`.
    fn confirm_selected(&mut self) -> Option<PickerOutcome> {
        match self.matches().get(self.selected).map(|e| (*e).clone()) {
            Some(entry) if entry.kind == EntryKind::Dir => {
                self.enter_dir(entry.path);
                None
            }
            Some(entry) => Some(PickerOutcome::Chosen(entry.path)),
            None => {
                let typed = self.filter.text();
                (!typed.trim().is_empty())
                    .then(|| PickerOutcome::Chosen(PathBuf::from(typed.trim())))
            }
        }
    }

    fn enter_dir(&mut self, path: PathBuf) {
        self.current_dir = path;
        self.list_dir();
    }

    /// `Backspace` on an empty filter: goes up one level. A no-op at
    /// `queries_dir` itself -- browsing stays scoped to under it; a typed
    /// path with a separator is still the way to reach anywhere else.
    fn up_dir(&mut self) {
        if self.current_dir == self.queries_dir {
            return;
        }
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.list_dir();
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
                Command::Confirm => return self.confirm_selected(),
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

        // An empty filter has nothing left to erase, so `Backspace` means
        // "go up a level" instead -- same key most file pickers use for
        // "out of this folder" once there's no text to delete.
        if code == KeyCode::Backspace && self.filter.is_empty() {
            self.up_dir();
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
            "Open query — type to filter, enter opens/enters, esc cancels",
            true,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);
        self.visible_height = chunks[2].height as usize;

        frame.render_widget(
            Paragraph::new(Span::styled(
                self.breadcrumb(),
                Style::default().fg(theme.text_dim),
            )),
            chunks[0],
        );

        let mut filter_line = vec![Span::styled("> ", Style::default().fg(theme.accent))];
        filter_line.extend(self.filter.spans(true));
        frame.render_widget(Paragraph::new(Line::from(filter_line)), chunks[1]);

        if matches.is_empty() {
            let hint = if self.filter.is_empty() {
                "no saved queries yet — save one with ctrl-s".to_string()
            } else {
                format!("no match — enter opens '{}' as a path", self.filter.text())
            };
            frame.render_widget(
                Paragraph::new(Span::styled(hint, Style::default().fg(theme.text_dim))),
                chunks[2],
            );
            return;
        }

        let items: Vec<ListItem> = matches
            .iter()
            .map(|entry| {
                let marker = match entry.kind {
                    EntryKind::Recent => " ● ",
                    EntryKind::Dir | EntryKind::File => "   ",
                };
                let label = match entry.kind {
                    EntryKind::Dir => format!("{}/", entry.label),
                    EntryKind::Recent | EntryKind::File => entry.label.clone(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker.to_string(), Style::default().fg(theme.accent)),
                    Span::styled(label, Style::default().fg(theme.text)),
                ]))
            })
            .collect();
        self.state
            .select(Some(self.selected.min(matches.len() - 1)));
        frame.render_stateful_widget(
            List::new(items).highlight_style(ui::selection_style()),
            chunks[2],
            &mut self.state,
        );
    }

    /// `current_dir` relative to `queries_dir`, root shown as `/`.
    fn breadcrumb(&self) -> String {
        match self.current_dir.strip_prefix(&self.queries_dir) {
            Ok(relative) if relative.as_os_str().is_empty() => "/".to_string(),
            Ok(relative) => format!("/{}/", relative.display()),
            Err(_) => self.current_dir.display().to_string(),
        }
    }
}

fn label_for(path: &Path, queries_dir: &Path) -> String {
    match path.strip_prefix(queries_dir) {
        Ok(relative) => relative.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
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

    fn picker_at_root(recent: &[String], dir: &Path) -> FilePickerComponent {
        FilePickerComponent::new(recent, dir, dir)
    }

    fn type_str(picker: &mut FilePickerComponent, text: &str) {
        for c in text.chars() {
            picker.handle_key_event(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    fn draw_text(picker: &mut FilePickerComponent) -> String {
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area()))
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
    fn lists_the_queries_directory_by_bare_name() {
        let dir = dir_with(&["a.sql", "b.sql"]);

        let picker = picker_at_root(&[], dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["a.sql", "b.sql"]);
    }

    #[test]
    fn recent_files_come_first_and_are_not_listed_twice() {
        let dir = dir_with(&["a.sql", "b.sql"]);
        let recent = vec![dir.path().join("b.sql").to_string_lossy().to_string()];

        let picker = picker_at_root(&recent, dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["b.sql", "a.sql"]);
        assert_eq!(picker.entries[0].kind, EntryKind::Recent);
        assert_eq!(picker.entries[1].kind, EntryKind::File);
    }

    #[test]
    fn a_recent_file_that_no_longer_exists_is_dropped() {
        let dir = dir_with(&["a.sql"]);
        let recent = vec!["/nowhere/gone.sql".to_string()];

        let picker = picker_at_root(&recent, dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["a.sql"], "offering it would just fail to open");
    }

    #[test]
    fn a_recent_file_outside_the_queries_directory_is_shown_by_full_path() {
        let dir = dir_with(&[]);
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("other.sql");
        std::fs::write(&outside, "select 1").unwrap();
        let recent = vec![outside.to_string_lossy().to_string()];

        let picker = picker_at_root(&recent, dir.path());

        assert_eq!(picker.entries[0].label, outside.to_string_lossy());
    }

    #[test]
    fn a_subdirectory_is_listed_before_files_and_marked_with_a_trailing_slash() {
        let dir = dir_with(&["a.sql"]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        let mut picker = picker_at_root(&[], dir.path());

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["reports", "a.sql"]);
        assert_eq!(picker.entries[0].kind, EntryKind::Dir);
        assert!(draw_text(&mut picker).contains("reports/"));
    }

    #[test]
    fn recent_files_are_only_offered_at_the_root() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/x.sql"), "select 1").unwrap();
        let recent = vec![dir.path().join("sub/x.sql").to_string_lossy().to_string()];
        let mut picker = picker_at_root(&recent, dir.path());
        // Entries are recent-first ("sub/x.sql"), then the "sub" directory
        // -- move past the recent entry to select the directory itself.
        picker.handle_key_event(KeyCode::Down, KeyModifiers::NONE);

        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE); // into "sub"

        assert!(
            picker.entries.iter().all(|e| e.kind != EntryKind::Recent),
            "recent belongs to the root view, not every folder"
        );
    }

    #[test]
    fn enter_on_a_directory_navigates_into_it_without_closing_the_picker() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        std::fs::write(dir.path().join("reports/q1.sql"), "select 1").unwrap();
        let mut picker = picker_at_root(&[], dir.path());

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(outcome, None, "the picker stays open");
        assert_eq!(picker.current_dir, dir.path().join("reports"));
        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["q1.sql"]);
    }

    #[test]
    fn enter_on_a_file_inside_a_subdirectory_chooses_its_full_path() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        std::fs::write(dir.path().join("reports/q1.sql"), "select 1").unwrap();
        let mut picker = picker_at_root(&[], dir.path());
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE); // into "reports"

        let outcome = picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(
            outcome,
            Some(PickerOutcome::Chosen(dir.path().join("reports/q1.sql")))
        );
    }

    #[test]
    fn backspace_on_an_empty_filter_goes_up_a_level() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        let mut picker = picker_at_root(&[], dir.path());
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE); // into "reports"
        assert_eq!(picker.current_dir, dir.path().join("reports"));

        picker.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(picker.current_dir, dir.path());
    }

    #[test]
    fn backspace_at_the_root_is_a_no_op() {
        let dir = dir_with(&["a.sql"]);
        let mut picker = picker_at_root(&[], dir.path());

        picker.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(picker.current_dir, dir.path());
    }

    #[test]
    fn backspace_with_filter_text_erases_a_character_instead_of_navigating() {
        let dir = dir_with(&["orders.sql"]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        let mut picker = picker_at_root(&[], dir.path());
        type_str(&mut picker, "ord");

        picker.handle_key_event(KeyCode::Backspace, KeyModifiers::NONE);

        assert_eq!(picker.filter.text(), "or");
        assert_eq!(picker.current_dir, dir.path(), "still at the root");
    }

    #[test]
    fn typing_filters_the_current_directory_only() {
        let dir = dir_with(&["orders.sql", "users.sql"]);
        let mut picker = picker_at_root(&[], dir.path());

        type_str(&mut picker, "ord");

        let labels: Vec<&str> = picker.matches().iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["orders.sql"]);
    }

    #[test]
    fn enter_opens_the_selected_file() {
        let dir = dir_with(&["a.sql", "b.sql"]);
        let mut picker = picker_at_root(&[], dir.path());

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
        let mut picker = picker_at_root(&[], dir.path());

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
        let mut picker = picker_at_root(&[], dir.path());

        assert_eq!(
            picker.handle_key_event(KeyCode::Esc, KeyModifiers::NONE),
            Some(PickerOutcome::Cancelled)
        );
    }

    #[test]
    fn filtering_keeps_the_selection_inside_the_shortened_list() {
        let dir = dir_with(&["a.sql", "b.sql", "orders.sql"]);
        let mut picker = picker_at_root(&[], dir.path());
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
    fn starts_in_the_given_start_dir_not_always_the_root() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        std::fs::write(dir.path().join("reports/q1.sql"), "select 1").unwrap();

        let picker = FilePickerComponent::new(&[], dir.path(), &dir.path().join("reports"));

        let labels: Vec<&str> = picker.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["q1.sql"]);
    }

    #[test]
    fn draw_shows_the_breadcrumb_for_the_current_directory() {
        let dir = dir_with(&[]);
        std::fs::create_dir(dir.path().join("reports")).unwrap();
        let mut picker = picker_at_root(&[], dir.path());
        picker.handle_key_event(KeyCode::Enter, KeyModifiers::NONE); // into "reports"

        let text = draw_text(&mut picker);

        assert!(text.contains("/reports/"), "buffer was: {text}");
    }

    #[test]
    fn draw_says_so_when_there_is_nothing_saved_yet() {
        let dir = dir_with(&[]);
        let mut picker = picker_at_root(&[], dir.path());

        let text = draw_text(&mut picker);

        assert!(text.contains("no saved queries yet"), "buffer was: {text}");
    }
}
