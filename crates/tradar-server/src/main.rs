use std::path::PathBuf;
use std::sync::Arc;

use tradar_core::storage::{ConnectionStore, default_connections_path};
use tradar_server::{Server, bind, default_socket_path, registry, serve};

/// `tradar-server [SOCKET_PATH]` -- defaults to `$XDG_RUNTIME_DIR/tradar/server.sock`.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);
    let store = ConnectionStore::at(default_connections_path()?);
    let server = Arc::new(Server::new(registry(), Some(store)));
    let shutdown = server.shutdown_signal();

    let listener = bind(&path).await?;
    eprintln!("tradar-server listening on {}", path.display());

    tokio::select! {
        _ = serve(listener, server) => {}
        _ = tokio::signal::ctrl_c() => {}
        _ = shutdown.notified() => {
            // Let the `shutdown` reply reach the client before the sockets go.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}
