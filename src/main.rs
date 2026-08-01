use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use tradar::app::{App, Screen};
use tradar::drivers::Driver;
use tradar::drivers::postgres::PostgresDriver;
use tradar::drivers::sqlite::SqliteDriver;
use tradar::query_engine::QueryEngine;
use tradar::storage::{ConnectionStore, DriverKind, default_connections_path};
use tradar::tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let connections_path = default_connections_path()?;
    let store = ConnectionStore::at(connections_path.clone());
    let connections = store.load()?;

    if connections.is_empty() {
        println!(
            "No saved connections found. Add one to {} and re-run tradar.\n\
             (There's no interactive \"add connection\" screen yet -- see \
             docs/superpowers/specs/2026-08-01-tradar-v1-design.md.)",
            connections_path.display()
        );
        return Ok(());
    }

    let mut app = App::new(connections);
    let mut engine: Option<QueryEngine> = None;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let keyboard_enhancement = supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhancement {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut app, &mut engine).await;

    if keyboard_enhancement {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    engine: &mut Option<QueryEngine>,
) -> anyhow::Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| tui::draw(frame, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            handle_key(app, engine, key.code, key.modifiers).await?;
        }
    }
    Ok(())
}

fn is_submit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::F(5)) || (code == KeyCode::Enter && modifiers.contains(KeyModifiers::CONTROL))
}

async fn handle_key(
    app: &mut App,
    engine: &mut Option<QueryEngine>,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> anyhow::Result<()> {
    match app.screen {
        Screen::ConnectionPicker => match code {
            KeyCode::Char('q') => app.quit(),
            KeyCode::Down | KeyCode::Char('j') => app.move_selection_down(),
            KeyCode::Up | KeyCode::Char('k') => app.move_selection_up(),
            KeyCode::Enter => connect_to_selected(app, engine).await,
            _ => {}
        },
        Screen::Query => match code {
            KeyCode::Esc => {
                app.back_to_picker();
                *engine = None;
            }
            _ if is_submit(code, modifiers) => run_query(app, engine).await,
            KeyCode::Enter => app.push_char('\n'),
            KeyCode::Backspace => app.backspace(),
            KeyCode::Char(c) => app.push_char(c),
            _ => {}
        },
    }
    Ok(())
}

async fn connect_to_selected(app: &mut App, engine: &mut Option<QueryEngine>) {
    let Some(connection) = app.connections.get(app.selected).cloned() else {
        return;
    };
    let mut driver: Box<dyn Driver> = match connection.driver {
        DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
        DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
    };
    match driver.connect().await {
        Ok(()) => {
            app.connect_to_selected();
            *engine = Some(QueryEngine::new(driver));
        }
        Err(e) => app.set_error(e.to_string()),
    }
}

async fn run_query(app: &mut App, engine: &mut Option<QueryEngine>) {
    let Some(engine) = engine.as_mut() else {
        return;
    };
    let query = app.query_input.clone();
    match engine.run(&query).await {
        Ok(result) => app.set_result(result),
        Err(e) => app.set_error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_enter_submits() {
        assert!(is_submit(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    #[test]
    fn plain_enter_does_not_submit() {
        assert!(!is_submit(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn f5_submits_regardless_of_modifiers() {
        assert!(is_submit(KeyCode::F(5), KeyModifiers::NONE));
    }

    #[test]
    fn plain_characters_do_not_submit() {
        assert!(!is_submit(KeyCode::Char('a'), KeyModifiers::NONE));
    }
}
