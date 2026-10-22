//! RabbitMQ connector -- the first connector in this workspace that does
//! *not* implement `QueryDriver`/reuse `tradar-query-workbench`. RabbitMQ
//! has no query language: browsing queues/exchanges and peeking/publishing
//! messages is a different shape ("Screen không bao giờ làm IO" in
//! docs/architecture.md), so this crate implements `Connector`/`Session`
//! directly and builds its own `Screen` (`RabbitScreen`, in `screen.rs`).
//!
//! Talks to the RabbitMQ **Management HTTP API** (via `reqwest`) rather than
//! AMQP -- see "Thiết kế UI: Kafka và RabbitMQ" in docs/architecture.md for
//! why: it reuses a dependency already in the workspace (Elasticsearch),
//! needs no new client crate, and covers browse/peek/publish. The tradeoff
//! is no real-time tail (the Management API has no streaming endpoint) --
//! peeking is polled on demand, not followed like Kafka.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

pub mod api;

#[cfg(feature = "tui")]
mod screen;
#[cfg(feature = "tui")]
mod session;

pub use api::*;
#[cfg(feature = "tui")]
pub use session::*;
