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

mod screen;

use async_trait::async_trait;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

pub(crate) use screen::HttpScreen;

/// Bounded per `tick()` call, same reasoning as every other `Session` -- see
/// "Screen không bao giờ làm IO" in docs/architecture.md. Only one request
/// is ever in flight at a time here, so this is a formality rather than a
/// real firehose limit.
const MAX_DRAIN_PER_TICK: usize = 8;

#[derive(Debug, Clone)]
pub(crate) struct HttpResponseData {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub elapsed_ms: u128,
}

pub(crate) enum HttpEvent {
    Response(anyhow::Result<HttpResponseData>),
}

/// Parses `raw` (one `Key: Value` header per line, blank lines ignored)
/// into pairs -- kept as a free function so it's unit-testable without a
/// running client.
pub(crate) fn parse_headers(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Resolves what the user typed in the URL field against the connection's
/// base URL. `path` is used as-is when it already names a scheme, or when
/// there's no base to combine it with -- letting a request target something
/// entirely unrelated to the connection's base, same as a real Postman
/// request can.
pub(crate) fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || base.is_empty() {
        return path.to_string();
    }
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub struct HttpSession {
    client: reqwest::Client,
    pub(crate) base_url: String,
    event_tx: UnboundedSender<HttpEvent>,
    event_rx: UnboundedReceiver<HttpEvent>,
    pub(crate) response: Option<HttpResponseData>,
    pub(crate) sending: bool,
    pub(crate) error: Option<String>,
}

impl HttpSession {
    fn new(client: reqwest::Client, base_url: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            client,
            base_url,
            event_tx,
            event_rx,
            response: None,
            sending: false,
            error: None,
        }
    }

    /// Sends one request. `method` is a plain verb (`"GET"`, `"POST"`, ...);
    /// an unrecognized one falls back to GET rather than failing outright,
    /// since the method field is always one of a fixed set the screen
    /// offers -- there's no way for it to be free text here.
    pub(crate) fn send(&mut self, method: &str, url: &str, headers_raw: &str, body: &str) {
        self.sending = true;
        self.error = None;
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let full_url = resolve_url(&self.base_url, url);
        let mut builder = self.client.request(method, full_url);
        for (key, value) in parse_headers(headers_raw) {
            builder = builder.header(key, value);
        }
        if !body.trim().is_empty() {
            builder = builder.body(body.to_string());
        }
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let result = async {
                let response = builder.send().await?;
                let status = response.status();
                let headers = response
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let body = response.text().await?;
                Ok(HttpResponseData {
                    status: status.as_u16(),
                    status_text: status.canonical_reason().unwrap_or("").to_string(),
                    headers,
                    body,
                    elapsed_ms: start.elapsed().as_millis(),
                })
            }
            .await;
            let _ = tx.send(HttpEvent::Response(result));
        });
    }
}

impl Session for HttpSession {
    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            let event = match self.event_rx.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            changed = true;
            match event {
                HttpEvent::Response(Ok(data)) => {
                    self.response = Some(data);
                    self.sending = false;
                    self.error = None;
                }
                HttpEvent::Response(Err(e)) => {
                    self.sending = false;
                    self.error = Some(e.to_string());
                }
            }
        }
        changed
    }

    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        _restore: Option<&str>,
    ) -> Box<dyn Component> {
        Box::new(HttpScreen::new(*self, action_tx))
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "http",
    display_name: "HTTP",
    icon: "🌐",
    capabilities: &[Capability::Publish],
};

struct HttpConnector;

#[async_trait]
impl Connector for HttpConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    /// Deliberately no liveness probe, unlike RabbitMQ's `connect` -- a base
    /// URL may be empty (every request typed in full) or may not be live
    /// yet, and neither should block adding the connection the way an
    /// unreachable database host should. `target` is the base URL prefix,
    /// used only when a request's URL field doesn't already name a scheme
    /// (see `resolve_url`); it may be empty.
    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let client = reqwest::Client::new();
        let base_url = connection.target.trim_end_matches('/').to_string();
        Ok(Box::new(HttpSession::new(client, base_url)))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(HttpConnector)
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
    fn parse_headers_skips_blank_lines() {
        let headers = parse_headers("Accept: json\n\n\nHost: example.com\n");

        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn parse_headers_skips_a_line_with_no_colon_or_an_empty_key() {
        let headers = parse_headers("not a header\n: value-with-empty-key\nX-Ok: yes");

        assert_eq!(headers, vec![("X-Ok".to_string(), "yes".to_string())]);
    }

    #[test]
    fn resolve_url_uses_the_path_as_is_when_it_already_has_a_scheme() {
        let url = resolve_url("https://api.example.com", "http://other.example.com/x");

        assert_eq!(url, "http://other.example.com/x");
    }

    #[test]
    fn resolve_url_uses_the_path_as_is_when_there_is_no_base() {
        let url = resolve_url("", "/users");

        assert_eq!(url, "/users");
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
    fn descriptor_declares_publish_but_not_query_or_schema() {
        assert_eq!(DESCRIPTOR.id, "http");
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Publish));
        assert!(!DESCRIPTOR.capabilities.contains(&Capability::Query));
        assert!(!DESCRIPTOR.capabilities.contains(&Capability::Schema));
    }

    #[tokio::test]
    async fn a_request_that_never_gets_a_response_leaves_sending_true_until_tick_drains_it() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let mut session = HttpSession {
            client: reqwest::Client::new(),
            base_url: String::new(),
            event_tx,
            event_rx,
            response: None,
            sending: true,
            error: None,
        };

        let changed = session.tick();

        assert!(!changed, "nothing arrived on the channel yet");
        assert!(session.sending);
    }

    #[tokio::test]
    async fn tick_applies_a_successful_response_and_clears_sending() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let mut session = HttpSession {
            client: reqwest::Client::new(),
            base_url: String::new(),
            event_tx: event_tx.clone(),
            event_rx,
            response: None,
            sending: true,
            error: None,
        };
        event_tx
            .send(HttpEvent::Response(Ok(HttpResponseData {
                status: 200,
                status_text: "OK".to_string(),
                headers: vec![],
                body: "{}".to_string(),
                elapsed_ms: 12,
            })))
            .unwrap();

        let changed = session.tick();

        assert!(changed);
        assert!(!session.sending);
        assert_eq!(session.response.unwrap().status, 200);
    }

    #[tokio::test]
    async fn tick_applies_a_failed_response_and_reports_the_error() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let mut session = HttpSession {
            client: reqwest::Client::new(),
            base_url: String::new(),
            event_tx: event_tx.clone(),
            event_rx,
            response: None,
            sending: true,
            error: None,
        };
        event_tx
            .send(HttpEvent::Response(Err(anyhow::anyhow!(
                "connection refused"
            ))))
            .unwrap();

        session.tick();

        assert!(!session.sending);
        assert_eq!(session.error.as_deref(), Some("connection refused"));
    }

    mod docker {
        //! Integration test against a real HTTP server, via `testcontainers`
        //! directly -- same reasoning as RabbitMQ/Kafka/Cassandra's own
        //! `mod docker` (no `testcontainers-modules` support needed here
        //! either; a plain static-file server image is enough).

        use std::time::Duration;

        use testcontainers::core::{IntoContainerPort, WaitFor};
        use testcontainers::runners::AsyncRunner;
        use testcontainers::{ContainerAsync, GenericImage, ImageExt};

        use super::*;

        async fn httpbin() -> (ContainerAsync<GenericImage>, String) {
            // gunicorn (this image's server) logs its startup line to
            // stderr, not stdout -- confirmed by hand (`docker logs
            // --details` vs `docker logs 2>&1 1>/dev/null`) after this wait
            // condition first hung the full 60s timeout with the image
            // already pulled and no port conflict, so it wasn't a slow
            // start.
            let container = GenericImage::new("kennethreitz/httpbin", "latest")
                .with_wait_for(WaitFor::message_on_stderr("Listening at"))
                .with_mapped_port(8080, 80.tcp())
                .start()
                .await
                .expect("httpbin container must start");
            (container, "http://127.0.0.1:8080".to_string())
        }

        async fn drain_until<T>(
            session: &mut HttpSession,
            timeout: Duration,
            mut ready: impl FnMut(&HttpSession) -> Option<T>,
        ) -> T {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                session.tick();
                if let Some(value) = ready(session) {
                    return value;
                }
                if tokio::time::Instant::now() >= deadline {
                    panic!("condition never became true within {timeout:?}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        #[tokio::test]
        async fn a_get_request_reaches_a_real_server_and_reports_status_and_body() {
            let (_container, base_url) = tokio::time::timeout(Duration::from_secs(60), httpbin())
                .await
                .expect("container must become ready within 60s");
            let mut session = HttpSession::new(reqwest::Client::new(), base_url);

            session.send("GET", "/get", "X-Test: 1", "");

            let response = drain_until(&mut session, Duration::from_secs(10), |s| {
                s.response.clone()
            })
            .await;
            assert_eq!(response.status, 200);
            assert!(response.body.contains("\"X-Test\""), "{}", response.body);
        }
    }
}
