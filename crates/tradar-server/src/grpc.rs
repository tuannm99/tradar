//! gRPC: connect, discover service/method, invoke -- fully dynamic, no
//! `.proto`-generated Rust types anywhere in this module. Not a
//! `Connector`/`QueryDriver` (see `lib.rs`'s doc comment on why it bypasses
//! `registry`), so this module owns its own connection state
//! (`server.rs`'s `State::grpc`) instead of going through `Arc<dyn
//! QueryDriver>`.
//!
//! Unary goes through `grpc.invoke`; server-/client-streaming and bidi go
//! through `grpc.stream.*` over the push channel (the same shape as Socket's
//! tail). Both take optional request `metadata` (ASCII headers) and report
//! the response headers (and, for streams, trailers). The dynamic-invocation
//! mechanics here were de-risked by a throwaway spike before this was
//! written -- see `docs/backlog/grpc-spike-2026-10-12.md`.

use std::path::Path;

use bytes::Buf;
use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
    ServiceDescriptor,
};
use tonic::client::Grpc;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::{KeyAndValueRef, MetadataKey, MetadataMap, MetadataValue};
use tonic::transport::Channel;
use tonic::{Request, Status};

mod call;
mod codec;
mod connect;
mod extension;
mod metadata;
mod schema;
mod skeleton;
mod stream;

pub use call::*;
pub(crate) use codec::*;
pub use connect::*;
pub use extension::*;
pub use metadata::*;
pub use schema::*;
pub use skeleton::*;
pub use stream::*;

#[cfg(test)]
mod tests;
