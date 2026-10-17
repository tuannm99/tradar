//! End-to-end for `kafka.*` against a real broker (`apache/kafka` in KRaft
//! mode, via `testcontainers::GenericImage` -- same single-node setup as
//! `tradar-connector-kafka`'s own `mod docker`), driven through
//! `Server::handle` with a real per-client outbox so the pushed
//! `kafka.tail.message` notifications arrive exactly as they would on a
//! socket. Needs a Docker daemon (`make test-kafka-server`).
//!
//! One test, not several: the broker has to advertise a fixed host port, so
//! two tests starting their own container would collide on it.
//!
//! Kafka advertises exactly the address it is told to, so the host port must
//! match (`localhost:9092`). On a Docker without a bridge network (some
//! sandboxes) published ports don't work: set `TRADAR_DOCKER_HOST_NETWORK=1`
//! to run the container on the host network instead.
#![cfg(feature = "kafka")]

use std::time::Duration;

use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde_json::{Value, json};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tradar_core::storage::{ConnectionStore, SavedConnection};
use tradar_server::{ClientConnection, Server, registry};

const BROKERS: &str = "localhost:9092";
const TOPIC: &str = "tradar-e2e";

async fn start_broker() -> ContainerAsync<GenericImage> {
    let request = GenericImage::new("apache/kafka", "4.0.1")
        .with_wait_for(WaitFor::message_on_stdout("Kafka Server started"))
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
        .with_startup_timeout(Duration::from_secs(120));
    let request = if std::env::var_os("TRADAR_DOCKER_HOST_NETWORK").is_some() {
        request.with_network("host")
    } else {
        request.with_mapped_port(9092, 9092.tcp())
    };
    request.start().await.expect("kafka container must start")
}

async fn produce(producer: &FutureProducer, key: &str, value: &str) {
    producer
        .send(
            FutureRecord::to(TOPIC).key(key).payload(value),
            Duration::from_secs(10),
        )
        .await
        .unwrap_or_else(|(e, _)| panic!("produce failed: {e}"));
}

async fn call(server: &Server, client: &ClientConnection, method: &str, params: Value) -> Value {
    server
        .handle(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            client,
        )
        .await
}

fn ok(response: &Value) -> &Value {
    assert!(
        response.get("error").is_none(),
        "unexpected error: {response}"
    );
    &response["result"]
}

/// The next `kafka.tail.message` pushed for `subscription`, or `None` if
/// nothing arrives within `timeout`.
async fn next_message(
    outbox: &mut UnboundedReceiver<String>,
    subscription: &str,
    timeout: Duration,
) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        let line = tokio::time::timeout(left, outbox.recv()).await.ok()??;
        let note: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(note["method"], "kafka.tail.message", "{note}");
        assert!(note.get("id").is_none(), "a push has no id: {note}");
        if note["params"]["subscription"] == subscription {
            return Some(note["params"].clone());
        }
    }
}

#[tokio::test]
async fn tail_lifecycle_through_a_real_broker() {
    let _broker = start_broker().await;

    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.toml"));
    store
        .save(&[SavedConnection {
            name: "bus".into(),
            driver: "kafka".into(),
            target: BROKERS.into(),
        }])
        .unwrap();
    let server = Server::new(registry(), Some(store));
    let (outbox_tx, mut outbox) = unbounded_channel::<String>();
    let client = server.new_client(outbox_tx);

    // -- connect (retry: the port can answer before metadata is ready) ----
    let mut connected = None;
    for _ in 0..15 {
        let response = call(
            &server,
            &client,
            "kafka.connect",
            json!({"connection": "bus"}),
        )
        .await;
        if response.get("error").is_none() {
            connected = Some(response);
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let connected = connected.expect("kafka.connect never succeeded");
    assert_eq!(connected["result"]["already"], false);
    let again = call(
        &server,
        &client,
        "kafka.connect",
        json!({"connection": "bus"}),
    )
    .await;
    assert_eq!(ok(&again)["already"], true);

    let list = call(&server, &client, "connections.list", Value::Null).await;
    assert_eq!(ok(&list)[0]["connected"], true);
    assert_eq!(ok(&list)[0]["supported"], true);

    // -- topics: the seeded one is listed, internal ones are hidden --------
    let mut config = ClientConfig::new();
    config.set("bootstrap.servers", BROKERS);
    let producer: FutureProducer = config.create().expect("producer must build");
    produce(&producer, "k1", "seed").await;

    let topics = call(
        &server,
        &client,
        "kafka.topics",
        json!({"connection": "bus"}),
    )
    .await;
    let topics: Vec<&str> = ok(&topics)
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(topics.contains(&TOPIC), "{topics:?}");
    assert!(
        topics.iter().all(|t| !t.starts_with("__")),
        "internal topics must be hidden: {topics:?}"
    );

    // -- tail from earliest: gets the seed, then a live message ------------
    let started = call(
        &server,
        &client,
        "kafka.tail.start",
        json!({"connection": "bus", "topic": TOPIC, "from": "earliest"}),
    )
    .await;
    let sub = ok(&started)["subscription"].as_str().unwrap().to_string();

    let seed = next_message(&mut outbox, &sub, Duration::from_secs(30))
        .await
        .expect("the seeded message must arrive");
    assert_eq!(seed["value"], "seed");
    assert_eq!(seed["key"], "k1");
    assert_eq!(seed["partition"], 0);

    produce(&producer, "k2", "live").await;
    let live = next_message(&mut outbox, &sub, Duration::from_secs(30))
        .await
        .expect("a message produced during the tail must arrive");
    assert_eq!(live["value"], "live");
    assert_eq!(live["offset"], seed["offset"].as_i64().unwrap() + 1);

    // -- stop: reported once, and nothing more is pushed -------------------
    let stopped = call(
        &server,
        &client,
        "kafka.tail.stop",
        json!({"subscription": sub}),
    )
    .await;
    assert_eq!(ok(&stopped)["stopped"], true);
    let stopped_again = call(
        &server,
        &client,
        "kafka.tail.stop",
        json!({"subscription": sub}),
    )
    .await;
    assert_eq!(ok(&stopped_again)["stopped"], false);
    produce(&producer, "k3", "after-stop").await;
    assert!(
        next_message(&mut outbox, &sub, Duration::from_secs(4))
            .await
            .is_none(),
        "a stopped tail must push nothing"
    );

    // -- client disconnect cleans up tails it never stopped ----------------
    let started = call(
        &server,
        &client,
        "kafka.tail.start",
        json!({"connection": "bus", "topic": TOPIC, "from": "latest"}),
    )
    .await;
    let orphan = ok(&started)["subscription"].as_str().unwrap().to_string();
    assert_ne!(orphan, sub, "subscription ids are never reused");
    tokio::time::sleep(Duration::from_secs(3)).await; // let it get assigned
    server.client_disconnected(&client);
    produce(&producer, "k4", "after-disconnect").await;
    assert!(
        next_message(&mut outbox, &orphan, Duration::from_secs(4))
            .await
            .is_none(),
        "a disconnected client's tail must be aborted"
    );

    // -- disconnect forgets the connection ---------------------------------
    let gone = call(
        &server,
        &client,
        "kafka.disconnect",
        json!({"connection": "bus"}),
    )
    .await;
    assert_eq!(ok(&gone)["disconnected"], true);
    let list = call(&server, &client, "connections.list", Value::Null).await;
    assert_eq!(ok(&list)[0]["connected"], false);
    let topics = call(
        &server,
        &client,
        "kafka.topics",
        json!({"connection": "bus"}),
    )
    .await;
    assert!(topics.get("error").is_some(), "{topics}");
}
