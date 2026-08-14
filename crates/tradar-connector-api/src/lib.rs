//! The SPI a connector implements: `Connector` is a near-stateless factory
//! that turns a `SavedConnection` into a `Session`; a `Session` is the
//! long-lived actor that owns IO and hands `RootComponent` a `Screen`
//! (`Box<dyn Component>`) to route keys/draws to. See "Kiến trúc mục tiêu:
//! connector pluggable" in docs/architecture.md for the full rationale.

use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

/// How long any connector may spend opening a connection before it gives
/// up. The TUI shows nothing but a frozen picker while a connect is in
/// flight, so a backend that waits on an unreachable host reads as a hung
/// app rather than a failed connect -- which is exactly the bug this once
/// was for Postgres (sqlx's 30s default). Five seconds is long enough for a
/// slow-but-real network and short enough that a wrong host tells you so.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs a connector's own connect, failing with a clear message rather than
/// hanging. Lives here rather than in each connector so every backend gets
/// the same bound and the same wording -- the underlying clients disagree
/// wildly about what they do when a host doesn't answer (sqlx has its own
/// timeout, `redis`/`mongodb`/`reqwest` have their own defaults or none).
pub async fn with_connect_timeout<T>(
    target: &str,
    connect: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "connecting to '{target}' timed out after {}s",
            CONNECT_TIMEOUT.as_secs()
        ),
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;
    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>>;
}

pub trait Session: Send + Sync {
    /// Drains this session's internal channel(s), updating its own state.
    /// Bounded per call -- see "Screen không bao giờ làm IO" in
    /// docs/architecture.md for why an unbounded drain here would starve
    /// rendering under a firehose-shaped connector (Kafka, a tail, ...).
    /// Returns whether anything changed that's worth a redraw, so the event
    /// loop in `main.rs` can skip redrawing an unchanged screen.
    fn tick(&mut self) -> bool;

    /// Builds this session's screen. `restore` is whatever that screen
    /// returned from `Component::restore_state` when the app last quit --
    /// opaque here, meaningful only to the screen (a query screen fills
    /// its editor with it). `None` on a fresh connect.
    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        restore: Option<&str>,
    ) -> Box<dyn Component>;
}

pub struct ConnectorDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub capabilities: &'static [Capability],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_connect_that_completes_in_time_is_passed_straight_through() {
        let result = with_connect_timeout("db", async { Ok(7) }).await;

        assert_eq!(result.unwrap(), 7);
    }

    #[tokio::test]
    async fn a_connect_that_fails_reports_its_own_error_not_a_timeout() {
        let result: anyhow::Result<()> =
            with_connect_timeout("db", async { anyhow::bail!("connection refused") }).await;

        assert_eq!(result.unwrap_err().to_string(), "connection refused");
    }

    /// Time is paused, so this asserts the bound exists without spending it.
    #[tokio::test(start_paused = true)]
    async fn a_connect_that_never_answers_gives_up_and_says_so() {
        let result: anyhow::Result<()> =
            with_connect_timeout("redis://10.0.0.1:6379", std::future::pending()).await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("timed out") && error.contains("10.0.0.1"),
            "the message has to name what it gave up on: {error}"
        );
    }
}
