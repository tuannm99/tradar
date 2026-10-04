//! Newline-delimited JSON over a unix socket: one request object per line,
//! one response object per line, in order. Chosen over msgpack-rpc so a
//! plugin needs nothing but `vim.json` and a pipe, and a human can poke the
//! server with `socat`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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

async fn serve_connection(stream: UnixStream, server: Arc<Server>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let response = if line.len() > MAX_LINE {
            error_response(-32600, "request too large")
        } else if line.trim().is_empty() {
            continue;
        } else {
            match serde_json::from_str::<Value>(&line) {
                Ok(request) => server.handle(request).await,
                Err(e) => error_response(-32700, &format!("parse error: {e}")),
            }
        };
        let mut out = response.to_string();
        out.push('\n');
        write.write_all(out.as_bytes()).await?;
    }
    Ok(())
}

fn error_response(code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": null, "error": {"code": code, "message": message}})
}
