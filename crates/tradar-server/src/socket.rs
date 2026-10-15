//! Socket tail: raw TCP, netcat-style, over the same push-channel this
//! server already built for Kafka (`kafka.rs`'s sibling module -- same
//! `ClientConnection`/notification mechanism, see
//! `docs/backlog/push-channel-design-2026-10-14.md`). Not a reuse of
//! `tradar-connector-kafka`'s TUI-shaped sibling `tradar-connector-socket`:
//! that crate's `SocketSession` is shaped around a `tick()`-driven event
//! queue for the TUI's render loop, same reason Kafka's own server module
//! didn't reuse its connector crate either.
//!
//! Unlike Kafka (a tail is a throwaway consumer against an already-probed
//! broker, decoupled from `kafka.connect`), a Socket "connection" *is* the
//! live TCP stream -- there's no broker-style resource to dial once and
//! reuse: `socket.tail.start` dials the target and starts tailing in the
//! same call, no separate `socket.connect` at all. A second
//! `socket.tail.start` against the same saved connection opens a second,
//! fully independent TCP connection (a second netcat session) rather than
//! sharing the first one's read -- same "every subscription is its own
//! independent reader, never shared" rule Kafka's tail already follows, and
//! the only shape that makes sense for a raw socket (nothing to fan out a
//! single TCP stream's bytes to several readers without one of them
//! racing the other for every byte).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// One `read()` can return at most this many bytes -- same bound and
/// reasoning as `tradar-connector-socket`'s own `READ_CHUNK_SIZE`.
const READ_CHUNK_SIZE: usize = 8192;

/// Same cap and reasoning as `kafka.rs`'s own `MAX_BUFFERED_MESSAGES`: a
/// live tail, not a durable log, so a client reading slower than data
/// arrives must not grow this server's memory without bound.
const MAX_BUFFERED_MESSAGES: usize = 500;

/// How many bytes of a binary (non-UTF-8-text) chunk to show as hex --
/// same bound and reasoning as `tradar-connector-socket`'s own
/// `HEX_PREVIEW_BYTES`: enough to recognize a protocol's magic bytes,
/// short enough that one binary chunk doesn't dominate the transcript.
const HEX_PREVIEW_BYTES: usize = 32;

fn looks_like_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| {
        s.chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t')
    })
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

/// Text when the bytes are valid UTF-8 with nothing but ordinary
/// whitespace control characters, a hex preview otherwise -- same
/// "decode when possible, don't silently lose binary data" rule
/// `tradar-connector-socket`'s own `format_entry` already uses, moved
/// server-side since JSON has no byte-string type to send raw bytes as.
fn chunk_text(bytes: &[u8]) -> String {
    if looks_like_text(bytes) {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        hex_preview(bytes)
    }
}

fn push_notification(buffer: &Mutex<VecDeque<Value>>, ready: &Notify, item: Value) {
    let mut q = buffer.lock().unwrap();
    q.push_back(item);
    while q.len() > MAX_BUFFERED_MESSAGES {
        q.pop_front();
    }
    drop(q);
    ready.notify_one();
}

struct Inner {
    write_half: tokio::sync::Mutex<OwnedWriteHalf>,
    reader: JoinHandle<()>,
    forwarder: JoinHandle<()>,
}

/// A running tail, cheaply `Clone`-able (an `Arc` around the write half and
/// both task handles) so `server.rs` can hand a copy to `socket.send`
/// (which needs to write) while keeping the original in its subscription
/// map -- `JoinHandle::abort` takes `&self`, so `stop` doesn't need to
/// consume the last copy either.
#[derive(Clone)]
pub struct TailHandle(Arc<Inner>);

impl TailHandle {
    pub fn stop(&self) {
        self.0.reader.abort();
        self.0.forwarder.abort();
    }

    pub async fn send(&self, payload: Vec<u8>) -> std::io::Result<()> {
        self.0.write_half.lock().await.write_all(&payload).await
    }
}

/// Dials `target` and starts tailing it immediately -- see the module doc
/// comment for why there's no separate connect step. Fails loudly here
/// (like `kafka::connect`'s metadata probe) rather than silently on the
/// first read.
///
/// Same two-task shape as `kafka::start_tail`: `reader` only talks to the
/// socket (never blocks on `push`), `forwarder` only talks to `push`,
/// connected by a bounded ring buffer that drops the oldest entry first if
/// a slow client falls behind.
pub async fn start_tail(
    target: &str,
    subscription: String,
    push: impl Fn(Value) + Send + 'static,
) -> anyhow::Result<TailHandle> {
    let stream =
        tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(target)).await??;
    let (mut read_half, write_half) = stream.into_split();

    let buffer: Arc<Mutex<VecDeque<Value>>> = Arc::new(Mutex::new(VecDeque::new()));
    let ready = Arc::new(Notify::new());

    let reader = {
        let buffer = Arc::clone(&buffer);
        let ready = Arc::clone(&ready);
        let subscription = subscription.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; READ_CHUNK_SIZE];
            loop {
                match read_half.read(&mut buf).await {
                    Ok(0) => {
                        push_notification(
                            &buffer,
                            &ready,
                            json!({"subscription": subscription, "error": "connection closed by peer"}),
                        );
                        return;
                    }
                    Ok(n) => {
                        push_notification(
                            &buffer,
                            &ready,
                            json!({
                                "subscription": subscription,
                                "direction": "received",
                                "text": chunk_text(&buf[..n]),
                            }),
                        );
                    }
                    Err(e) => {
                        push_notification(
                            &buffer,
                            &ready,
                            json!({"subscription": subscription, "error": e.to_string()}),
                        );
                        return;
                    }
                }
            }
        })
    };

    let forwarder = tokio::spawn(async move {
        loop {
            let batch: Vec<Value> = {
                let mut q = buffer.lock().unwrap();
                q.drain(..).collect()
            };
            if batch.is_empty() {
                ready.notified().await;
                continue;
            }
            for item in batch {
                push(item);
            }
        }
    });

    Ok(TailHandle(Arc::new(Inner {
        write_half: tokio::sync::Mutex::new(write_half),
        reader,
        forwarder,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    #[test]
    fn push_notification_drops_the_oldest_past_the_cap() {
        let buffer: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
        let ready = Notify::new();
        for i in 0..(MAX_BUFFERED_MESSAGES + 10) {
            push_notification(&buffer, &ready, json!({"i": i}));
        }
        let q = buffer.lock().unwrap();
        assert_eq!(q.len(), MAX_BUFFERED_MESSAGES);
        assert_eq!(q.front().unwrap()["i"], 10);
        assert_eq!(q.back().unwrap()["i"], MAX_BUFFERED_MESSAGES + 9);
    }

    #[test]
    fn chunk_text_renders_plain_text_as_is_and_binary_as_hex() {
        assert_eq!(chunk_text(b"hello\n"), "hello\n");
        assert_eq!(chunk_text(&[0xff, 0x00, 0x01]), "ff 00 01");
    }

    /// A throwaway echo server, same shape as
    /// `tradar-connector-socket`'s own test helper -- a raw TCP listener
    /// needs no Docker, unlike every other network connector test here.
    async fn echo_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
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
        addr
    }

    #[tokio::test]
    async fn start_tail_fails_against_a_closed_port() {
        // Port 0 always refuses a connect attempt.
        let result = start_tail("127.0.0.1:0", "s1".to_string(), |_| {}).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_echoes_back_as_a_received_notification() {
        let addr = echo_server().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = start_tail(&addr, "s1".to_string(), move |item| {
            let _ = tx.send(item);
        })
        .await
        .unwrap();

        handle.send(b"hello\n".to_vec()).await.unwrap();

        let notification = timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notification["subscription"], "s1");
        assert_eq!(notification["direction"], "received");
        assert_eq!(notification["text"], "hello\n");
    }

    #[tokio::test]
    async fn a_closed_peer_is_reported_as_an_error_notification() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _handle = start_tail(&addr, "s1".to_string(), move |item| {
            let _ = tx.send(item);
        })
        .await
        .unwrap();

        let notification = timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notification["subscription"], "s1");
        assert!(notification.get("error").is_some());
    }
}
