//! Socket connector -- raw TCP, netcat-style: a live connection stays open
//! (not one request/response then close), lines/bytes are sent and
//! received freely in either direction. Like `tradar-connector-kafka`,
//! does not implement `QueryDriver`/reuse `tradar-query-workbench` (no
//! query language) and needs a background task pushing data in real time
//! through a bounded-per-tick channel, same "Screen không bao giờ làm IO"
//! shape. See "Thiết kế UI: HTTP, gRPC, Socket" in docs/architecture.md
//! for the design this implements, and
//! `docs/backlog/socket-connector-2026-09-29.md` for where the real
//! implementation deviated from it.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

mod screen;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

pub(crate) use screen::SocketScreen;

/// Bounded per `tick()` call -- see the module doc comment. Matches
/// `tradar-connector-kafka`'s own bound exactly, for the same reason: a
/// busy connection could otherwise starve rendering.
const MAX_DRAIN_PER_TICK: usize = 64;

/// How many of the most recent log entries `SocketSession` keeps. Older
/// ones are dropped -- same reasoning as Kafka's `MAX_BUFFERED_MESSAGES`,
/// just a smaller cap: an entry here can be much larger than one Kafka
/// message (a single `read()` can return several KB).
const MAX_BUFFERED_ENTRIES: usize = 500;

/// One read from the socket can return at most this many bytes -- large
/// enough that a chatty line-based protocol never gets split across
/// several log entries in practice, small enough to bound one entry's
/// memory.
const READ_CHUNK_SIZE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Sent,
    Received,
}

#[derive(Debug, Clone)]
pub(crate) struct LogEntry {
    pub direction: Direction,
    pub bytes: Vec<u8>,
    /// Elapsed time since this session first connected, not wall-clock --
    /// avoids pulling in a calendar/timezone-aware crate (`chrono`) just
    /// for a display timestamp; "+12.34s" is exactly as useful for reading
    /// a live transcript back as an absolute time would be.
    pub at: Duration,
}

pub(crate) enum SocketEvent {
    Data(Direction, Vec<u8>),
    Reconnected,
    Disconnected(String),
    SendFailed(String),
}

/// Reads from `read_half` until it closes or errors, pushing every chunk
/// (and the eventual disconnect reason) through `tx`. Shared by the
/// initial connect and `reconnect()` -- both need exactly this loop.
fn spawn_reader(mut read_half: OwnedReadHalf, tx: UnboundedSender<SocketEvent>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0u8; READ_CHUNK_SIZE];
        loop {
            match read_half.read(&mut buf).await {
                Ok(0) => {
                    let _ = tx.send(SocketEvent::Disconnected(
                        "connection closed by peer".to_string(),
                    ));
                    return;
                }
                Ok(n) => {
                    if tx
                        .send(SocketEvent::Data(Direction::Received, buf[..n].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(SocketEvent::Disconnected(e.to_string()));
                    return;
                }
            }
        }
    })
}

pub struct SocketSession {
    pub(crate) target: String,
    connect_start: Instant,
    /// Shared with every in-flight `send()` task -- a plain field would
    /// need `&mut self` for the write itself, but `send()` hands the write
    /// off to a spawned task (same "don't block key handling on IO" rule
    /// as `KafkaSession::publish`) rather than awaiting it inline, so two
    /// sends issued close together need to serialize against each other
    /// through something `Clone`-able instead. `None` only replaces this
    /// during the brief window inside `reconnect()`, matching
    /// `KafkaSession::tail_handle`'s "not connected right now" shape.
    write_half: Arc<Mutex<Option<OwnedWriteHalf>>>,
    event_tx: UnboundedSender<SocketEvent>,
    event_rx: UnboundedReceiver<SocketEvent>,
    read_handle: Option<JoinHandle<()>>,
    pub(crate) entries: VecDeque<LogEntry>,
    pub(crate) connected: bool,
    pub(crate) error: Option<String>,
}

impl SocketSession {
    fn new(target: String, stream: TcpStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let read_handle = spawn_reader(read_half, event_tx.clone());
        Self {
            target,
            connect_start: Instant::now(),
            write_half: Arc::new(Mutex::new(Some(write_half))),
            event_tx,
            event_rx,
            read_handle: Some(read_handle),
            entries: VecDeque::new(),
            connected: true,
            error: None,
        }
    }

    pub(crate) fn push_entry(&mut self, direction: Direction, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        self.entries.push_back(LogEntry {
            direction,
            bytes,
            at: self.connect_start.elapsed(),
        });
        while self.entries.len() > MAX_BUFFERED_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// `text` plus a trailing `\n` when `append_newline` is set -- the
    /// screen owns that toggle, this just does what it's told. Sent bytes
    /// are logged immediately (optimistic -- matches the buffer echoing
    /// its own input the moment `Enter` is pressed, not once the write
    /// actually lands); a failed write shows up as `self.error` on the
    /// next `tick()` instead of un-echoing the entry, since by then it may
    /// already be half-written and there's no clean way to say "that
    /// didn't happen" about bytes that may partially have.
    pub(crate) fn send(&mut self, text: &str, append_newline: bool) {
        let mut payload = text.as_bytes().to_vec();
        if append_newline {
            payload.push(b'\n');
        }
        if payload.is_empty() {
            return;
        }
        self.push_entry(Direction::Sent, payload.clone());

        let write_half = Arc::clone(&self.write_half);
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut guard = write_half.lock().await;
            let result = match guard.as_mut() {
                Some(stream) => stream.write_all(&payload).await,
                None => {
                    let _ = tx.send(SocketEvent::SendFailed("not connected".to_string()));
                    return;
                }
            };
            if let Err(e) = result {
                let _ = tx.send(SocketEvent::SendFailed(e.to_string()));
            }
        });
    }

    /// Drops the current connection (if any) and dials `self.target` again
    /// from scratch -- no retry loop, no backoff, same "the user asked,
    /// try once" shape as every other explicit reconnect in this app (none
    /// of the DB connectors auto-retry a dropped connection either).
    pub(crate) fn reconnect(&mut self) {
        if let Some(handle) = self.read_handle.take() {
            handle.abort();
        }
        self.connected = false;
        self.error = None;

        let target = self.target.clone();
        let write_half_slot = Arc::clone(&self.write_half);
        let tx = self.event_tx.clone();
        self.read_handle = Some(tokio::spawn(async move {
            let stream = match TcpStream::connect(&target).await {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = tx.send(SocketEvent::Disconnected(e.to_string()));
                    return;
                }
            };
            let (read_half, write_half) = stream.into_split();
            *write_half_slot.lock().await = Some(write_half);
            let _ = tx.send(SocketEvent::Reconnected);

            let mut buf = [0u8; READ_CHUNK_SIZE];
            let mut read_half = read_half;
            loop {
                match read_half.read(&mut buf).await {
                    Ok(0) => {
                        let _ = tx.send(SocketEvent::Disconnected(
                            "connection closed by peer".to_string(),
                        ));
                        return;
                    }
                    Ok(n) => {
                        if tx
                            .send(SocketEvent::Data(Direction::Received, buf[..n].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(SocketEvent::Disconnected(e.to_string()));
                        return;
                    }
                }
            }
        }));
    }
}

impl Session for SocketSession {
    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            let event = match self.event_rx.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            changed = true;
            match event {
                SocketEvent::Data(direction, bytes) => self.push_entry(direction, bytes),
                SocketEvent::Reconnected => {
                    self.connected = true;
                    self.error = None;
                    self.connect_start = Instant::now();
                }
                SocketEvent::Disconnected(error) => {
                    self.connected = false;
                    self.error = Some(error);
                }
                SocketEvent::SendFailed(error) => self.error = Some(error),
            }
        }
        changed
    }

    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        _restore: Option<&str>,
    ) -> Box<dyn Component> {
        Box::new(SocketScreen::new(*self, action_tx))
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "socket",
    display_name: "Socket",
    icon: "🔌",
    capabilities: &[Capability::Streaming],
};

struct SocketConnector;

#[async_trait]
impl Connector for SocketConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let target = connection.target.clone();
        // The TCP handshake itself is the liveness proof here -- unlike
        // Kafka/Postgres/etc, there's no separate "are you really there"
        // round trip to make: a plain socket has no protocol of its own to
        // speak yet.
        let stream = tradar_connector_spi::with_connect_timeout(&target, async {
            TcpStream::connect(&target)
                .await
                .map_err(anyhow::Error::from)
        })
        .await?;
        let session = SocketSession::new(target, stream);
        Ok(Box::new(session))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(SocketConnector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    #[test]
    fn descriptor_declares_streaming() {
        assert_eq!(DESCRIPTOR.id, "socket");
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Streaming));
    }

    /// A throwaway TCP listener that echoes back whatever it reads, on
    /// every connection it accepts (not just the first -- `reconnect()`
    /// needs a second one to dial into) -- no Docker needed at all, unlike
    /// every other network connector in this workspace: a raw socket
    /// server is exactly `tokio::net::TcpListener`, nothing to containerize.
    async fn echo_server() -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => {
                                if socket.write_all(&buf[..n]).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        (addr, handle)
    }

    async fn drain_until<T>(
        session: &mut SocketSession,
        ready: impl Fn(&SocketSession) -> Option<T>,
    ) -> T {
        timeout(Duration::from_secs(5), async {
            loop {
                session.tick();
                if let Some(value) = ready(session) {
                    return value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition never became true within 5s")
    }

    #[tokio::test]
    async fn connect_succeeds_against_a_listening_port() {
        let (addr, _server) = echo_server().await;

        let connector = SocketConnector;
        let saved = SavedConnection {
            name: "test".to_string(),
            driver: "socket".to_string(),
            target: addr,
        };
        let result = connector.connect(saved).await;

        assert!(result.is_ok(), "connect failed: {:?}", result.err());
    }

    #[tokio::test]
    async fn connect_fails_with_nothing_listening() {
        // Port 0 always refuses a connect attempt (never a real listener).
        let connector = SocketConnector;
        let saved = SavedConnection {
            name: "test".to_string(),
            driver: "socket".to_string(),
            target: "127.0.0.1:0".to_string(),
        };

        let result = connector.connect(saved).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_echoes_back_and_both_directions_are_logged() {
        let (addr, _server) = echo_server().await;
        let stream = TcpStream::connect(&addr).await.unwrap();
        let mut session = SocketSession::new(addr, stream);

        session.send("hello", true);

        let entry = drain_until(&mut session, |s| {
            s.entries
                .iter()
                .find(|e| e.direction == Direction::Received)
                .cloned()
        })
        .await;
        assert_eq!(entry.bytes, b"hello\n");
        assert_eq!(session.entries[0].direction, Direction::Sent);
        assert_eq!(session.entries[0].bytes, b"hello\n");
    }

    #[tokio::test]
    async fn send_without_the_newline_toggle_sends_exactly_the_typed_bytes() {
        let (addr, _server) = echo_server().await;
        let stream = TcpStream::connect(&addr).await.unwrap();
        let mut session = SocketSession::new(addr, stream);

        session.send("ping", false);

        let entry = drain_until(&mut session, |s| {
            s.entries
                .iter()
                .find(|e| e.direction == Direction::Received)
                .cloned()
        })
        .await;
        assert_eq!(entry.bytes, b"ping");
    }

    #[tokio::test]
    async fn a_closed_peer_is_reported_as_disconnected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        });
        let stream = TcpStream::connect(&addr).await.unwrap();
        let mut session = SocketSession::new(addr, stream);

        drain_until(&mut session, |s| (!s.connected).then_some(())).await;

        assert!(session.error.is_some());
    }

    #[tokio::test]
    async fn reconnect_dials_the_same_target_again_and_recovers() {
        let (addr, _server) = echo_server().await;
        let stream = TcpStream::connect(&addr).await.unwrap();
        let mut session = SocketSession::new(addr, stream);

        session.reconnect();

        drain_until(&mut session, |s| s.connected.then_some(())).await;
        assert!(session.error.is_none());

        session.send("still works", true);
        let entry = drain_until(&mut session, |s| {
            s.entries
                .iter()
                .find(|e| e.direction == Direction::Received)
                .cloned()
        })
        .await;
        assert_eq!(entry.bytes, b"still works\n");
    }

    #[tokio::test]
    async fn the_entry_buffer_never_grows_past_its_cap() {
        let (addr, _server) = echo_server().await;
        let stream = TcpStream::connect(&addr).await.unwrap();
        let mut session = SocketSession::new(addr, stream);

        for _ in 0..(MAX_BUFFERED_ENTRIES + 20) {
            session.push_entry(Direction::Sent, vec![b'x']);
        }

        assert_eq!(session.entries.len(), MAX_BUFFERED_ENTRIES);
    }
}
