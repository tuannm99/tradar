use std::collections::HashMap;
use std::io;
use std::sync::Arc;

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

use tradar_app::components::RootComponent;
use tradar_connector_api::{Connector, Session};
use tradar_core::action::{Action, Component};
use tradar_core::storage::{ConnectionStore, SavedConnection, default_connections_path};

/// Every connector compiled into this binary. Adding a connector means
/// adding a dependency line in `Cargo.toml` and a line here -- nothing else
/// in the workspace needs to change (see "Registry" in
/// docs/architecture.md).
fn registry() -> HashMap<String, Box<dyn Connector>> {
    let connectors: Vec<Box<dyn Connector>> = vec![
        tradar_postgres::connector(),
        tradar_sqlite::connector(),
        tradar_elasticsearch::connector(),
        tradar_redis::connector(),
        tradar_mongo::connector(),
    ];
    connectors
        .into_iter()
        .map(|c| (c.descriptor().id.to_string(), c))
        .collect()
}

/// The result of a connect attempt, carried across a `tokio::spawn`
/// boundary. Deliberately *not* `Action::Opened` itself: a `Screen`
/// (`Box<dyn Component>`) can hold non-`Send` state (`edtui`'s
/// `EditorState` holds an `Rc`-based clipboard), so it must be built with
/// `Session::build_screen` on this single-threaded event loop, never inside
/// a spawned task. `Session` itself is `Send + Sync` and crosses fine.
enum ConnectOutcome {
    Opened {
        connection: SavedConnection,
        session: Box<dyn Session>,
        epoch: u64,
    },
    Failed {
        error: String,
        epoch: u64,
    },
}

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

    let registry = Arc::new(registry());
    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let (connect_tx, connect_rx) = mpsc::unbounded_channel();
    let mut root = RootComponent::new(connections);

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

    let result = run(
        &mut terminal,
        &mut root,
        registry,
        action_tx,
        action_rx,
        connect_tx,
        connect_rx,
    )
    .await;

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
    registry: Arc<HashMap<String, Box<dyn Connector>>>,
    action_tx: mpsc::UnboundedSender<Action>,
    mut action_rx: mpsc::UnboundedReceiver<Action>,
    connect_tx: mpsc::UnboundedSender<ConnectOutcome>,
    mut connect_rx: mpsc::UnboundedReceiver<ConnectOutcome>,
) -> anyhow::Result<()> {
    terminal.draw(|frame| root.draw(frame, frame.area()))?;

    while !root.should_quit {
        if event::poll(std::time::Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(action) = root.handle_key_event(key.code, key.modifiers)
        {
            let _ = action_tx.send(action);
        }

        while let Ok(action) = action_rx.try_recv() {
            match action {
                Action::OpenRequested { connection, epoch } => {
                    spawn_connect(registry.clone(), connect_tx.clone(), connection, epoch);
                }
                other => {
                    if let Some(next) = root.update(other) {
                        let _ = action_tx.send(next);
                    }
                }
            }
        }

        while let Ok(outcome) = connect_rx.try_recv() {
            match outcome {
                ConnectOutcome::Opened {
                    connection,
                    session,
                    epoch,
                } => {
                    let screen = session.build_screen(action_tx.clone());
                    root.update(Action::Opened {
                        connection,
                        screen,
                        epoch,
                    });
                }
                ConnectOutcome::Failed { error, epoch } => {
                    root.update(Action::OpenFailed { error, epoch });
                }
            }
        }

        // Drains whatever the active screen's `Session` has queued up (e.g.
        // a completed query) -- see "Screen không bao giờ làm IO" in
        // docs/architecture.md. Always redrawn afterwards, since a tick can
        // change state with no accompanying key press or channel message.
        root.tick();

        if !root.should_quit {
            terminal.draw(|frame| root.draw(frame, frame.area()))?;
        }
    }
    Ok(())
}

fn spawn_connect(
    registry: Arc<HashMap<String, Box<dyn Connector>>>,
    connect_tx: mpsc::UnboundedSender<ConnectOutcome>,
    connection: SavedConnection,
    epoch: u64,
) {
    tokio::spawn(async move {
        let Some(connector) = registry.get(&connection.driver) else {
            let _ = connect_tx.send(ConnectOutcome::Failed {
                error: format!(
                    "unknown connector '{}': not compiled into this build",
                    connection.driver
                ),
                epoch,
            });
            return;
        };
        match connector.connect(connection.clone()).await {
            Ok(session) => {
                let _ = connect_tx.send(ConnectOutcome::Opened {
                    connection,
                    session,
                    epoch,
                });
            }
            Err(e) => {
                let _ = connect_tx.send(ConnectOutcome::Failed {
                    error: e.to_string(),
                    epoch,
                });
            }
        }
    });
}
