use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
use tradar_core::storage;
use tradar_core::storage::{
    ConnectionStore, SavedConnection, SessionState, SessionStore, TabState,
    default_connections_path, default_session_path,
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
        tradar_cassandra::connector(),
        tradar_rabbitmq::connector(),
        tradar_kafka::connector(),
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

    // Where ctrl-s writes and ctrl-o looks. Failing to work out the path
    // (no home directory, say) just means the query screen falls back to
    // asking for a full path.
    if let (Ok(queries_dir), Ok(recent_path)) = (
        storage::default_queries_dir(),
        storage::default_recent_path(),
    ) {
        storage::init_query_files(queries_dir, storage::RecentStore::at(recent_path));
    }

    // The saved-snippet library (`Ctrl+K`/`Ctrl+L`). Same fallback as
    // above: no config directory just means those keys have nothing to
    // save into, not a reason to fail startup.
    if let Ok(snippets_path) = storage::default_snippets_path() {
        storage::init_snippets(storage::SnippetStore::at(snippets_path));
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

    // What each restored tab had in its editor last time, by tab index --
    // handed to `build_screen` when that tab's connect resolves.
    let restore_by_tab: HashMap<usize, String> = session_state
        .tabs
        .iter()
        .enumerate()
        .filter(|(_, tab)| !tab.query.is_empty())
        .map(|(index, tab)| (index, tab.query.clone()))
        .collect();

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
        Wiring {
            registry,
            action_tx,
            action_rx,
            connect_tx,
            connect_rx,
            restore_by_tab,
        },
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
        // Whatever the screen wants back next run -- the query text, for a
        // query screen.
        let query = match &tab.screen {
            tradar_app::components::ScreenSlot::Active(screen) => {
                screen.restore_state().unwrap_or_default()
            }
            tradar_app::components::ScreenSlot::ConnectionPicker => String::new(),
        };
        tabs.push(TabState {
            connection: name.clone(),
            query,
        });
    }
    SessionState {
        active_tab: active_tab.unwrap_or(0),
        tabs,
    }
}

/// Everything the event loop needs besides the terminal and the component
/// tree: the connector registry, the two channels it pumps, and the
/// per-tab text a restored session should reopen with.
struct Wiring {
    registry: Arc<HashMap<String, Box<dyn Connector>>>,
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,
    connect_tx: mpsc::UnboundedSender<ConnectOutcome>,
    connect_rx: mpsc::UnboundedReceiver<ConnectOutcome>,
    restore_by_tab: HashMap<usize, String>,
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    root: &mut RootComponent,
    wiring: Wiring,
) -> anyhow::Result<()> {
    let Wiring {
        registry,
        action_tx,
        mut action_rx,
        connect_tx,
        mut connect_rx,
        restore_by_tab,
    } = wiring;
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
                // Some terminals report every pointer motion once mouse
                // capture is on (SGR "any-event" mode), not just clicks and
                // scrolls -- without this filter, dragging the mouse a long
                // diagonal across the grid queues a redraw per motion event
                // `handle_mouse_event` was going to ignore anyway (its match
                // only reacts to `Down(Left)`/`ScrollDown`/`ScrollUp`), and
                // `event::read` only drains one event per loop iteration, so
                // the whole burst has to be drawn away before the actual
                // click lands. Distance-dependent lag: short move, few
                // motion events; a click on a distant cell, many.
                Event::Mouse(mouse)
                    if matches!(
                        mouse.kind,
                        MouseEventKind::Down(MouseButton::Left)
                            | MouseEventKind::ScrollDown
                            | MouseEventKind::ScrollUp
                    ) =>
                {
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
                    let screen = session.build_screen(
                        action_tx.clone(),
                        restore_by_tab.get(&tab).map(String::as_str),
                    );
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
