//! Kafka tail: the first connector this server pushes background
//! notifications for (`kafka.tail.message`, with no request behind each
//! one -- see `docs/backlog/push-channel-design-2026-10-14.md`). Not a
//! `Connector`/`QueryDriver`, and not `tradar-connector-kafka`'s TUI
//! `KafkaSession` (a `tick()`-driven queue for a render loop, not "push a
//! line the instant a message arrives"); only the throwaway consumer-group
//! naming comes from that crate. `server.rs` owns the connection-id/subscription
//! bookkeeping and the `ClientConnection::notify` wiring; this module only
//! knows `rdkafka`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer, DefaultConsumerContext, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::topic_partition_list::{Offset, TopicPartitionList};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tradar_connector_kafka::ephemeral_group_id;

const CLUSTER_TIMEOUT: Duration = Duration::from_secs(5);

/// How many not-yet-forwarded tail notifications are kept before the
/// oldest is dropped -- same constant and reasoning as
/// `tradar-connector-kafka`'s own `MAX_BUFFERED_MESSAGES`: a live tail,
/// not a durable log, so a client reading slower than messages arrive
/// must not grow this server's memory without bound.
const MAX_BUFFERED_MESSAGES: usize = 500;

pub struct KafkaConnection {
    pub(crate) brokers: String,
}

/// Opens a Kafka client and probes the broker's metadata once -- fails
/// loudly here (like `grpc.connect`'s channel dial), not silently on the
/// first `list_topics`/`start_tail` instead.
pub async fn connect(target: &str) -> anyhow::Result<KafkaConnection> {
    let brokers = target.to_string();
    let probe = brokers.clone();
    tokio::task::spawn_blocking(move || {
        let mut config = ClientConfig::new();
        config.set("bootstrap.servers", &probe);
        let client: BaseConsumer = config.create()?;
        client.fetch_metadata(None, CLUSTER_TIMEOUT)?;
        Ok::<(), rdkafka::error::KafkaError>(())
    })
    .await??;
    Ok(KafkaConnection { brokers })
}

/// Topic names, internal bookkeeping topics (`__consumer_offsets`, ...)
/// filtered out -- same rule `tradar-connector-kafka`'s own `list_topics`
/// uses for the same reason Cassandra skips `system*` keyspaces.
pub async fn list_topics(brokers: &str) -> anyhow::Result<Vec<String>> {
    let brokers = brokers.to_string();
    let names = tokio::task::spawn_blocking(move || {
        let mut config = ClientConfig::new();
        config.set("bootstrap.servers", &brokers);
        let client: BaseConsumer = config.create()?;
        let metadata = client.fetch_metadata(None, CLUSTER_TIMEOUT)?;
        let mut names: Vec<String> = metadata
            .topics()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|n| !n.starts_with("__"))
            .collect();
        names.sort_unstable();
        Ok::<Vec<String>, rdkafka::error::KafkaError>(names)
    })
    .await??;
    Ok(names)
}

fn push_notification(buffer: &Mutex<VecDeque<Value>>, ready: &Notify, item: Value) {
    let mut q = buffer.lock().unwrap();
    q.push_back(item);
    while q.len() > MAX_BUFFERED_MESSAGES {
        q.pop_front();
    }
    drop(q);
    ready.notify_one();
}

/// A running tail: `reader` only talks to Kafka, `forwarder` only talks to
/// `push` (see `start_tail`) -- both aborted together by `stop`.
pub struct TailHandle {
    reader: JoinHandle<()>,
    forwarder: JoinHandle<()>,
}

impl TailHandle {
    pub fn stop(self) {
        self.reader.abort();
        self.forwarder.abort();
    }
}

/// Starts tailing `topic` from `from_beginning ? Beginning : End` with a
/// throwaway consumer/group (see `ephemeral_group_id`) -- same mechanics
/// as `tradar-connector-kafka`'s own `start_tail`, just delivering
/// messages as `push(notification)` calls instead of into a `tick()`-read
/// event queue.
///
/// Two tasks, not one, connected by a bounded ring buffer
/// (`MAX_BUFFERED_MESSAGES`, mirroring `KafkaSession.messages`): `reader`
/// never blocks on `push` (reading from Kafka is cheap and must stay
/// that way), `forwarder` drains the buffer into `push` and drops the
/// oldest entry first if it ever gets ahead of a slow client -- the
/// firehose-safe shape `docs/architecture.md`'s "Screen không bao giờ làm
/// IO" section already describes, moved from a `tick()`-drained queue to
/// a push-driven one.
pub fn start_tail(
    brokers: String,
    topic: String,
    from_beginning: bool,
    subscription: String,
    push: impl Fn(Value) + Send + 'static,
) -> TailHandle {
    let buffer: Arc<Mutex<VecDeque<Value>>> = Arc::new(Mutex::new(VecDeque::new()));
    let ready = Arc::new(Notify::new());

    let reader = {
        let buffer = Arc::clone(&buffer);
        let ready = Arc::clone(&ready);
        let subscription = subscription.clone();
        tokio::spawn(async move {
            macro_rules! fail {
                ($msg:expr) => {{
                    push_notification(
                        &buffer,
                        &ready,
                        json!({"subscription": subscription, "error": $msg}),
                    );
                    return;
                }};
            }

            let mut config = ClientConfig::new();
            config
                .set("bootstrap.servers", &brokers)
                .set("group.id", ephemeral_group_id("tradar-server-tail"))
                .set("enable.auto.commit", "false");
            let consumer: StreamConsumer<DefaultConsumerContext> = match config.create() {
                Ok(c) => c,
                Err(e) => fail!(e.to_string()),
            };
            let metadata = match consumer.fetch_metadata(Some(&topic), CLUSTER_TIMEOUT) {
                Ok(m) => m,
                Err(e) => fail!(e.to_string()),
            };
            let Some(topic_metadata) = metadata.topics().iter().find(|t| t.name() == topic) else {
                fail!(format!("unknown topic '{topic}'"));
            };

            let offset = if from_beginning {
                Offset::Beginning
            } else {
                Offset::End
            };
            let mut assignment = TopicPartitionList::new();
            for partition in topic_metadata.partitions() {
                assignment
                    .add_partition_offset(&topic, partition.id(), offset)
                    .ok();
            }
            if let Err(e) = consumer.assign(&assignment) {
                fail!(e.to_string());
            }

            let mut stream = consumer.stream();
            while let Some(next) = stream.next().await {
                let notification = match next {
                    Ok(message) => json!({
                        "subscription": subscription,
                        "partition": message.partition(),
                        "offset": message.offset(),
                        "key": message.key().map(|k| String::from_utf8_lossy(k).into_owned()),
                        "value": message
                            .payload()
                            .map(|p| String::from_utf8_lossy(p).into_owned())
                            .unwrap_or_default(),
                    }),
                    Err(e) => json!({"subscription": subscription, "error": e.to_string()}),
                };
                push_notification(&buffer, &ready, notification);
            }
        })
    };

    let forwarder = tokio::spawn(async move {
        loop {
            let batch: Vec<Value> = {
                let mut q = buffer.lock().unwrap();
                q.drain(..).collect()
            };
            if batch.is_empty() {
                ready.notified().await;
                continue;
            }
            for item in batch {
                push(item);
            }
        }
    });

    TailHandle { reader, forwarder }
}

struct KafkaState {
    connections: std::collections::HashMap<String, KafkaConnection>,
    tails: crate::subscriptions::Subscriptions<TailHandle>,
}

impl Default for KafkaState {
    fn default() -> Self {
        Self {
            connections: Default::default(),
            tails: crate::subscriptions::Subscriptions::new("kafka"),
        }
    }
}

/// `kafka.*` -- connections plus push-style tail subscriptions (see
/// `docs/backlog/push-channel-design-2026-10-14.md`).
#[derive(Default)]
pub struct KafkaExtension {
    state: std::sync::Mutex<KafkaState>,
}

#[async_trait::async_trait]
impl crate::extension::Extension for KafkaExtension {
    fn driver(&self) -> &'static str {
        "kafka"
    }

    fn is_connected(&self, name: &str) -> bool {
        self.state.lock().unwrap().connections.contains_key(name)
    }

    /// A running tail holds its own copy of the brokers, so closing the
    /// connection never cuts a tail short.
    fn connected(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .connections
            .keys()
            .cloned()
            .collect()
    }

    fn close(&self, name: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .connections
            .remove(name)
            .is_some()
    }

    /// Aborts and forgets every tail `client_id` started and never
    /// explicitly `kafka.tail.stop`ped.
    fn client_disconnected(&self, client_id: u64) {
        let handles = self.state.lock().unwrap().tails.remove_client(client_id);
        for handle in handles {
            handle.stop();
        }
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

        let brokers_of = |name: &str| {
            self.state
                .lock()
                .unwrap()
                .connections
                .get(name)
                .map(|c| c.brokers.clone())
                .ok_or_else(|| not_connected("kafka", name))
        };
        match method {
            // Opens the Kafka client and probes the broker's metadata once
            // (fails loudly here rather than on the first `topics`/`tail`).
            "connect" => {
                let name = str_param(params, "connection")?;
                if self.is_connected(name) {
                    return Ok(json!({"connected": true, "already": true}));
                }
                let saved = ctx.saved(name, "kafka")?;
                let conn = connect(&saved.target).await.map_err(|e| {
                    RpcError::app(format!("không kết nối được tới `{}`: {e}", saved.target))
                })?;
                self.state
                    .lock()
                    .unwrap()
                    .connections
                    .insert(name.to_string(), conn);
                Ok(json!({"connected": true, "already": false}))
            }
            "disconnect" => {
                let name = str_param(params, "connection")?;
                let removed = self.close(name);
                Ok(json!({"disconnected": removed}))
            }
            "topics" => {
                let brokers = brokers_of(str_param(params, "connection")?)?;
                let topics = list_topics(&brokers)
                    .await
                    .map_err(|e| RpcError::app(e.to_string()))?;
                Ok(json!(topics))
            }
            // One background subscription per call, never shared with
            // another client tailing the same topic. Returns immediately
            // with a `subscription` id; messages arrive later as
            // `kafka.tail.message` notifications on the caller's socket.
            "tail.start" => {
                let name = str_param(params, "connection")?;
                let topic = str_param(params, "topic")?.to_string();
                let from_beginning = params.get("from").and_then(Value::as_str) == Some("earliest");
                let brokers = brokers_of(name)?;

                let subscription = self.state.lock().unwrap().tails.next_id();
                let client_id = ctx.client.id;
                let push_client = ctx.client.clone();
                let handle = start_tail(
                    brokers,
                    topic,
                    from_beginning,
                    subscription.clone(),
                    move |item| push_client.notify("kafka.tail.message", item),
                );

                self.state.lock().unwrap().tails.insert(
                    subscription.clone(),
                    client_id,
                    (),
                    handle,
                );
                Ok(json!({"subscription": subscription}))
            }
            "tail.stop" => {
                let subscription = str_param(params, "subscription")?;
                let handle = self.state.lock().unwrap().tails.remove(subscription);
                let stopped = handle.is_some();
                if let Some(handle) = handle {
                    handle.stop();
                }
                Ok(json!({"stopped": stopped}))
            }
            _ => Err(unknown_method("kafka", method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `push_notification` is the one piece of this module testable
    /// without a real broker: the ring-buffer/drop-oldest bookkeeping.
    /// The Kafka wire-level parts (`connect`/`list_topics`/`start_tail`'s
    /// own consumer setup) need a live broker -- not available in this
    /// sandbox (no Docker daemon), same limitation every other
    /// Docker-gated connector test in this workspace already has. See the
    /// backlog entry's "Chưa làm".
    #[test]
    fn push_notification_drops_the_oldest_past_the_cap() {
        let buffer: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
        let ready = Notify::new();
        for i in 0..(MAX_BUFFERED_MESSAGES + 10) {
            push_notification(&buffer, &ready, json!({"i": i}));
        }
        let q = buffer.lock().unwrap();
        assert_eq!(q.len(), MAX_BUFFERED_MESSAGES);
        assert_eq!(q.front().unwrap()["i"], 10);
        assert_eq!(q.back().unwrap()["i"], MAX_BUFFERED_MESSAGES + 9);
    }

    #[tokio::test]
    async fn push_notification_notifies_a_waiter() {
        let buffer: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
        let ready = Notify::new();
        push_notification(&buffer, &ready, json!({"ok": true}));
        // A permit must already be stored -- `notified()` resolves without
        // ever needing a concurrent waiter to race against.
        tokio::time::timeout(Duration::from_millis(200), ready.notified())
            .await
            .expect("a stored permit must make notified() resolve quickly");
    }
}
