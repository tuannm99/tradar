//! Kafka connector -- like `tradar-connector-rabbitmq`, does not implement
//! `QueryDriver`/reuse `tradar-query-workbench` (Kafka has no query
//! language either). Unlike RabbitMQ, this one *does* need a real-time
//! firehose: tailing a topic is exactly the "Kafka consumer of thousands
//! of messages/sec" example `docs/architecture.md`'s "Screen không bao giờ
//! làm IO" section uses to justify the bounded-per-tick-drain design, so
//! this crate is the first real exercise of that path.
//!
//! v1 scope was Topics mode only (tail + publish); Groups mode (per-
//! partition consumer-group lag, added 2026-10-01) is the second mode --
//! see `docs/backlog/kafka-groups-mode-2026-10-01.md`.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

pub mod tail;

#[cfg(feature = "tui")]
mod screen;
#[cfg(feature = "tui")]
mod session;

#[cfg(feature = "tui")]
pub use session::*;
pub use tail::*;
