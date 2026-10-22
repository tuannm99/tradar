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

pub mod preview;

#[cfg(feature = "tui")]
mod screen;
#[cfg(feature = "tui")]
mod session;

pub use preview::*;
#[cfg(feature = "tui")]
pub use session::*;
