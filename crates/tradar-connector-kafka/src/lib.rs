//! Kafka connector -- like `tradar-connector-rabbitmq`, does not implement
//! `QueryDriver`/reuse `tradar-query-workbench` (Kafka has no query
//! language either). Unlike RabbitMQ, this one *does* need a real-time
//! firehose: tailing a topic is exactly the "Kafka consumer of thousands
//! of messages/sec" example `docs/architecture.md`'s "Screen không bao giờ
//! làm IO" section uses to justify the bounded-per-tick-drain design, so
//! this crate is the first real exercise of that path.
//!
//! v1 scope was Topics mode only (tail + publish); Groups mode (per-
//! partition consumer-group lag, added 2026-10-01) is the second mode --
//! see `docs/backlog/kafka-groups-mode-2026-10-01.md`.
//!
//! Exposes only `connector()`; everything else is this crate's own
//! business.

mod screen;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer, DefaultConsumerContext, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::topic_partition_list::{Offset, TopicPartitionList};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use tradar_connector_spi::{Connector, ConnectorDescriptor, Session};
use tradar_core::action::{Action, Component};
use tradar_core::capability::Capability;
use tradar_core::storage::SavedConnection;

pub(crate) use screen::KafkaScreen;

/// Bounded per `tick()` call -- see the module doc comment and "Screen
/// không bao giờ làm IO" in docs/architecture.md. This is the one connector
/// in the workspace where that bound is load-bearing rather than a
/// formality.
const MAX_DRAIN_PER_TICK: usize = 64;

/// How many of the most recent tailed messages `KafkaSession` keeps.
/// Older ones are dropped -- this is a live tail, not a durable log
/// viewer, and an unbounded buffer would leak memory on a busy topic left
/// running overnight.
const MAX_BUFFERED_MESSAGES: usize = 500;

const CLUSTER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) struct TopicInfo {
    pub name: String,
    pub partitions: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct KafkaMessageRow {
    pub partition: i32,
    pub offset: i64,
    pub key: Option<String>,
    pub value: String,
}

/// One row of `fetch_group_list`'s result -- just enough to populate the
/// Groups sidebar. Member assignments are raw protocol-encoded bytes
/// (format depends on which partition assignor the group's members
/// negotiated), not worth decoding just to list which topics a group
/// touches when `fetch_group_lag` below gets the same answer, and more,
/// straight from the broker's own offset bookkeeping.
#[derive(Debug, Clone)]
pub(crate) struct ConsumerGroupInfo {
    pub name: String,
    pub state: String,
    pub members: usize,
}

/// One partition's lag for a consumer group -- `fetch_group_lag`'s result.
#[derive(Debug, Clone)]
pub(crate) struct GroupLagRow {
    pub topic: String,
    pub partition: i32,
    pub committed: i64,
    pub high_watermark: i64,
}

impl GroupLagRow {
    pub(crate) fn lag(&self) -> i64 {
        self.high_watermark - self.committed
    }
}

pub(crate) enum KafkaEvent {
    Topics(anyhow::Result<Vec<TopicInfo>>),
    Groups(anyhow::Result<Vec<ConsumerGroupInfo>>),
    GroupLag(anyhow::Result<Vec<GroupLagRow>>),
    Message(KafkaMessageRow),
    TailFailed(String),
    Published(anyhow::Result<()>),
}

fn ephemeral_group_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    // Never joins a real consumer group -- every tail session gets its own,
    // so tailing a topic never steals partitions from (or rebalances) an
    // application's actual consumers.
    format!("tradar-tail-{now}-{n}")
}

pub struct KafkaSession {
    brokers: String,
    producer: FutureProducer,
    metadata_client: Arc<BaseConsumer>,
    event_tx: UnboundedSender<KafkaEvent>,
    event_rx: UnboundedReceiver<KafkaEvent>,
    tail_handle: Option<JoinHandle<()>>,
    pub(crate) topics: Vec<TopicInfo>,
    pub(crate) groups: Vec<ConsumerGroupInfo>,
    pub(crate) group_lag: Vec<GroupLagRow>,
    pub(crate) messages: VecDeque<KafkaMessageRow>,
    pub(crate) tailing_topic: Option<String>,
    pub(crate) publishing: bool,
    pub(crate) error: Option<String>,
}

impl KafkaSession {
    fn new(brokers: String, producer: FutureProducer, metadata_client: Arc<BaseConsumer>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let session = Self {
            brokers,
            producer,
            metadata_client,
            event_tx,
            event_rx,
            tail_handle: None,
            topics: Vec::new(),
            groups: Vec::new(),
            group_lag: Vec::new(),
            messages: VecDeque::new(),
            tailing_topic: None,
            publishing: false,
            error: None,
        };
        session.list_topics();
        session.list_groups();
        session
    }

    pub(crate) fn list_topics(&self) {
        let metadata_client = Arc::clone(&self.metadata_client);
        let tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = metadata_client
                .fetch_metadata(None, CLUSTER_TIMEOUT)
                .map(|metadata| {
                    metadata
                        .topics()
                        .iter()
                        // Kafka's own internal bookkeeping topics aren't
                        // anything a user browsing their own data wants to
                        // see -- same reasoning as skipping Cassandra's
                        // `system*` keyspaces.
                        .filter(|t| !t.name().starts_with("__"))
                        .map(|t| TopicInfo {
                            name: t.name().to_string(),
                            partitions: t.partitions().len(),
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(anyhow::Error::from);
            let _ = tx.send(KafkaEvent::Topics(result));
        });
    }

    pub(crate) fn list_groups(&self) {
        let metadata_client = Arc::clone(&self.metadata_client);
        let tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = metadata_client
                .fetch_group_list(None, CLUSTER_TIMEOUT)
                .map(|list| {
                    list.groups()
                        .iter()
                        .map(|g| ConsumerGroupInfo {
                            name: g.name().to_string(),
                            state: g.state().to_string(),
                            members: g.members().len(),
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(anyhow::Error::from);
            let _ = tx.send(KafkaEvent::Groups(result));
        });
    }

    /// Computes per-partition lag for `group`: a throwaway consumer
    /// configured with that `group.id` -- never subscribed or assigned, so
    /// it never joins the group's active membership, rebalances it, or
    /// disturbs its real consumers -- asks the broker's group coordinator
    /// for the group's committed offsets directly (the `OffsetFetch`
    /// request, the same one `kafka-consumer-groups.sh --describe` makes;
    /// unlike `Consumer::poll`, it doesn't require being a joined member).
    /// Every partition of every known topic is queried; the ones this group
    /// has never committed on come back `Offset::Invalid` and are filtered
    /// out, leaving exactly the partitions it actually consumes. Read-only:
    /// never seeks or commits anything, matching the non-goal already on
    /// record for this connector (no offset seek/reset from the UI).
    pub(crate) fn fetch_group_lag(&self, group: &str) {
        let brokers = self.brokers.clone();
        let group = group.to_string();
        let topics = self.topics.clone();
        let tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = (|| -> anyhow::Result<Vec<GroupLagRow>> {
                let mut config = ClientConfig::new();
                config
                    .set("bootstrap.servers", &brokers)
                    .set("group.id", &group);
                let consumer: BaseConsumer = config.create()?;

                let mut tpl = TopicPartitionList::new();
                for topic in &topics {
                    if topic.partitions > 0 {
                        tpl.add_partition_range(&topic.name, 0, topic.partitions as i32 - 1);
                    }
                }
                let committed = consumer.committed_offsets(tpl, CLUSTER_TIMEOUT)?;

                let mut rows = Vec::new();
                for elem in committed.elements() {
                    let Offset::Offset(committed_offset) = elem.offset() else {
                        continue;
                    };
                    let (_, high_watermark) = consumer.fetch_watermarks(
                        elem.topic(),
                        elem.partition(),
                        CLUSTER_TIMEOUT,
                    )?;
                    rows.push(GroupLagRow {
                        topic: elem.topic().to_string(),
                        partition: elem.partition(),
                        committed: committed_offset,
                        high_watermark,
                    });
                }
                rows.sort_by(|a, b| a.topic.cmp(&b.topic).then(a.partition.cmp(&b.partition)));
                Ok(rows)
            })();
            let _ = tx.send(KafkaEvent::GroupLag(result));
        });
    }

    /// Starts tailing `topic` from `from_beginning ? Beginning : End`,
    /// replacing whatever tail was previously running. Each call gets its
    /// own throwaway consumer/group -- see `ephemeral_group_id`.
    pub(crate) fn start_tail(&mut self, topic: &str, from_beginning: bool) {
        if let Some(handle) = self.tail_handle.take() {
            handle.abort();
        }
        self.messages.clear();
        self.tailing_topic = Some(topic.to_string());

        let mut config = ClientConfig::new();
        config
            .set("bootstrap.servers", &self.brokers)
            .set("group.id", ephemeral_group_id())
            .set("enable.auto.commit", "false");
        let topic = topic.to_string();
        let tx = self.event_tx.clone();

        self.tail_handle = Some(tokio::spawn(async move {
            let consumer: StreamConsumer<DefaultConsumerContext> = match config.create() {
                Ok(consumer) => consumer,
                Err(e) => {
                    let _ = tx.send(KafkaEvent::TailFailed(e.to_string()));
                    return;
                }
            };
            let metadata = match consumer.fetch_metadata(Some(&topic), CLUSTER_TIMEOUT) {
                Ok(metadata) => metadata,
                Err(e) => {
                    let _ = tx.send(KafkaEvent::TailFailed(e.to_string()));
                    return;
                }
            };
            let Some(topic_metadata) = metadata.topics().iter().find(|t| t.name() == topic) else {
                let _ = tx.send(KafkaEvent::TailFailed(format!("unknown topic '{topic}'")));
                return;
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
                let _ = tx.send(KafkaEvent::TailFailed(e.to_string()));
                return;
            }

            let mut stream = consumer.stream();
            while let Some(next) = stream.next().await {
                match next {
                    Ok(message) => {
                        let row = KafkaMessageRow {
                            partition: message.partition(),
                            offset: message.offset(),
                            key: message
                                .key()
                                .map(|k| String::from_utf8_lossy(k).into_owned()),
                            value: message
                                .payload()
                                .map(|p| String::from_utf8_lossy(p).into_owned())
                                .unwrap_or_default(),
                        };
                        if tx.send(KafkaEvent::Message(row)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(KafkaEvent::TailFailed(e.to_string()));
                    }
                }
            }
        }));
    }

    pub(crate) fn publish(&mut self, topic: &str, key: Option<&str>, payload: &str) {
        self.publishing = true;
        let producer = self.producer.clone();
        let topic = topic.to_string();
        let key = key.map(str::to_string);
        let payload = payload.to_string();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut record = FutureRecord::to(&topic).payload(&payload);
            if let Some(key) = &key {
                record = record.key(key);
            }
            let result = producer
                .send(record, Duration::from_secs(5))
                .await
                .map(|_| ())
                .map_err(|(e, _)| anyhow::Error::from(e));
            let _ = tx.send(KafkaEvent::Published(result));
        });
    }
}

impl Session for KafkaSession {
    fn tick(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..MAX_DRAIN_PER_TICK {
            let event = match self.event_rx.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            changed = true;
            match event {
                KafkaEvent::Topics(Ok(topics)) => {
                    self.topics = topics;
                    self.error = None;
                }
                KafkaEvent::Topics(Err(e)) => self.error = Some(e.to_string()),
                KafkaEvent::Groups(Ok(groups)) => {
                    self.groups = groups;
                    self.error = None;
                }
                KafkaEvent::Groups(Err(e)) => self.error = Some(e.to_string()),
                KafkaEvent::GroupLag(Ok(rows)) => {
                    self.group_lag = rows;
                    self.error = None;
                }
                KafkaEvent::GroupLag(Err(e)) => self.error = Some(e.to_string()),
                KafkaEvent::Message(row) => {
                    self.messages.push_back(row);
                    while self.messages.len() > MAX_BUFFERED_MESSAGES {
                        self.messages.pop_front();
                    }
                }
                KafkaEvent::TailFailed(error) => self.error = Some(error),
                KafkaEvent::Published(Ok(())) => {
                    self.publishing = false;
                    self.error = None;
                }
                KafkaEvent::Published(Err(e)) => {
                    self.publishing = false;
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
        Box::new(KafkaScreen::new(*self, action_tx))
    }
}

const DESCRIPTOR: ConnectorDescriptor = ConnectorDescriptor {
    id: "kafka",
    display_name: "Kafka",
    icon: "📨",
    capabilities: &[Capability::Streaming, Capability::Publish, Capability::Tail],
};

struct KafkaConnector;

#[async_trait]
impl Connector for KafkaConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &DESCRIPTOR
    }

    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>> {
        let brokers = connection.target.clone();
        let mut config = ClientConfig::new();
        config.set("bootstrap.servers", &brokers);

        let metadata_client: BaseConsumer = config
            .create()
            .map_err(|e| anyhow::anyhow!("could not build a Kafka client for '{brokers}': {e}"))?;
        let metadata_client = Arc::new(metadata_client);
        let producer: FutureProducer = config.create().map_err(|e| {
            anyhow::anyhow!("could not build a Kafka producer for '{brokers}': {e}")
        })?;

        let probe = Arc::clone(&metadata_client);
        tradar_connector_spi::with_connect_timeout(&brokers, async move {
            tokio::task::spawn_blocking(move || probe.fetch_metadata(None, CLUSTER_TIMEOUT))
                .await
                .map_err(anyhow::Error::from)?
                .map_err(anyhow::Error::from)?;
            Ok(())
        })
        .await?;

        let session = KafkaSession::new(brokers, producer, metadata_client);
        Ok(Box::new(session))
    }
}

pub fn connector() -> Box<dyn Connector> {
    Box::new(KafkaConnector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_declares_streaming_publish_and_tail() {
        assert_eq!(DESCRIPTOR.id, "kafka");
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Streaming));
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Publish));
        assert!(DESCRIPTOR.capabilities.contains(&Capability::Tail));
    }

    #[test]
    fn ephemeral_group_ids_never_repeat() {
        let a = ephemeral_group_id();
        let b = ephemeral_group_id();

        assert_ne!(a, b);
        assert!(a.starts_with("tradar-tail-"));
    }

    #[test]
    fn group_lag_row_computes_lag_as_high_watermark_minus_committed() {
        let row = GroupLagRow {
            topic: "orders".to_string(),
            partition: 0,
            committed: 7,
            high_watermark: 10,
        };

        assert_eq!(row.lag(), 3);
    }

    mod docker {
        //! Integration test against a real Kafka broker, via
        //! `testcontainers::GenericImage` mirroring the KRaft single-node
        //! setup already in `docker-compose.yml`'s `kafka` service --
        //! `testcontainers-modules`' Kafka support targets Confluent's
        //! images/ZooKeeper mode, not `apache/kafka`'s KRaft layout, so
        //! this crate builds its own image config from scratch rather than
        //! using it, same reasoning as Cassandra/RabbitMQ's `mod docker`.

        use std::time::Duration;

        use rdkafka::consumer::CommitMode;
        use testcontainers::core::{IntoContainerPort, WaitFor};
        use testcontainers::runners::AsyncRunner;
        use testcontainers::{ContainerAsync, GenericImage, ImageExt};

        use super::*;

        /// Needs host port 9092 free -- can't run alongside the long-lived
        /// dev instance from `docker compose up kafka`, same constraint as
        /// Cassandra/RabbitMQ's docker tests. Kafka always advertises
        /// exactly the address/port it's told to via
        /// `KAFKA_ADVERTISED_LISTENERS`, so -- like Cassandra's
        /// `broadcast_rpc_address` -- the host port has to match what's
        /// advertised (`localhost:9092`), not a random one.
        async fn connected_session() -> (ContainerAsync<GenericImage>, KafkaSession) {
            let container = GenericImage::new("apache/kafka", "4.0.1")
                .with_wait_for(WaitFor::message_on_stdout("Kafka Server started"))
                .with_mapped_port(9092, 9092.tcp())
                .with_env_var("KAFKA_NODE_ID", "1")
                .with_env_var("KAFKA_PROCESS_ROLES", "broker,controller")
                .with_env_var("KAFKA_CONTROLLER_QUORUM_VOTERS", "1@localhost:29093")
                .with_env_var(
                    "KAFKA_LISTENERS",
                    "PLAINTEXT://:29092,CONTROLLER://:29093,EXTERNAL://:9092",
                )
                .with_env_var(
                    "KAFKA_ADVERTISED_LISTENERS",
                    "PLAINTEXT://localhost:29092,EXTERNAL://localhost:9092",
                )
                .with_env_var(
                    "KAFKA_LISTENER_SECURITY_PROTOCOL_MAP",
                    "PLAINTEXT:PLAINTEXT,CONTROLLER:PLAINTEXT,EXTERNAL:PLAINTEXT",
                )
                .with_env_var("KAFKA_INTER_BROKER_LISTENER_NAME", "PLAINTEXT")
                .with_env_var("KAFKA_CONTROLLER_LISTENER_NAMES", "CONTROLLER")
                .with_env_var("KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR", "1")
                .with_env_var("KAFKA_TRANSACTION_STATE_LOG_REPLICATION_FACTOR", "1")
                .with_env_var("KAFKA_TRANSACTION_STATE_LOG_MIN_ISR", "1")
                .with_env_var("KAFKA_GROUP_INITIAL_REBALANCE_DELAY_MS", "0")
                .with_env_var("KAFKA_NUM_PARTITIONS", "1")
                .with_startup_timeout(Duration::from_secs(120))
                .start()
                .await
                .expect("kafka container must start");

            let brokers = "localhost:9092".to_string();
            let mut config = ClientConfig::new();
            config.set("bootstrap.servers", &brokers);
            let metadata_client: BaseConsumer = config.create().expect("client must build");
            let metadata_client = Arc::new(metadata_client);
            let producer: FutureProducer = config.create().expect("producer must build");

            let mut last_error = None;
            for _ in 0..10 {
                let client = Arc::clone(&metadata_client);
                match tokio::task::spawn_blocking(move || {
                    client.fetch_metadata(None, Duration::from_secs(5))
                })
                .await
                {
                    Ok(Ok(_)) => {
                        last_error = None;
                        break;
                    }
                    Ok(Err(e)) => last_error = Some(e.to_string()),
                    Err(e) => last_error = Some(e.to_string()),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            if let Some(error) = last_error {
                panic!("kafka broker never became ready: {error}");
            }

            let session = KafkaSession::new(brokers, producer, metadata_client);
            (container, session)
        }

        async fn drain_until<T>(
            session: &mut KafkaSession,
            timeout: Duration,
            mut ready: impl FnMut(&KafkaSession) -> Option<T>,
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

        // One test, one container: the fixed host port (9092, forced by
        // Kafka always advertising exactly what it's told to -- see
        // `connected_session`'s doc comment) means a second concurrently-
        // started container can't bind it, so this covers the tail round
        // trip, `list_topics`, and (2026-10-01) Groups mode's `list_groups`/
        // `fetch_group_lag` all in one test rather than racing multiple
        // `#[tokio::test]`s against the same port.
        #[tokio::test]
        async fn tail_publish_list_topics_and_group_lag_round_trip_through_a_real_broker() {
            let (_container, mut session) =
                tokio::time::timeout(Duration::from_secs(150), connected_session())
                    .await
                    .expect("container must become ready within 150s");

            session.publish("events", Some("user-1"), "signed-up");
            drain_until(&mut session, Duration::from_secs(10), |s| {
                (!s.publishing && s.error.is_none()).then_some(())
            })
            .await;

            session.start_tail("events", true);
            let row = drain_until(&mut session, Duration::from_secs(20), |s| {
                s.messages.front().cloned()
            })
            .await;
            assert_eq!(row.key.as_deref(), Some("user-1"));
            assert_eq!(row.value, "signed-up");

            session.publish("notifications", None, "hello");
            drain_until(&mut session, Duration::from_secs(10), |s| {
                (!s.publishing && s.error.is_none()).then_some(())
            })
            .await;
            session.list_topics();
            let topics = drain_until(&mut session, Duration::from_secs(10), |s| {
                s.topics
                    .iter()
                    .any(|t| t.name == "notifications")
                    .then(|| s.topics.clone())
            })
            .await;
            assert!(topics.iter().any(|t| t.name == "notifications"));

            // Groups mode: a real consumer (not the ephemeral tail consumer
            // above, which never commits) joins a fixed group, consumes the
            // one "events" message, and commits -- giving `fetch_group_lag`
            // a real committed offset to find via the broker's group
            // coordinator, exactly like the production code path.
            let mut config = ClientConfig::new();
            config
                .set("bootstrap.servers", "localhost:9092")
                .set("group.id", "lag-test-group")
                .set("auto.offset.reset", "earliest")
                .set("enable.auto.commit", "false");
            let lag_consumer: StreamConsumer = config.create().expect("consumer must build");
            lag_consumer
                .subscribe(&["events"])
                .expect("must subscribe to events");
            let message = tokio::time::timeout(Duration::from_secs(20), lag_consumer.recv())
                .await
                .expect("must receive the message within 20s")
                .expect("receive must not error");
            lag_consumer
                .commit_message(&message, CommitMode::Sync)
                .expect("commit must succeed");
            drop(message);
            drop(lag_consumer);

            session.list_groups();
            drain_until(&mut session, Duration::from_secs(10), |s| {
                s.groups
                    .iter()
                    .any(|g| g.name == "lag-test-group")
                    .then_some(())
            })
            .await;

            session.fetch_group_lag("lag-test-group");
            let lag_rows = drain_until(&mut session, Duration::from_secs(10), |s| {
                (!s.group_lag.is_empty()).then(|| s.group_lag.clone())
            })
            .await;
            let events_row = lag_rows
                .iter()
                .find(|r| r.topic == "events")
                .expect("lag-test-group must have committed on events");
            assert_eq!(events_row.partition, 0);
            assert_eq!(
                events_row.committed, 1,
                "committed the one message consumed"
            );
            assert_eq!(
                events_row.high_watermark, 1,
                "only one message was ever produced to events"
            );
            assert_eq!(events_row.lag(), 0, "consumer caught up to the end");
        }
    }
}
