//! HTTP connector -- a "database" only in the loose sense the rest of this
//! app doesn't require: no query language, no schema, no rows. Like Kafka
//! and RabbitMQ this implements `Connector`/`Session` directly and builds
//! its own bespoke `Screen` (`HttpScreen`, in `screen.rs`) rather than
//! reusing `QueryScreenComponent` -- see "Thiết kế UI: HTTP, gRPC, Socket"
//! in docs/architecture.md for the full design.
//!
//! UI shape is Postman-style (separate method/URL/headers/body fields, a
//! response pane below), not the console-style REPL Elasticsearch and Redis
//! use -- a deliberate choice made after review, not the initial one.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

pub mod request;

#[cfg(feature = "tui")]
mod screen;
#[cfg(feature = "tui")]
mod session;

pub use request::*;
#[cfg(feature = "tui")]
pub use session::*;
