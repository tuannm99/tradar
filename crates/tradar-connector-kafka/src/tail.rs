//! Consumer settings for tailing a topic. Shared by the TUI session and
//! `tradar-server`.

use std::sync::atomic::{AtomicU64, Ordering};

/// A consumer group id no real application uses, `<prefix>-<nanos>-<n>`.
pub fn ephemeral_group_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    // Never joins a real consumer group -- every tail session gets its own,
    // so tailing a topic never steals partitions from (or rebalances) an
    // application's actual consumers.
    format!("{prefix}-{now}-{n}")
}
