use std::io;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use tradar::action::{Action, Component};
use tradar::components::RootComponent;
use tradar::drivers::Driver;
use tradar::drivers::elasticsearch::{self, ElasticsearchDriver};
use tradar::drivers::mongo::MongoDriver;
use tradar::drivers::postgres::PostgresDriver;
use tradar::drivers::redis::RedisDriver;
use tradar::drivers::sqlite::SqliteDriver;
use tradar::query_engine::QueryEngine;
use tradar::storage::{ConnectionStore, DriverKind, SavedConnection, default_connections_path};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let connections_path = default_connections_path()?;
    let store = ConnectionStore::at(connections_path.clone());
    let connections = store.load()?;

    if connections.is_empty() {
        println!(
            "No saved connections found. Add one to {} and re-run tradar.\n\
             (There's no interactive \"add connection\" screen yet -- see \
             docs/architecture.md.)",
            connections_path.display()
        );
        return Ok(());
    }

    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let mut root = RootComponent::new(connections, action_tx.clone());

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

    let result = run(&mut terminal, &mut root, action_tx, action_rx).await;

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
    root: &mut RootComponent,
    action_tx: mpsc::UnboundedSender<Action>,
    mut action_rx: mpsc::UnboundedReceiver<Action>,
) -> anyhow::Result<()> {
    terminal.draw(|frame| root.draw(frame, frame.area()))?;

    while !root.should_quit {
        let mut dirty = false;

        if event::poll(std::time::Duration::from_millis(50))? {
            dirty = true;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && let Some(action) = root.handle_key_event(key.code, key.modifiers)
            {
                let _ = action_tx.send(action);
            }
        }

        while let Ok(action) = action_rx.try_recv() {
            dirty = true;
            match action {
                Action::ConnectRequested { connection, epoch } => {
                    spawn_connect(action_tx.clone(), connection, epoch);
                }
                Action::ExportCurl { connection, query } => {
                    export_curl(&connection, &query);
                }
                other => {
                    if let Some(next) = root.update(other) {
                        let _ = action_tx.send(next);
                    }
                }
            }
        }

        if dirty && !root.should_quit {
            terminal.draw(|frame| root.draw(frame, frame.area()))?;
        }
    }
    Ok(())
}

fn spawn_connect(
    action_tx: mpsc::UnboundedSender<Action>,
    connection: SavedConnection,
    epoch: u64,
) {
    tokio::spawn(async move {
        let mut driver: Box<dyn Driver> = match connection.driver {
            DriverKind::Sqlite => Box::new(SqliteDriver::new(&connection.target)),
            DriverKind::Postgres => Box::new(PostgresDriver::new(&connection.target)),
            DriverKind::Elasticsearch => Box::new(ElasticsearchDriver::new(&connection.target)),
            DriverKind::Redis => Box::new(RedisDriver::new(&connection.target)),
            DriverKind::Mongo => Box::new(MongoDriver::new(&connection.target)),
        };
        match driver.connect().await {
            Ok(()) => {
                let engine = QueryEngine::new(driver);
                let schema = engine.list_schema().await.map_err(|e| e.to_string());
                let _ = action_tx.send(Action::Connected {
                    connection,
                    engine,
                    schema,
                    epoch,
                });
            }
            Err(e) => {
                let _ = action_tx.send(Action::ConnectFailed {
                    error: e.to_string(),
                    epoch,
                });
            }
        }
    });
}

fn export_curl(connection: &SavedConnection, query: &str) {
    if connection.driver != DriverKind::Elasticsearch {
        return;
    }
    let Some(curl) = elasticsearch::to_curl(&connection.target, query) else {
        return;
    };
    let script = format!("#!/usr/bin/env bash\n{curl}\n");
    let _ = std::fs::write("./tradar-query.sh", script);
}
