//! Idle connections: a database connection nobody has used for a while is
//! closed so its pool (sockets, prepared statements, driver buffers) stops
//! costing memory. The next call reconnects -- the Neovim plugin already
//! retries on "is not connected", so the user only sees a short pause.
//!
//! The clock is "last call that named this connection" (`Server::touch`),
//! not the background `status` poll, which would otherwise keep every
//! connection warm forever.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

/// Used when `TRADAR_IDLE_TTL` is unset.
pub const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// `TRADAR_IDLE_TTL`: seconds, or a number with an `s`/`m`/`h` suffix
/// (`90`, `10m`, `2h`). `0`, `off` and `never` disable it. Unreadable values
/// fall back to the default rather than silently disabling the feature.
pub fn parse_ttl(value: Option<&str>) -> Option<Duration> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Some(DEFAULT_IDLE_TTL);
    };
    if ["off", "never", "none", "false"]
        .iter()
        .any(|w| raw.eq_ignore_ascii_case(w))
    {
        return None;
    }
    let (digits, unit) = match raw.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => raw.split_at(i),
        None => (raw, "s"),
    };
    let factor = match unit.trim().to_ascii_lowercase().as_str() {
        "s" | "" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return Some(DEFAULT_IDLE_TTL),
    };
    match digits.parse::<u64>() {
        Ok(0) => None,
        Ok(n) => Some(Duration::from_secs(n.saturating_mul(factor))),
        Err(_) => Some(DEFAULT_IDLE_TTL),
    }
}

impl Server {
    /// The idle limit this server was started with (`None`: never closes).
    pub fn idle_ttl(&self) -> Option<Duration> {
        self.idle_ttl
    }

    /// Marks `name` as used now. Only connections that are actually open are
    /// tracked (an extension's connection, e.g. Kafka, is not in `drivers`).
    pub(super) fn touch(&self, name: &str) {
        let mut state = self.state.lock().unwrap();
        if state.drivers.contains_key(name) {
            state.last_used.insert(name.to_string(), Instant::now());
        }
    }

    /// Closes every connection idle for `ttl` as of `now`; returns their
    /// names. Never closes one that is busy: a transaction in progress, a
    /// cancellable query still running, or any call still holding the driver.
    pub fn reap_idle(&self, ttl: Duration, now: Instant) -> Vec<String> {
        let mut state = self.state.lock().unwrap();
        let idle: Vec<String> = state
            .drivers
            .iter()
            .filter(|(name, driver)| {
                let quiet = state
                    .last_used
                    .get(*name)
                    .is_none_or(|t| now.saturating_duration_since(*t) >= ttl);
                quiet
                    && !driver.in_transaction()
                    && Arc::strong_count(driver) == 1
                    && !state.running.values().any(|(_, d)| Arc::ptr_eq(d, driver))
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &idle {
            state.drivers.remove(name);
            state.completions.remove(name);
            state.last_used.remove(name);
        }
        idle
    }
}

/// Checks about four times per `ttl` (at least every 5 s, at most every
/// minute) until the process ends.
pub async fn reap_loop(server: Arc<Server>, ttl: Duration) {
    let every = (ttl / 4).clamp(Duration::from_secs(5), Duration::from_secs(60));
    loop {
        tokio::time::sleep(every).await;
        for name in server.reap_idle(ttl, Instant::now()) {
            tracing::info!(
                "idle-disconnect [{name}] after {}s without use",
                ttl.as_secs()
            );
        }
    }
}
