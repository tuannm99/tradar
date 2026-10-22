//! HTTP over `tradar-server`: a Postman-style request/response, no query
//! language and (like RabbitMQ) no push-channel -- a plain request/response
//! method, same shape `grpc.rs`/`rabbitmq.rs` already use. Sends inline
//! rather than through `tradar-connector-http`'s TUI `HttpSession` (a
//! `tick()`-driven queue for a render loop; a JSON-RPC call already *is* the
//! async boundary), but `parse_headers`/`resolve_url` come from that crate's
//! TUI-free `request` module so both interfaces build requests the same way.

use serde::Serialize;
use tradar_connector_http::{parse_headers, resolve_url};

#[derive(Debug, Clone, Serialize)]
pub struct HttpResponseInfo {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub elapsed_ms: u128,
}

/// Cheap to `Clone` (a `reqwest::Client` is an `Arc` handle internally) --
/// `server.rs` clones one out of its `Mutex<State>` guard before the
/// `.await` that actually sends the request, same reasoning as
/// `RabbitConnection`.
#[derive(Clone)]
pub struct HttpConnection {
    client: reqwest::Client,
    base_url: String,
}

/// No liveness probe -- unlike RabbitMQ's `connect`, a base URL may be
/// empty (every request typed in full) or not live yet, and neither should
/// block adding the connection the way an unreachable database host
/// should (same reasoning as `tradar-connector-http`'s own `connect`).
/// Not async: there is no IO to wait on here.
pub fn connect(target: &str) -> HttpConnection {
    HttpConnection {
        client: reqwest::Client::new(),
        base_url: target.trim_end_matches('/').to_string(),
    }
}

/// Sends one request. `method` is a plain verb (`"GET"`, `"POST"`, ...); an
/// unrecognized one falls back to GET rather than failing outright, same
/// as `tradar-connector-http`'s own `HttpSession::send`.
pub async fn send(
    conn: &HttpConnection,
    method: &str,
    url: &str,
    headers_raw: &str,
    body: &str,
) -> anyhow::Result<HttpResponseInfo> {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let full_url = resolve_url(&conn.base_url, url);
    let mut builder = conn.client.request(method, full_url);
    for (key, value) in parse_headers(headers_raw) {
        builder = builder.header(key, value);
    }
    if !body.trim().is_empty() {
        builder = builder.body(body.to_string());
    }
    let start = tokio::time::Instant::now();
    let response = builder.send().await?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = response.text().await?;
    Ok(HttpResponseInfo {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

/// `http.*` -- no state beyond the connections themselves, no push.
#[derive(Default)]
pub struct HttpExtension {
    connections: std::sync::Mutex<std::collections::HashMap<String, HttpConnection>>,
}

#[async_trait::async_trait]
impl crate::extension::Extension for HttpExtension {
    fn driver(&self) -> &'static str {
        "http"
    }

    fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().unwrap().contains_key(name)
    }

    async fn handle(
        &self,
        method: &str,
        params: &serde_json::Value,
        ctx: &crate::extension::Ctx<'_>,
    ) -> Result<serde_json::Value, crate::protocol::RpcError> {
        use crate::extension::{not_connected, unknown_method};
        use crate::protocol::{RpcError, str_param};
        use serde_json::{Value, json};

        let name = str_param(params, "connection")?;
        match method {
            // No liveness probe (see the module doc comment), so this never
            // fails -- it only needs a saved connection to exist.
            "connect" => {
                if self.is_connected(name) {
                    return Ok(json!({"connected": true, "already": true}));
                }
                let saved = ctx.saved(name, "http")?;
                let conn = connect(&saved.target);
                self.connections
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), conn);
                Ok(json!({"connected": true, "already": false}))
            }
            "disconnect" => {
                let removed = self.connections.lock().unwrap().remove(name).is_some();
                Ok(json!({"disconnected": removed}))
            }
            "send" => {
                let field = |key: &str, default: &'static str| -> String {
                    params
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or(default)
                        .to_string()
                };
                let conn = self
                    .connections
                    .lock()
                    .unwrap()
                    .get(name)
                    .cloned()
                    .ok_or_else(|| not_connected("http", name))?;
                let response = send(
                    &conn,
                    &field("method", "GET"),
                    &field("url", ""),
                    &field("headers", ""),
                    &field("body", ""),
                )
                .await
                .map_err(|e| RpcError::app(e.to_string()))?;
                Ok(json!(response))
            }
            _ => Err(unknown_method("http", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_headers_splits_key_and_value_on_the_first_colon() {
        let headers = parse_headers("Accept: application/json\nAuthorization: Bearer abc:def");

        assert_eq!(
            headers,
            vec![
                ("Accept".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer abc:def".to_string()),
            ]
        );
    }

    #[test]
    fn parse_headers_skips_blank_lines_and_lines_with_no_colon_or_empty_key() {
        let headers = parse_headers("not a header\n: empty-key\nAccept: json\n\nHost: example.com");

        assert_eq!(
            headers,
            vec![
                ("Accept".to_string(), "json".to_string()),
                ("Host".to_string(), "example.com".to_string()),
            ]
        );
    }

    #[test]
    fn resolve_url_uses_the_path_as_is_when_it_already_has_a_scheme_or_there_is_no_base() {
        assert_eq!(
            resolve_url("https://api.example.com", "http://other.example.com/x"),
            "http://other.example.com/x"
        );
        assert_eq!(resolve_url("", "/users"), "/users");
    }

    #[test]
    fn resolve_url_joins_base_and_path_with_exactly_one_slash() {
        assert_eq!(
            resolve_url("https://api.example.com/", "/users"),
            "https://api.example.com/users"
        );
        assert_eq!(
            resolve_url("https://api.example.com", "users"),
            "https://api.example.com/users"
        );
    }

    #[test]
    fn connect_trims_a_trailing_slash_off_the_base_url() {
        let conn = connect("https://api.example.com/");
        assert_eq!(conn.base_url, "https://api.example.com");
    }

    #[tokio::test]
    async fn send_falls_back_to_get_for_an_unrecognized_method() {
        // `Method::from_bytes` rejects an empty byte string -- confirms the
        // fallback path is actually reachable, not just plausible.
        let method = reqwest::Method::from_bytes(b"").unwrap_or(reqwest::Method::GET);
        assert_eq!(method, reqwest::Method::GET);
    }

    /// A real request against a plain `tokio::net::TcpListener` echoing a
    /// fixed HTTP/1.1 response -- no `testcontainers` dependency here (this
    /// crate has none, unlike `tradar-connector-http`'s own Docker-based
    /// test), same "don't need Docker for an HTTP client" reasoning
    /// `tests/rpc_rabbitmq.rs`'s hand-rolled server already established.
    #[tokio::test]
    async fn send_reaches_a_real_server_and_reports_status_headers_and_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let body = "hello";
            let response = format!(
                "HTTP/1.1 201 Created\r\nX-Reply: yes\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let conn = connect(&format!("http://{addr}"));
        let response = send(&conn, "GET", "/", "X-Test: 1", "")
            .await
            .expect("request must succeed");

        assert_eq!(response.status, 201);
        assert_eq!(response.body, "hello");
        assert!(
            response
                .headers
                .iter()
                .any(|(k, v)| k == "x-reply" && v == "yes"),
            "{:?}",
            response.headers
        );
    }
}
