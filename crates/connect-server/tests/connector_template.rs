//! Gate 5 contribution kit composed through public APIs on isolated loopback state.
#![allow(clippy::unwrap_used)]
use connect_client::{ClientConfig, ConnectClient};
use connect_connector_api::{
    conformance::{Fixture, Injection},
    Connector,
};
use connect_connector_template::LocalProvider;
use connect_protocol::Operation;
use connect_server::{cli::init, config::ServerConfig, providers::Worker, runtime::RunningServer};
use connect_store::Route;
use std::{sync::Arc, time::Duration};

async fn fixture() -> (
    tempfile::TempDir,
    RunningServer,
    ConnectClient,
    String,
    Arc<LocalProvider>,
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
    let agent = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let mut client = ConnectClient::open(config).await.unwrap();
    let conversation = client
        .create("conversation", vec![agent.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let provider = Arc::new(LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap());
    server
        .store
        .register_connector(provider.descriptor())
        .unwrap();
    server
        .store
        .configure_route(
            &operator.credential_id,
            &Route {
                id: "contribution".into(),
                identity_id: agent.identity_id,
                credential_id: agent.credential_id,
                conversation_id: conversation.clone(),
                connector: provider.name().into(),
                target: "fixture-peer".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    (tmp, server, client, conversation, provider)
}
async fn delivery(client: &mut ConnectClient, conversation: &str, expected: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = client
                .context_deliveries(conversation, None, 10)
                .await
                .unwrap();
            if let Some(item) = page
                .deliveries
                .iter()
                .find(|d| d.provider == "local-contributor" && d.state == expected)
            {
                return item.id.strip_prefix("provider-job:").unwrap().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn template_context_and_independent_contact_survive_host_and_adapter_restart() {
    let (tmp, server, mut client, cid, provider) = fixture().await;
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let send = Operation::Send {
        request_id: "contributor-first".into(),
        conversation_id: cid.clone(),
        body: "The approved color is violet.".into(),
    };
    let receipt = client.request(send.clone()).await.unwrap();
    let id = delivery(&mut client, &cid, "delivered").await;
    assert_eq!(provider.effects(&id), 1);
    assert!(client
        .history(&cid, None, 10)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|m| m.body.contains("Historical sources:") && m.body.contains("violet")));
    assert_eq!(client.request(send.clone()).await.unwrap(), receipt);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
    let state = tmp.path().join("state");
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let provider = Arc::new(LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(client.request(send).await.unwrap(), receipt);
    assert_eq!(provider.effects(&id), 1);
    // The remote peer independently initiates a new message on the same route.
    // Ingress authenticates the adapter response; the host maps the existing
    // route credential and journals it without invoking outbound send.
    let contact = provider
        .independent_message("remote-contact", "Independently proposed color is amber.")
        .unwrap();
    let incoming_id = Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact,
    )
    .unwrap();
    assert_eq!(
        Worker::receive(
            server.service.as_ref(),
            provider.as_ref(),
            "contribution",
            &contact
        )
        .unwrap(),
        incoming_id
    );
    assert_eq!(provider.effects("remote-contact"), 1);
    let history = client.history(&cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.id == incoming_id && m.body.contains("amber"))
            .count(),
        1
    );
    // A fresh local request exports that inbound source as bounded history.
    client
        .send(
            "contact-follow-up",
            &cid,
            "What did the independent peer propose?",
        )
        .await
        .unwrap();
    let follow_up = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let history = client.history(&cid, None, 100).await.unwrap();
            if history
                .messages
                .iter()
                .any(|m| m.body.contains("amber") && m.body.contains("Current request:"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(follow_up.is_ok());
    let deliveries = client.context_deliveries(&cid, None, 100).await.unwrap();
    let next = deliveries
        .deliveries
        .iter()
        .find_map(|delivery| {
            let job = delivery.id.strip_prefix("provider-job:")?;
            (job != id && delivery.state == "delivered").then_some(job)
        })
        .expect("new context reply committed");
    let attributed = server
        .store
        .connector_receipt(next, provider.name())
        .unwrap();
    assert!(attributed
        .reply_sources
        .iter()
        .any(|source| source.ends_with(&format!(":{incoming_id}"))));
    let mut config = ClientConfig::load(&state.join("agent.toml")).unwrap();
    config.endpoint = server.address;
    let mut app = ConnectClient::open(config).await.unwrap();
    let incoming = app.inbox(None, 100, 0).await.unwrap();
    assert!(incoming.events.iter().any(|event| event
        .message
        .as_ref()
        .is_some_and(|m| m.body.contains("amber"))));
    worker.shutdown().await.unwrap();
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn unknown_effect_is_not_repeated_after_worker_and_adapter_restart() {
    let (tmp, server, mut client, cid, provider) = fixture().await;
    provider.inject(Injection::UncertainMessage);
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let send = Operation::Send {
        request_id: "contributor-unknown".into(),
        conversation_id: cid.clone(),
        body: "Uncertain provider response".into(),
    };
    let receipt = client.request(send.clone()).await.unwrap();
    let id = delivery(&mut client, &cid, "uncertain").await;
    assert_eq!(provider.effects(&id), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
    let state = tmp.path().join("state");
    let server = RunningServer::start_local(
        ServerConfig::load(&state.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let provider = Arc::new(LocalProvider::open_fixture(tmp.path().join("provider.json")).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    let mut config = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    assert_eq!(client.request(send).await.unwrap(), receipt);
    assert_eq!(provider.effects(&id), 1);
    assert_eq!(delivery(&mut client, &cid, "uncertain").await, id);
    // An authoritative read under the original durable job ID resolves acceptance
    // and commits the saved reply/provenance without another provider send.
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &id)
            .await
            .unwrap(),
        connect_connector_api::lifecycle::Apply::Applied
    );
    assert_eq!(delivery(&mut client, &cid, "delivered").await, id);
    assert_eq!(provider.effects(&id), 1);
    let receipt = server
        .store
        .connector_receipt(&id, provider.name())
        .unwrap();
    assert_eq!(
        receipt.observation.verification,
        connect_connector_api::lifecycle::Verification::Reconciliation
    );
    assert!(!receipt.reply_sources.is_empty());
    assert_eq!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &id)
            .await
            .unwrap(),
        connect_connector_api::lifecycle::Apply::Duplicate
    );
    assert_eq!(provider.effects(&id), 1);
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn duplicate_composition_is_rejected_before_worker_claims() {
    let (_tmp, server, client, _cid, provider) = fixture().await;
    assert!(
        Worker::with_drivers(server.service.clone(), vec![provider.clone(), provider]).is_err()
    );
    drop(client);
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn invalid_or_unready_composition_cannot_persist_contracts_or_claim_work() {
    use async_trait::async_trait;
    use connect_connector_api::{
        manifest::{Manifest, Readiness},
        Completion, DeliveryRequest, Failure,
    };
    struct MissingSecret;
    #[async_trait]
    impl Connector for MissingSecret {
        fn name(&self) -> &'static str {
            "missing-secret"
        }
        fn readiness(&self) -> Readiness {
            Readiness::MissingSecret
        }
        async fn deliver(&self, _: &DeliveryRequest) -> Result<Completion, Failure> {
            panic!("must not dispatch");
        }
    }
    struct Unsupported;
    #[async_trait]
    impl Connector for Unsupported {
        fn name(&self) -> &'static str {
            "unsupported"
        }
        fn manifest(&self) -> Manifest {
            let mut value = Manifest::messaging(self.descriptor());
            value.video = true;
            value
        }
        async fn deliver(&self, _: &DeliveryRequest) -> Result<Completion, Failure> {
            panic!("must not dispatch");
        }
    }
    let (tmp, server, mut client, cid, provider) = fixture().await;
    client
        .send("before-claim", &cid, "Pending work")
        .await
        .unwrap();
    assert!(Worker::with_drivers(
        server.service.clone(),
        vec![provider.clone(), Arc::new(MissingSecret)]
    )
    .is_err());
    assert!(Worker::with_drivers(
        server.service.clone(),
        vec![provider, Arc::new(Unsupported)]
    )
    .is_err());
    let db = rusqlite::Connection::open(tmp.path().join("state/state.sqlite3")).unwrap();
    let manifests: u32 = db
        .query_row("SELECT COUNT(*) FROM connector_manifests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(manifests, 0);
    let claimed: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM provider_jobs WHERE state!='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claimed, 0);
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn normalized_host_receipts_preserve_sources_terminal_order_and_current_authority() {
    use connect_connector_api::lifecycle::*;
    let (tmp, server, mut client, cid, provider) = fixture().await;
    let worker = Worker::with_drivers(server.service.clone(), vec![provider.clone()]).unwrap();
    client
        .send("source-check", &cid, "Source color is violet.")
        .await
        .unwrap();
    let id = delivery(&mut client, &cid, "delivered").await;
    let receipt = server
        .store
        .connector_receipt(&id, provider.name())
        .unwrap();
    assert!(!receipt.reply_sources.is_empty());
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &receipt)
            .unwrap(),
        Apply::Duplicate
    );
    let before = client
        .history(&cid, None, 100)
        .await
        .unwrap()
        .messages
        .len();
    let mut forged = receipt.clone();
    forged.observation.event_id = "forged-source".into();
    forged.reply_sources = vec!["foreign-conversation-source".into()];
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &forged)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        client
            .history(&cid, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        before
    );
    let mut older = receipt.clone();
    older.reply = None;
    older.reply_sources.clear();
    older.observation.event_id = "late-acceptance".into();
    older.observation.state = State::Message(MessageState::Accepted);
    older.observation.verification = Verification::VerifiedCallback;
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &older)
            .unwrap(),
        Apply::Older
    );
    assert_eq!(
        server
            .store
            .connector_receipt(&id, provider.name())
            .unwrap(),
        receipt
    );
    let mut contradictory = older;
    contradictory.observation.event_id = "conflicting-terminal".into();
    contradictory.observation.state = State::Message(MessageState::Failed);
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &contradictory)
            .unwrap_err()
            .code,
        "conflict"
    );
    let contact = provider
        .independent_message("contact-auth", "An independent incoming contact.")
        .unwrap();
    Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact,
    )
    .unwrap();
    let operator = ClientConfig::load(&tmp.path().join("state/operator.toml")).unwrap();
    let authority = server.store.authenticate(&operator.credential).unwrap();
    let agent = ClientConfig::load(&tmp.path().join("state/agent.toml")).unwrap();
    server
        .store
        .revoke(&authority, &agent.credential_id)
        .unwrap();
    // Revocation is checked before replay of both retained receipt directions.
    assert!(server
        .store
        .apply_connector_receipt(provider.name(), &receipt)
        .is_err());
    assert!(Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact
    )
    .is_err());
    assert!(
        Worker::reconcile(server.service.as_ref(), provider.as_ref(), &id)
            .await
            .is_err()
    );
    worker.shutdown().await.unwrap();
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn final_callback_before_attempt_response_retains_answer_once() {
    use connect_connector_api::{lifecycle::*, DeliveryRequest};
    let (_tmp, server, mut client, cid, provider) = fixture().await;
    server
        .store
        .register_connector_manifest(
            provider.descriptor(),
            &Connector::manifest(provider.as_ref()),
        )
        .unwrap();
    client
        .send(
            "callback-before-response",
            &cid,
            "Approved color is violet.",
        )
        .await
        .unwrap();
    let job = server.store.claim_provider_job().unwrap().unwrap();
    let request = DeliveryRequest {
        target: job.route.target.clone(),
        source: job.route.source.clone(),
        body: job.body.clone(),
        previous_chat_id: job.previous_chat_id.clone(),
        context: job.context.clone(),
    };
    let mut response = provider.deliver_observed(&job.id, &request).await.unwrap();
    let mut final_receipt = response.clone();
    final_receipt.reply = None;
    final_receipt.reply_sources.clear();
    final_receipt.observation.event_id = "final-first".into();
    final_receipt.observation.verification = Verification::VerifiedCallback;
    let (body, signature) = provider.signed_callback(&final_receipt.observation);
    final_receipt.observation = provider.authenticate_callback(&body, &signature).unwrap();
    assert_eq!(
        Worker::observe(server.service.as_ref(), provider.as_ref(), &final_receipt).unwrap(),
        Apply::Applied
    );
    response.observation.state = State::Message(MessageState::Accepted);
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &response)
            .unwrap(),
        Apply::Older
    );
    assert_eq!(
        server
            .store
            .apply_connector_receipt(provider.name(), &response)
            .unwrap(),
        Apply::Duplicate
    );
    let history = client.history(&cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body.contains("Historical sources:"))
            .count(),
        1
    );
    assert_eq!(provider.effects(&job.id), 1);
    assert!(!server
        .store
        .connector_receipt(&job.id, provider.name())
        .unwrap()
        .reply_sources
        .is_empty());
    assert_eq!(delivery(&mut client, &cid, "delivered").await, job.id);
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn independent_incoming_replay_checks_current_expiry_and_external_binding() {
    let (tmp, server, client, _cid, provider) = fixture().await;
    let contact = provider
        .independent_message("expiry-contact", "Independent incoming message.")
        .unwrap();
    let id = Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact,
    )
    .unwrap();
    let mut reused = contact.clone();
    reused.mutation_id = "another-logical-id".into();
    assert!(Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &reused
    )
    .is_err());
    let agent = ClientConfig::load(&tmp.path().join("state/agent.toml")).unwrap();
    let c = rusqlite::Connection::open(tmp.path().join("state/state.sqlite3")).unwrap();
    // A private clock-boundary fixture, with no sleep or altered host clock.
    c.execute(
        "UPDATE credentials SET expires_at=0 WHERE id=?1",
        [&agent.credential_id],
    )
    .unwrap();
    assert!(Worker::receive(
        server.service.as_ref(),
        provider.as_ref(),
        "contribution",
        &contact
    )
    .is_err());
    let messages: u32 = c
        .query_row(
            "SELECT COUNT(*) FROM connector_incoming WHERE message_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(messages, 1);
    drop(client);
    server.shutdown().await.unwrap();
}

struct ReceiveOnly;
#[async_trait::async_trait]
impl Connector for ReceiveOnly {
    fn name(&self) -> &'static str {
        "receive-only"
    }
    fn manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut manifest = connect_connector_api::manifest::Manifest::messaging(self.descriptor());
        manifest.operations =
            [connect_connector_api::manifest::ConnectorOperation::MessageReceive].into();
        manifest
    }
    async fn deliver(
        &self,
        _: &connect_connector_api::DeliveryRequest,
    ) -> Result<connect_connector_api::Completion, connect_connector_api::Failure> {
        panic!("receive-only connector must never receive an outbound provider call")
    }
}
#[tokio::test]
async fn receive_only_manifest_retains_ingress_but_fails_outbound_before_claim_or_provider_call() {
    use connect_connector_api::{
        lifecycle::{IncomingMessage, Verification},
        manifest::ConnectorOperation,
    };
    let (tmp, server, mut client, _, _) = fixture().await;
    let agent = ClientConfig::load(&tmp.path().join("state/agent.toml")).unwrap();
    let operator = ClientConfig::load(&tmp.path().join("state/operator.toml")).unwrap();
    let cid = client
        .create("receive-only-conversation", vec![agent.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let driver = Arc::new(ReceiveOnly);
    use connect_protocol::Outcome;
    let connection = rusqlite::Connection::open(server.store.path()).unwrap();
    server
        .store
        .register_connector(driver.descriptor())
        .unwrap();
    server
        .store
        .configure_route(
            &operator.credential_id,
            &Route {
                id: "receive-only-route".into(),
                identity_id: agent.identity_id,
                credential_id: agent.credential_id,
                conversation_id: cid.clone(),
                connector: driver.name().into(),
                target: "fixture-peer".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let worker = Worker::with_drivers(server.service.clone(), vec![driver.clone()]).unwrap();
    assert_eq!(
        server
            .store
            .require_connector_operation(driver.name(), ConnectorOperation::MessageSend)
            .unwrap_err()
            .code,
        "unsupported-connector-operation"
    );
    let incoming = IncomingMessage {
        mutation_id: "independent-incoming".into(),
        external_id: "external-contact".into(),
        body: "Independent incoming contact".into(),
        verification: Verification::VerifiedCallback,
    };
    let id = Worker::receive(
        server.service.as_ref(),
        driver.as_ref(),
        "receive-only-route",
        &incoming,
    )
    .unwrap();
    assert_eq!(
        Worker::receive(
            server.service.as_ref(),
            driver.as_ref(),
            "receive-only-route",
            &incoming
        )
        .unwrap(),
        id
    );
    let response = client
        .send(
            "native-response",
            &cid,
            "Ordinary communication remains visible",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Outcome::ProviderStatus { deliveries } = client
                .request(Operation::ProviderStatus {
                    conversation_id: cid.clone(),
                })
                .await
                .unwrap()
            else {
                panic!()
            };
            if deliveries.iter().any(|delivery| delivery.state == "failed") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .send(
                "native-response",
                &cid,
                "Ordinary communication remains visible"
            )
            .await
            .unwrap(),
        response
    );
    assert_eq!(connection.query_row::<u64,_,_>("SELECT COUNT(*) FROM provider_jobs WHERE state='sending' OR provider_id IS NOT NULL", [], |row| row.get(0)).unwrap(), 0);
    assert_eq!(
        connection
            .query_row::<u64, _, _>("SELECT COUNT(*) FROM context_deliveries", [], |row| row
                .get(0))
            .unwrap(),
        0
    );
    worker.shutdown().await.unwrap();
    let restarted = Worker::with_drivers(server.service.clone(), vec![driver.clone()]).unwrap();
    assert!(server.store.claim_provider_job().unwrap().is_none());
    restarted.shutdown().await.unwrap();
    client.close().await;
    server.shutdown().await.unwrap();
}
