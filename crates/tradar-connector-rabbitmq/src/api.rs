//! The RabbitMQ Management API's shapes and how a target URL becomes a base
//! URL + credentials + vhost. Shared by the TUI session and `tradar-server`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueInfo {
    pub name: String,
    #[serde(default)]
    pub messages_ready: u64,
    #[serde(default)]
    pub messages_unacknowledged: u64,
    #[serde(default)]
    pub consumers: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingInfo {
    pub destination: String,
    #[serde(default)]
    pub routing_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub routing_key: String,
    #[serde(default)]
    pub redelivered: bool,
    #[serde(default)]
    pub payload: String,
}

/// A parsed `target`: everything needed to call the Management API.
#[derive(Debug, Clone)]
pub struct RabbitTarget {
    pub base_url: String,
    pub auth: Option<(String, String)>,
    /// Already the URL-encoded path segment the Management API expects
    /// (e.g. `%2f` for the default vhost) -- see `parse_target`.
    pub vhost: String,
}

/// Parses `target` (a full URL, e.g.
/// `http://user:password@localhost:15672/%2f`) into what's needed to call
/// the Management API. The vhost is taken as-is from the URL path, already
/// percent-encoded exactly how the Management API wants it in its own URLs
/// -- so `%2f` (default vhost) or a custom vhost name round-trip without
/// this crate needing its own percent-encoding logic.
pub fn parse_target(target: &str) -> anyhow::Result<RabbitTarget> {
    let url = reqwest::Url::parse(target)
        .map_err(|e| anyhow::anyhow!("invalid RabbitMQ management URL '{target}': {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("'{target}' has no host"))?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let base_url = format!("{}://{host}{port}", url.scheme());
    // `username()`/`password()` come back still percent-encoded (`p%40ss`
    // for `p@ss`); sending that as Basic credentials is a wrong password.
    let decode = |s: &str| {
        percent_encoding::percent_decode_str(s)
            .decode_utf8_lossy()
            .into_owned()
    };
    let auth = if url.username().is_empty() {
        None
    } else {
        Some((decode(url.username()), decode(url.password().unwrap_or(""))))
    };
    let vhost = match url.path().trim_start_matches('/') {
        "" => "%2f".to_string(),
        other => other.to_string(),
    };
    Ok(RabbitTarget {
        base_url,
        auth,
        vhost,
    })
}

impl RabbitTarget {
    /// A Management API request for `path` (`/api/...`), with the target's
    /// credentials attached.
    pub fn request(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.base_url);
        let request = client.request(method, url);
        match &self.auth {
            Some((user, pass)) => request.basic_auth(user, Some(pass)),
            None => request,
        }
    }
}
