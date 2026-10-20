use super::*;

#[test]
fn a_url_with_credentials_and_no_path_defaults_to_the_default_vhost() {
    let target = parse_target("http://user:password@localhost:15672").unwrap();

    assert_eq!(target.base_url, "http://localhost:15672");
    assert_eq!(
        target.auth,
        Some(("user".to_string(), "password".to_string()))
    );
    assert_eq!(target.vhost, "%2f");
}

#[test]
fn a_percent_encoded_user_and_password_are_decoded_for_basic_auth() {
    let target = parse_target("http://me%40corp:p%40ss%3Aw%2Frd@localhost:15672/%2f").unwrap();

    assert_eq!(
        target.auth,
        Some(("me@corp".to_string(), "p@ss:w/rd".to_string()))
    );
    // The vhost keeps its encoding: it is spliced into a URL path as is.
    assert_eq!(target.vhost, "%2f");
}

#[test]
fn a_url_with_an_explicit_encoded_default_vhost_is_passed_through() {
    let target = parse_target("http://user:password@localhost:15672/%2f").unwrap();

    assert_eq!(target.vhost, "%2f");
}

#[test]
fn a_url_with_a_custom_vhost_keeps_it() {
    let target = parse_target("http://user:password@localhost:15672/staging").unwrap();

    assert_eq!(target.vhost, "staging");
}

#[test]
fn a_url_without_credentials_has_no_auth() {
    let target = parse_target("http://localhost:15672").unwrap();

    assert_eq!(target.auth, None);
}

#[test]
fn an_unparseable_target_is_a_clear_error_not_a_panic() {
    let error = parse_target("not a url").unwrap_err();

    assert!(error.to_string().contains("not a url"), "{error}");
}

#[test]
fn queue_info_parses_the_fields_the_screen_actually_shows() {
    let json = r#"{"name": "orders", "messages_ready": 3, "messages_unacknowledged": 1, "consumers": 2, "extra_field_ignored": true}"#;

    let queue: QueueInfo = serde_json::from_str(json).unwrap();

    assert_eq!(queue.name, "orders");
    assert_eq!(queue.messages_ready, 3);
    assert_eq!(queue.messages_unacknowledged, 1);
    assert_eq!(queue.consumers, 2);
}

#[test]
fn exchange_info_maps_the_reserved_type_field_name() {
    let json = r#"{"name": "orders.topic", "type": "topic"}"#;

    let exchange: ExchangeInfo = serde_json::from_str(json).unwrap();

    assert_eq!(exchange.name, "orders.topic");
    assert_eq!(exchange.kind, "topic");
}

#[test]
fn descriptor_declares_schema_and_publish_but_not_streaming_or_tail() {
    assert_eq!(DESCRIPTOR.id, "rabbitmq");
    assert!(DESCRIPTOR.capabilities.contains(&Capability::Schema));
    assert!(DESCRIPTOR.capabilities.contains(&Capability::Publish));
    assert!(!DESCRIPTOR.capabilities.contains(&Capability::Streaming));
    assert!(!DESCRIPTOR.capabilities.contains(&Capability::Tail));
}

mod docker {
    //! Integration tests against a real RabbitMQ, via `testcontainers`
    //! directly -- same reasoning as Cassandra's `mod docker`
    //! (`testcontainers-modules` has no RabbitMQ feature either, and
    //! consistency with the rest of the workspace's connectors beats
    //! saving one dependency).

    use std::time::Duration;

    use testcontainers::core::{IntoContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{ContainerAsync, GenericImage, ImageExt};

    use super::*;

    /// Needs host ports 5672/15672 free -- can't run alongside the
    /// long-lived dev instance from `docker compose up rabbitmq`, same
    /// constraint as Cassandra's docker tests.
    async fn connected_session() -> (ContainerAsync<GenericImage>, RabbitSession) {
        let container = GenericImage::new("rabbitmq", "4-management-alpine")
            .with_wait_for(WaitFor::message_on_stdout("Server startup complete"))
            .with_mapped_port(5672, 5672.tcp())
            .with_mapped_port(15672, 15672.tcp())
            .with_env_var("RABBITMQ_DEFAULT_USER", "user")
            .with_env_var("RABBITMQ_DEFAULT_PASS", "password")
            .with_startup_timeout(Duration::from_secs(90))
            .start()
            .await
            .expect("rabbitmq container must start");

        let target = parse_target("http://user:password@127.0.0.1:15672").unwrap();
        let client = reqwest::Client::new();

        // The management plugin can take a few seconds after the log
        // line above before it actually answers HTTP -- retry rather
        // than assume the very first request lands.
        let mut last_error = None;
        for _ in 0..10 {
            let mut request = client.get(format!("{}/api/overview", target.base_url));
            if let Some((user, pass)) = &target.auth {
                request = request.basic_auth(user, Some(pass));
            }
            match tokio::time::timeout(Duration::from_secs(5), request.send()).await {
                Ok(Ok(response)) if response.status().is_success() => {
                    last_error = None;
                    break;
                }
                Ok(Ok(response)) => last_error = Some(format!("status {}", response.status())),
                Ok(Err(e)) => last_error = Some(e.to_string()),
                Err(_) => last_error = Some("overview request timed out".to_string()),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        if let Some(error) = last_error {
            panic!("rabbitmq management API never became ready: {error}");
        }

        let session = RabbitSession::new(client, target);
        (container, session)
    }

    async fn declare_queue(session: &RabbitSession, queue: &str) {
        let response = session
            .request(
                reqwest::Method::PUT,
                &format!("/api/queues/{}/{queue}", session.target.vhost),
            )
            // RabbitMQ 4.x deprecates non-durable, non-exclusive queues
            // by default ("transient_nonexcl_queues") -- durable is the
            // only shape a plain `PUT /api/queues` can declare there.
            .json(&serde_json::json!({ "durable": true, "auto_delete": false }))
            .send()
            .await
            .expect("declare queue request must be sendable");
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            panic!("declaring queue '{queue}' failed: {status} {body}");
        }
    }

    async fn drain_until<T>(
        session: &mut RabbitSession,
        timeout: Duration,
        mut ready: impl FnMut(&RabbitSession) -> Option<T>,
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

    // One test, one container: two fixed host ports (5672/15672) mean a
    // second concurrently-started container can't bind them, so this
    // covers both the publish/peek round trip and `refresh_queues`
    // rather than racing two `#[tokio::test]`s against the same ports.
    #[tokio::test]
    async fn publish_peek_and_refresh_round_trip_through_a_real_broker() {
        let (_container, mut session) =
            tokio::time::timeout(Duration::from_secs(150), connected_session())
                .await
                .expect("container must become ready within 150s");

        declare_queue(&session, "orders").await;
        declare_queue(&session, "notifications").await;

        // Publish via the default exchange (name `""`), routed by queue
        // name -- avoids needing a custom exchange/binding for this test.
        session.publish("", "orders", "hello from tradar");
        drain_until(&mut session, Duration::from_secs(10), |s| {
            (!s.publishing && s.error.is_none()).then_some(())
        })
        .await;

        session.peek_messages("orders");
        let messages = drain_until(&mut session, Duration::from_secs(10), |s| {
            (!s.messages.is_empty()).then(|| s.messages.clone())
        })
        .await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, "hello from tradar");
        assert_eq!(messages[0].routing_key, "orders");

        session.refresh_queues();
        let queues = drain_until(&mut session, Duration::from_secs(10), |s| {
            s.queues
                .iter()
                .any(|q| q.name == "notifications")
                .then(|| s.queues.clone())
        })
        .await;
        assert!(queues.iter().any(|q| q.name == "notifications"));
    }
}
