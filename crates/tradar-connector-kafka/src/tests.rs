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
    let a = ephemeral_group_id("tradar-tail");
    let b = ephemeral_group_id("tradar-tail");

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
