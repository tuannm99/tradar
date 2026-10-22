//! The deprecated TUI's side of this connector: its `Session`, `Connector`
//! and event plumbing. Built only with the `tui` feature; `tradar-server`
//! uses the shared, TUI-free parts in `tail` instead.

use crate::tail::*;
use std::collections::VecDeque;
use std::sync::Arc;
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

pub(crate) use crate::screen::KafkaScreen;

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

pub struct KafkaSession {
    pub(crate) brokers: String,
    pub(crate) producer: FutureProducer,
    pub(crate) metadata_client: Arc<BaseConsumer>,
    pub(crate) event_tx: UnboundedSender<KafkaEvent>,
    pub(crate) event_rx: UnboundedReceiver<KafkaEvent>,
    pub(crate) tail_handle: Option<JoinHandle<()>>,
    pub(crate) topics: Vec<TopicInfo>,
    pub(crate) groups: Vec<ConsumerGroupInfo>,
    pub(crate) group_lag: Vec<GroupLagRow>,
    pub(crate) messages: VecDeque<KafkaMessageRow>,
    pub(crate) tailing_topic: Option<String>,
    pub(crate) publishing: bool,
    pub(crate) error: Option<String>,
}

impl KafkaSession {
    pub(crate) fn new(
        brokers: String,
        producer: FutureProducer,
        metadata_client: Arc<BaseConsumer>,
    ) -> Self {
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
                            members: member_count(g),
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
            .set("group.id", ephemeral_group_id("tradar-tail"))
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

/// `g.members().len()`, without calling `members()` on a group that has
/// none: rdkafka (0.36 through 0.39) builds that slice with
/// `slice::from_raw_parts(members, member_cnt)`, and librdkafka leaves
/// `members` null when the count is 0 -- undefined behavior, which a debug
/// build turns into an abort (found by the CI Docker job). Kafka's own
/// definition of an `Empty`/`Dead` group is one with no members, and a group
/// in a rebalance may be momentarily memberless, so only a `Stable` or
/// `CompletingRebalance` group is asked; the rest count as 0.
fn member_count(g: &rdkafka::groups::GroupInfo) -> usize {
    match g.state() {
        "Stable" | "CompletingRebalance" => g.members().len(),
        _ => 0,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
