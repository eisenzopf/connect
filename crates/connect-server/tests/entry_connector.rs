//! An independent context adapter and real storage failure through public SDKs.
#![allow(clippy::unwrap_used)]
use async_trait::async_trait;
use connect_client::{ClientConfig, ConnectClient};
use connect_connector_api::{Completion, Connector, DeliveryRequest, Descriptor, Failure};
use connect_protocol::{Operation, ReadinessState};
use connect_server::{cli::init, config::ServerConfig, providers::Worker, runtime::RunningServer};
use connect_store::Route;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct ResearchAssistant {
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl Connector for ResearchAssistant {
    fn name(&self) -> &'static str {
        "research-assistant"
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            conversational: true,
            ..Descriptor::one_way(self.name())
        }
    }
    async fn deliver(
        &self,
        request: &DeliveryRequest,
        permit: connect_connector_api::DispatchPermit,
    ) -> Result<Completion, Failure> {
        permit.admit_effect()?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(request
            .context
            .as_deref()
            .unwrap()
            .contains("reference fact"));
        Ok(Completion {
            provider_id: "fixture-completion".into(),
            reply: Some("I retained the reference fact".into()),
        })
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    RunningServer,
    ConnectClient,
    String,
    Arc<ResearchAssistant>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let assistant = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let mut client = ConnectClient::open(config).await.unwrap();
    let conversation = client
        .create("conversation", vec![assistant.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let adapter = Arc::new(ResearchAssistant {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    server
        .store
        .register_connector(adapter.descriptor())
        .unwrap();
    server
        .store
        .configure_route(
            &operator.credential_id,
            &Route {
                id: "research".into(),
                identity_id: assistant.identity_id,
                credential_id: assistant.credential_id,
                conversation_id: conversation.clone(),
                connector: adapter.name().into(),
                target: "local-fixture".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    (tmp, server, client, conversation, adapter)
}

#[tokio::test]
async fn independent_context_adapter_uses_composition_only_and_survives_restart() {
    let (tmp, server, mut client, cid, adapter) = fixture().await;
    let worker = Worker::with_drivers(server.service.clone(), vec![adapter.clone()])
        .await
        .unwrap();
    let command = Operation::Send {
        request_id: "one-logical-send".into(),
        conversation_id: cid.clone(),
        message: connect_protocol::messages::MessageDraft::text_all("reference fact: violet"),
    };
    let receipt = client.request(command.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let deliveries = client.context_deliveries(&cid, None, 10).await.unwrap();
            if deliveries
                .deliveries
                .iter()
                .any(|d| d.provider == "research-assistant" && d.state == "delivered")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(client.request(command.clone()).await.unwrap(), receipt);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
    let state = tmp.path().join("state");
    let server = RunningServer::start(ServerConfig::load(&state.join("server.toml")).unwrap())
        .await
        .unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![adapter.clone()])
        .await
        .unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(client.request(command).await.unwrap(), receipt);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_result_commit_updates_health_and_preserves_uncertain_work_without_resend() {
    let (tmp, server, mut client, cid, adapter) = fixture().await;
    let db = rusqlite::Connection::open(tmp.path().join("state/state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_result BEFORE UPDATE OF provider_id ON provider_jobs BEGIN SELECT RAISE(FAIL, 'fixture commit failure'); END;").unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![adapter.clone()])
        .await
        .unwrap();
    let command = Operation::Send {
        request_id: "uncertain-send".into(),
        conversation_id: cid,
        message: connect_protocol::messages::MessageDraft::text_all("reference fact: violet"),
    };
    let receipt = client.request(command.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server.service.readiness.failed())
        .await
        .unwrap();
    let health = client.health().await.unwrap().readiness.unwrap();
    assert_eq!(health.state, ReadinessState::Unavailable);
    assert!(health
        .components
        .iter()
        .any(|c| c.component == "provider-delivery" && c.reason == "worker-exited"));
    assert_eq!(client.request(command.clone()).await.unwrap(), receipt);
    assert!(worker.shutdown().await.is_err());
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    db.execute_batch("DROP TRIGGER fail_result").unwrap();
    assert_eq!(server.store.recover_provider_jobs().unwrap(), 1);
    let state: String = db
        .query_row("SELECT state FROM provider_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "uncertain");
    let worker = Worker::with_drivers(server.service.clone(), vec![adapter.clone()])
        .await
        .unwrap();
    assert_eq!(client.request(command).await.unwrap(), receipt);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}
