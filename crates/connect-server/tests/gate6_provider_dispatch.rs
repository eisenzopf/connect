//! Actual loopback provider I/O and current authority, no provider accounts.
#![allow(clippy::unwrap_used)]
use connect_client::ClientConfig;
use connect_connectors::Vapi;
use connect_protocol::{provisioning::ProvisioningCommand, Operation, Outcome};
use connect_server::{cli::init, config::ServerConfig, providers::Worker, runtime::RunningServer};
use connect_store::{Route, Store};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

async fn request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut bytes = vec![];
    let mut part = [0; 8192];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(4), socket.read(&mut part))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&part[..n]);
        assert!(bytes.len() < 1024 * 1024);
        if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .find_map(|s| {
                    s.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return bytes;
            }
        }
    }
}
fn sample(name: &str, command: &ProvisioningCommand, status: u16, response: &serde_json::Value) {
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(std::path::PathBuf::from(directory).join(name),serde_json::to_vec_pretty(&serde_json::json!({"route":"/v2/identities/{id}/revoke","request":command,"status":status,"response":response})).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn real_provider_io_retains_authority_until_cancelled_connection_closes_and_retry_revokes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (sent, received) = oneshot::channel();
    let (closed, closure) = oneshot::channel();
    let backend = tokio::spawn(async move {
        let (mut assistant, _) = listener.accept().await.unwrap();
        let get = request(&mut assistant).await;
        assert!(get.starts_with(b"GET /assistant/"));
        let body=serde_json::to_vec(&serde_json::json!({"id":"12345678-1234-1234-1234-123456789012","model":{"messages":[{"role":"system","content":"Context: {{connect_context}}"}]}})).unwrap();
        assistant.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
        assistant.write_all(&body).await.unwrap();
        drop(assistant);
        let (mut send, _) = listener.accept().await.unwrap();
        let bytes = request(&mut send).await;
        assert!(bytes.starts_with(b"POST /chat "));
        assert!(String::from_utf8_lossy(&bytes).contains("owned-provider-body"));
        sent.send(()).unwrap();
        // No response: the owned request remains in flight until the worker cancels.
        let mut remainder = vec![];
        tokio::time::timeout(Duration::from_secs(6), send.read_to_end(&mut remainder))
            .await
            .unwrap()
            .unwrap();
        assert!(remainder.is_empty());
        closed.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let human = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    let bot = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let admin = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let Outcome::Conversation(conv) = server
        .store
        .execute(
            &human.credential_id,
            &Operation::Create {
                request_id: "owned-conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![bot
                    .identity_id
                    .clone()]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    server
        .store
        .configure_route(
            &admin.credential_id,
            &Route {
                id: "owned-route".into(),
                identity_id: bot.identity_id.clone(),
                credential_id: bot.credential_id.clone(),
                conversation_id: conv.id.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let driver = Arc::new(Vapi::new("fixture-secret", &base).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![driver.clone()])
        .await
        .unwrap();
    server
        .store
        .execute(
            &human.credential_id,
            &Operation::Send {
                request_id: "one-owned-send".into(),
                conversation_id: conv.id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("owned-provider-body"),
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    let before = Store::inspect_snapshot(server.store.path()).unwrap();
    assert!(
        Worker::with_drivers(server.service.clone(), vec![driver.clone()])
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("still owns pending work")
    );
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    let command = ProvisioningCommand::RevokeIdentity {
        operation_id: "retire-after-owned-io".into(),
        identity_id: human.identity_id.clone(),
        expected_revision: 0,
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!(
        "http://{}/v2/identities/{}/revoke",
        server.http_address.unwrap(),
        human.identity_id
    );
    let response = http
        .post(&url)
        .bearer_auth(&admin.credential)
        .json(&command)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let response: serde_json::Value = response.json().await.unwrap();
    sample("provider-dispatch-busy.json", &command, 503, &response);
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    worker.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), closure)
        .await
        .unwrap()
        .unwrap();
    let Outcome::ProviderStatus { deliveries, .. } = server
        .store
        .execute(
            &human.credential_id,
            &Operation::ProviderStatus {
                conversation_id: conv.id.clone(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].state, "uncertain");
    assert!(deliveries[0].provider_id.is_none());
    let response = http
        .post(&url)
        .bearer_auth(&admin.credential)
        .json(&command)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: serde_json::Value = response.json().await.unwrap();
    sample("provider-dispatch-retry.json", &command, 200, &response);
    let restarted = Worker::with_drivers(server.service.clone(), vec![driver])
        .await
        .unwrap();
    restarted.shutdown().await.unwrap();
    backend.await.unwrap();
    server.shutdown().await.unwrap();
}

struct StatusAdapter {
    http: connect_connector_http::Client,
    effects: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl connect_connector_api::Connector for StatusAdapter {
    fn name(&self) -> &'static str {
        "owned-status-fixture"
    }
    fn descriptor(&self) -> connect_connector_api::Descriptor {
        connect_connector_api::Descriptor::one_way(self.name())
    }
    fn manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut m = connect_connector_api::manifest::Manifest::messaging(self.descriptor());
        m.operations
            .insert(connect_connector_api::manifest::ConnectorOperation::MessageStatus);
        m
    }
    fn message_extension(&self) -> Option<&dyn connect_connector_api::lifecycle::MessageExtension> {
        Some(self)
    }
    async fn deliver(
        &self,
        _: &connect_connector_api::DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<connect_connector_api::Completion, connect_connector_api::Failure> {
        permit.admit_effect()?;
        Err(connect_connector_api::Failure::Rejected)
    }
}
#[async_trait::async_trait]
impl connect_connector_api::lifecycle::MessageExtension for StatusAdapter {
    async fn send(
        &self,
        _: &connect_connector_api::lifecycle::MessageRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<connect_connector_api::lifecycle::MessageReceipt, connect_connector_api::Failure>
    {
        permit.admit_effect()?;
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(connect_connector_api::Failure::Uncertain)
    }
    async fn message_status(
        &self,
        id: &str,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<connect_connector_api::lifecycle::MessageReceipt, connect_connector_api::Failure>
    {
        let value = self
            .http
            .request(
                connect_connector_http::HttpMethod::GET,
                &["status", id],
                None,
                permit,
            )
            .await?
            .success()?;
        serde_json::from_value(value).map_err(|_| connect_connector_api::Failure::Uncertain)
    }
}

#[tokio::test]
async fn actual_status_get_cancellation_releases_authority_and_read_only_retry_never_creates_again()
{
    use connect_connector_api::lifecycle::*;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (sent, received) = oneshot::channel();
    let (closed, closure) = oneshot::channel();
    let (checked, check_done) = oneshot::channel();
    let backend = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let bytes = request(&mut first).await;
        let line = String::from_utf8(bytes).unwrap();
        let path = line.split_whitespace().nth(1).unwrap();
        assert!(path.starts_with("/status/"));
        let id = path.trim_start_matches("/status/").to_owned();
        sent.send(id.clone()).unwrap();
        let mut tail = vec![];
        tokio::time::timeout(Duration::from_secs(6), first.read_to_end(&mut tail))
            .await
            .unwrap()
            .unwrap();
        assert!(tail.is_empty());
        closed.send(()).unwrap();
        let (mut second, _) = tokio::time::timeout(Duration::from_secs(4), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let bytes = request(&mut second).await;
        assert!(bytes.starts_with(format!("GET /status/{id} ").as_bytes()));
        let receipt = MessageReceipt {
            observation: Observation {
                mutation_id: id,
                external_id: Some("observed-status-reference".into()),
                event_id: "status-one".into(),
                state: State::Message(MessageState::Accepted),
                verification: Verification::Reconciliation,
            },
            reply: None,
            reply_sources: vec![],
        };
        let body = serde_json::to_vec(&receipt).unwrap();
        second.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
        second.write_all(&body).await.unwrap();
        drop(second);
        tokio::select! {
            _ = listener.accept() => panic!("Unauthorized status lookup opened another connection"),
            result = check_done => result.unwrap(),
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let human = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    let bot = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let admin = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let Outcome::Conversation(conv) = server
        .store
        .execute(
            &human.credential_id,
            &Operation::Create {
                request_id: "status-conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![bot
                    .identity_id
                    .clone()]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let driver = Arc::new(StatusAdapter {
        http: connect_connector_http::Client::new(&base, "fixture-secret").unwrap(),
        effects: Default::default(),
    });
    use connect_connector_api::Connector;
    server
        .store
        .register_connector(driver.descriptor())
        .unwrap();
    server
        .store
        .configure_route(
            &admin.credential_id,
            &Route {
                id: "status-route".into(),
                identity_id: bot.identity_id,
                credential_id: bot.credential_id,
                conversation_id: conv.id.clone(),
                connector: driver.name().into(),
                target: "fixture-peer".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![driver.clone()])
        .await
        .unwrap();
    server
        .store
        .execute(
            &human.credential_id,
            &Operation::Send {
                request_id: "one-status-effect".into(),
                conversation_id: conv.id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("one possible effect"),
            },
        )
        .unwrap();
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Outcome::ProviderStatus { deliveries } = server
                .store
                .execute(
                    &human.credential_id,
                    &Operation::ProviderStatus {
                        conversation_id: conv.id.clone(),
                    },
                )
                .unwrap()
            else {
                panic!()
            };
            if let Some(job) = deliveries.iter().find(|d| d.state == "uncertain") {
                break job.job_id.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let service = server.service.clone();
    let pending_driver = driver.clone();
    let pending_id = id.clone();
    let lookup = tokio::spawn(async move {
        Worker::reconcile(&service, pending_driver.as_ref(), &pending_id).await
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), received)
            .await
            .unwrap()
            .unwrap(),
        id
    );
    let before = Store::inspect_snapshot(server.store.path()).unwrap();
    let command = ProvisioningCommand::RevokeIdentity {
        operation_id: "revoke-after-status-io".into(),
        identity_id: human.identity_id.clone(),
        expected_revision: 0,
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!(
        "http://{}/v2/identities/{}/revoke",
        server.http_address.unwrap(),
        human.identity_id
    );
    let response = http
        .post(&url)
        .bearer_auth(&admin.credential)
        .json(&command)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    let response: serde_json::Value = response.json().await.unwrap();
    sample("provider-status-busy.json", &command, 503, &response);
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    lookup.abort();
    assert!(lookup.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(1), closure)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    assert_eq!(
        Worker::reconcile(&server.service, driver.as_ref(), &id)
            .await
            .unwrap(),
        Apply::Applied
    );
    assert_eq!(driver.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    let response = http
        .post(&url)
        .bearer_auth(&admin.credential)
        .json(&command)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: serde_json::Value = response.json().await.unwrap();
    sample("provider-status-retry.json", &command, 200, &response);
    let after = Store::inspect_snapshot(server.store.path()).unwrap();
    assert!(Worker::reconcile(&server.service, driver.as_ref(), &id)
        .await
        .is_err());
    assert_eq!(Store::inspect_snapshot(server.store.path()).unwrap(), after);
    assert_eq!(driver.effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    checked.send(()).unwrap();
    backend.await.unwrap();
    worker.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}
