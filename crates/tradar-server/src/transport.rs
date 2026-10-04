//! Newline-delimited JSON over a unix socket: one request object per line,
//! one response object per line, in order. Chosen over msgpack-rpc so a
//! plugin needs nothing but `vim.json` and a pipe, and a human can poke the
//! server with `socat`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::server::Server;

/// A request line longer than this is refused rather than buffered: this
/// socket answers queries, so a client has no business sending megabytes.
const MAX_LINE: usize = 1 << 20;

pub fn default_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("tradar").join("server.sock")
}

/// Binds `path`, owner-only: the socket is a door to every saved database,
/// so the directory is `0700` before the socket exists and the socket is
/// `0600` after -- no window where another local user can connect.
pub async fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        // A live server answers; a stale file from a crash doesn't.
        if UnixStream::connect(path).await.is_ok() {
            anyhow::bail!("a tradar-server is already listening on {}", path.display());
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub async fn serve(listener: UnixListener, server: Arc<Server>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = serve_connection(stream, server).await;
        });
    }
}

/// Requests on one connection run concurrently, so a slow `execute` never
/// stands in front of the `cancel` meant for it -- which also means replies
/// can arrive out of order; clients match them by `id`. The tasks live in a
/// `JoinSet`, so a client that disconnects takes its in-flight queries with
/// it instead of leaving them running for nobody.
async fn serve_connection(stream: UnixStream, server: Arc<Server>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let (replies, mut outbox) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = outbox.recv().await {
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut in_flight = JoinSet::new();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let replies = replies.clone();
        let server = Arc::clone(&server);
        in_flight.spawn(async move {
            let response = if line.len() > MAX_LINE {
                error_response(-32600, "request too large")
            } else {
                match serde_json::from_str::<Value>(&line) {
                    Ok(request) => server.handle(request).await,
                    Err(e) => error_response(-32700, &format!("parse error: {e}")),
                }
            };
            let _ = replies.send(format!("{response}\n"));
        });
        // Reap finished tasks so a long-lived connection doesn't grow the set.
        while in_flight.try_join_next().is_some() {}
    }
    drop(in_flight);
    drop(replies);
    let _ = writer.await;
    Ok(())
}

fn error_response(code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": null, "error": {"code": code, "message": message}})
}
