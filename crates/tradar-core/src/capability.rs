//! What a connector can do, independent of its identity -- lets a picker or
//! screen reason about a connector without hardcoding which one it is. Not
//! consumed by any UI yet (see "Non-goals" in docs/architecture.md); defined
//! now because retrofitting it after several connectors exist would be more
//! disruptive than defining the shape up front.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Query,
    Schema,
    Streaming,
    Publish,
    Tail,
    Explain,
    Export,
}
