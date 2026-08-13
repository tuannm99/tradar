use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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
use tradar_core::config;
use tradar_core::storage::{
    ConnectionStore, SavedConnection, SessionState, SessionStore, default_connections_path,
    default_session_path,
};

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
/// boundary. Deliberately *not* `Action::Opened` itself: `Component` isn't
/// bound to `Send`, so a `Screen` (`Box<dyn Component>`) must be built with
/// `Session::build_screen` on this single-threaded event loop, never inside
/// a spawned task. `Session` itself is `Send + Sync` and crosses fine.
enum ConnectOutcome {
    Opened {
        connection: SavedConnection,
        session: Box<dyn Session>,
        epoch: u64,
        tab: usize,
    },
    Failed {
        error: String,
        epoch: u64,
        tab: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Theme and key bindings, before anything is drawn. A broken config is
    // reported and skipped rather than being fatal: losing your colors
    // shouldn't stop you reaching your database.
    let config_path = config::default_config_path()?;
    if let Err(e) = config::init(&config_path) {
        eprintln!(
            "warning: ignoring {}: {e}\n         (using built-in theme and key bindings)",
            config_path.display()
        );
    }

    let store = ConnectionStore::at(default_connections_path()?);
    let connections = store.load()?;

    let session_store = SessionStore::at(default_session_path()?);
    let session_state = session_store.load().unwrap_or_default();

    let registry = Arc::new(registry());
    // The picker's driver field offers exactly what this build can connect
    // to, sorted so the list doesn't shuffle between runs (the registry is
    // a HashMap).
    let mut drivers: Vec<String> = registry.keys().cloned().collect();
    drivers.sort();

    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let (connect_tx, connect_rx) = mpsc::unbounded_channel();
    let mut root = RootComponent::new(connections).with_editing(drivers, store);

    // Reconnect whatever tabs were open (and connected) when the app last
    // quit, same as a user hand-picking each one from the picker again.
    for action in root.restore_tabs(&session_state) {
        if let Action::OpenRequested {
            connection,
            epoch,
            tab,
        } = action
        {
            spawn_connect(registry.clone(), connect_tx.clone(), connection, epoch, tab);
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    // Best-effort -- a failure to persist the session (e.g. an unwritable
    // config dir) must never mask the app's actual exit result.
    let _ = session_store.save(&session_state_of(&root));

    result
}

/// Which tabs were `Active` (connected) when the app is quitting, and which
/// of those was focused -- the shape `SessionStore::save` persists so the
/// next run can reconnect them. Tabs still sitting on the picker have
/// nothing worth remembering.
fn session_state_of(root: &RootComponent) -> SessionState {
    let mut tabs = Vec::new();
    let mut active_tab = None;
    for (i, tab) in root.tabs.iter().enumerate() {
        let Some(name) = &tab.title else { continue };
        if i == root.active_tab {
            active_tab = Some(tabs.len());
        }
        tabs.push(name.clone());
    }
    SessionState {
        active_tab: active_tab.unwrap_or(0),
        tabs,
    }
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
        // Only a key press, a channel message, or a tick that reports a
        // real change is worth a redraw -- otherwise `terminal.draw` would
        // re-diff and (mostly no-op) repaint the whole widget tree up to
        // 20 times a second even while the screen is sitting perfectly
        // still.
        let mut dirty = false;

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    dirty = true;
                    if let Some(action) = root.handle_key_event(key.code, key.modifiers) {
                        let _ = action_tx.send(action);
                    }
                }
                Event::Mouse(mouse) => {
                    dirty = true;
                    if let Some(action) = root.handle_mouse_event(mouse) {
                        let _ = action_tx.send(action);
                    }
                }
                // Resize and the key-release/repeat kinds still need a
                // redraw, but carry nothing to act on.
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }

        while let Ok(action) = action_rx.try_recv() {
            dirty = true;
            match action {
                Action::OpenRequested {
                    connection,
                    epoch,
                    tab,
                } => {
                    spawn_connect(registry.clone(), connect_tx.clone(), connection, epoch, tab);
                }
                other => {
                    if let Some(next) = root.update(other) {
                        let _ = action_tx.send(next);
                    }
                }
            }
        }

        while let Ok(outcome) = connect_rx.try_recv() {
            dirty = true;
            match outcome {
                ConnectOutcome::Opened {
                    connection,
                    session,
                    epoch,
                    tab,
                } => {
                    let screen = session.build_screen(action_tx.clone());
                    root.update(Action::Opened {
                        connection,
                        screen,
                        epoch,
                        tab,
                    });
                }
                ConnectOutcome::Failed { error, epoch, tab } => {
                    root.update(Action::OpenFailed { error, epoch, tab });
                }
            }
        }

        // Drains whatever the active screen's `Session` has queued up (e.g.
        // a completed query) -- see "Screen không bao giờ làm IO" in
        // docs/architecture.md.
        if root.tick() {
            dirty = true;
        }

        if dirty && !root.should_quit {
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
    tab: usize,
) {
    tokio::spawn(async move {
        let Some(connector) = registry.get(&connection.driver) else {
            let _ = connect_tx.send(ConnectOutcome::Failed {
                error: format!(
                    "unknown connector '{}': not compiled into this build",
                    connection.driver
                ),
                epoch,
                tab,
            });
            return;
        };
        match connector.connect(connection.clone()).await {
            Ok(session) => {
                let _ = connect_tx.send(ConnectOutcome::Opened {
                    connection,
                    session,
                    epoch,
                    tab,
                });
            }
            Err(e) => {
                let _ = connect_tx.send(ConnectOutcome::Failed {
                    error: e.to_string(),
                    epoch,
                    tab,
                });
            }
        }
    });
}
