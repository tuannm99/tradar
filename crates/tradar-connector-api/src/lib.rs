//! The SPI a connector implements: `Connector` is a near-stateless factory
//! that turns a `SavedConnection` into a `Session`; a `Session` is the
//! long-lived actor that owns IO and hands `RootComponent` a `Screen`
//! (`Box<dyn Component>`) to route keys/draws to. See "Kiến trúc mục tiêu:
//! connector pluggable" in docs/architecture.md for the full rationale.

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

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

    fn build_screen(self: Box<Self>, action_tx: UnboundedSender<Action>) -> Box<dyn Component>;
}

pub struct ConnectorDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub capabilities: &'static [Capability],
}
