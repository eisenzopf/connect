#![recursion_limit = "256"]
//! Current public SDK/provisioning plus protected stock connector composition.
//! No application Store access or altered service dispatch in this journey.
#![allow(clippy::unwrap_used)]
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig, ApplicationOperation},
    canonical::CanonicalHttpClient,
    ClientConfig, MessageDraft,
};
use connect_connector_api::conformance::{Fixture, Injection};
use connect_connector_template::LocalProvider;
use connect_protocol::{
    credential_issuance::CredentialIssueRequest, messages::MessageAudience, provisioning::*,
    Operation, Outcome,
};
use connect_server::{
    canonical::{CanonicalConnectors, CanonicalServerConfig, RunningCanonicalServer},
    canonical_messages::{
        CanonicalMessagesConfig, MessageAccount, MessageRoute, ProviderMessageEndpoint,
    },
    cli::init,
    config::ServerConfig,
    providers::Worker,
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};

fn http(server: &RunningCanonicalServer, token: &str) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint: server.http_address.unwrap(),
        credential: token.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        timeout_ms: 10000,
    }
}
struct Setup {
    temp: tempfile::TempDir,
    config: CanonicalServerConfig,
    operator: ClientConfig,
    peer_token: String,
    cid: String,
    root_pid: String,
    ai_pid: String,
    route: MessageRoute,
    remote: PathBuf,
    endpoint_id: String,
}
async fn setup() -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let old = ServerConfig::load(&state.join("server.toml")).unwrap();
    let operator = ClientConfig::load(&state.join("operator.toml")).unwrap();
    let peer = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let signing = temp.path().join("server-signing.der");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    std::fs::write(&signing, pair.serialize_der()).unwrap();
    let mut config = CanonicalServerConfig {
        webrtc: None,
        network_profile: Default::default(),
        webtransport: None,
        websocket: None,
        bind: old.bind,
        database: old.database,
        certificate: old.certificate,
        private_key: old.private_key,
        signing_key_id: "connector-stock".into(),
        signing_algorithm: "EdDSA".into(),
        signing_private_key: signing,
        signing_key_not_before: chrono::Utc::now() - chrono::Duration::seconds(1),
        signing_key_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        http_bind: Some("127.0.0.1:0".parse().unwrap()),
        mcp: None,
        record_https: None,
        record_retention: Default::default(),
        user_assurance: vec![],
        telephone: None,
        public_vapi: None,
        provider_voice: None,
        provider_messages: None,
        limits: old.limits,
        session_reconnect_grace_seconds: 30,
    };
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    let mut admin = ApplicationClient::open(http(&server, &operator.credential))
        .await
        .unwrap();
    let ProvisioningOutcome::Identity {
        identity_id: ai, ..
    } = admin
        .provision(ProvisioningCommand::Identity {
            operation_id: "connector-ai".into(),
            alias: "connector-ai".into(),
            kind: IdentityKind::Ai,
            display_name: None,
        })
        .await
        .unwrap()
    else {
        panic!("identity")
    };
    let mut client = CanonicalHttpClient::open(http(&server, &operator.credential))
        .await
        .unwrap();
    let mut create = connect_client::conversations::CreateRequest::persistent(vec![
        ai.clone(),
        peer.identity_id.clone(),
    ]);
    create.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("connector-room", create)
        .await
        .unwrap();
    let root_pid = room
        .participants
        .iter()
        .find(|p| p.identity_id == operator.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let ai_pid = room
        .participants
        .iter()
        .find(|p| p.identity_id == ai)
        .unwrap()
        .participant_id
        .clone();
    let peer_pid = room
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let recipients = vec![
        operator.identity_id.clone(),
        ai.clone(),
        peer.identity_id.clone(),
    ];
    let delegate = admin
        .issue_scoped_credential(CredentialIssueRequest {
            operation_id: "connector-delegate".into(),
            identity_id: ai.clone(),
            operations: vec!["send".into(), "history".into(), "context_view".into()],
            recipients: recipients.clone(),
            expires_at: chrono::Utc::now().timestamp() + 600,
        })
        .await
        .unwrap();
    let peer_key = admin
        .issue_scoped_credential(CredentialIssueRequest {
            operation_id: "connector-reader".into(),
            identity_id: peer.identity_id,
            operations: vec![
                "history".into(),
                "context_view".into(),
                "conversation.get".into(),
                "message.history".into(),
            ],
            recipients,
            expires_at: chrono::Utc::now().timestamp() + 600,
        })
        .await
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = admin
        .provision(ProvisioningCommand::Endpoint {
            operation_id: "connector-endpoint".into(),
            identity_id: ai.clone(),
            device: DeviceSpec {
                id: "connector-device".into(),
                kind: "service".into(),
                platform: "fixture".into(),
            },
            transport: EndpointTransport::Connector,
            address: "connector://local-contributor/fixture-peer".into(),
            capabilities: vec!["messaging".into()],
            secret_ref: Some("isolated-fixture-key".into()),
        })
        .await
        .unwrap()
    else {
        panic!("endpoint")
    };
    admin
        .provision(ProvisioningCommand::VerifyEndpoint {
            operation_id: "connector-verified".into(),
            endpoint_id: endpoint_id.clone(),
            expected_revision: 0,
            evidence_ref: "isolated-fixture-backend".into(),
        })
        .await
        .unwrap();
    client
        .send_message(
            "public-primer",
            &room.id,
            "public-primer-message",
            MessageDraft::text_all("public-context-primer"),
        )
        .await
        .unwrap();
    client
        .send_message(
            "excluded-secret",
            &room.id,
            "excluded-secret-message",
            MessageDraft::text(
                MessageAudience::Participants(vec![peer_pid]),
                "exclude-from-connector-secret",
            ),
        )
        .await
        .unwrap();
    config.provider_messages = Some(CanonicalMessagesConfig {
        operator_credential_id: operator.credential_id.clone(),
        accounts: vec![MessageAccount {
            connector: "local-contributor".into(),
            secret_reference: "isolated-fixture-key".into(),
        }],
        endpoints: vec![ProviderMessageEndpoint {
            route_id: "contributed-route".into(),
            connector: "local-contributor".into(),
            transport: EndpointTransport::Connector,
            endpoint_id: endpoint_id.clone(),
            source_endpoint_id: None,
            account_credential_id: operator.credential_id.clone(),
            secret_reference: "isolated-fixture-key".into(),
        }],
        telnyx_callbacks: None,
    });
    let route = MessageRoute {
        id: "contributed-route".into(),
        identity_id: ai,
        credential_id: delegate.credential_id,
        conversation_id: room.id.clone(),
        connector: "local-contributor".into(),
        target: "fixture-peer".into(),
        source: None,
        enabled: true,
        incoming_to: MessageAudience::Participants(vec![root_pid.clone()]),
    };
    drop(client);
    drop(admin);
    server.shutdown().await.unwrap();
    let remote = temp.path().join("remote-provider.json");
    Setup {
        temp,
        config,
        operator,
        peer_token: peer_key.credential,
        cid: room.id,
        root_pid,
        ai_pid,
        route,
        remote,
        endpoint_id,
    }
}
impl Setup {
    async fn start(&self, driver: Arc<LocalProvider>) -> RunningCanonicalServer {
        RunningCanonicalServer::start_with_connectors(
            self.config.clone(),
            CanonicalConnectors {
                messages: Some(vec![driver]),
                message_routes: vec![self.route.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }
}
async fn delivered(app: &mut ApplicationClient, cid: &str, state: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = app
                .execute(
                    ApplicationOperation::try_from(Operation::ContextDeliveries {
                        conversation_id: cid.into(),
                        before: None,
                        limit: 100,
                    })
                    .unwrap(),
                )
                .await
                .unwrap();
            let Outcome::ContextDeliveries(page) = result else {
                panic!("deliveries")
            };
            if let Some(row) = page
                .deliveries
                .iter()
                .find(|v| v.provider == "local-contributor" && v.state == state)
            {
                return row.id.strip_prefix("provider-job:").unwrap().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn reply(client: &mut CanonicalHttpClient, cid: &str, needle: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = client
                .history(cid, serde_json::from_value(json!({"limit":100})).unwrap())
                .await
                .unwrap();
            if let Some(row) = page.messages.iter().find(|v| v.body.contains(needle)) {
                return row.body.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contributed_stock_host_private_context_duplicate_reconnect_and_ingress_use_public_workflows(
) {
    let f = setup().await;
    let driver = Arc::new(LocalProvider::open_fixture(f.remote.clone()).unwrap());
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let draft = MessageDraft::text(
        MessageAudience::Participants(vec![f.ai_pid.clone()]),
        "violet-current-question",
    );
    let receipt = client
        .send_message(
            "original-question",
            &f.cid,
            "original-question-message",
            draft.clone(),
        )
        .await
        .unwrap();
    let job = delivered(&mut app, &f.cid, "delivered").await;
    assert_eq!(driver.effects(&job), 1);
    let answer = reply(
        &mut client,
        &f.cid,
        "Current request: violet-current-question",
    )
    .await;
    assert!(answer.contains("public-context-primer"));
    assert!(!answer.contains("exclude-from-connector-secret"));
    let mut peer = CanonicalHttpClient::open(http(&server, &f.peer_token))
        .await
        .unwrap();
    let visible = peer
        .history(
            &f.cid,
            serde_json::from_value(json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert!(visible
        .messages
        .iter()
        .any(|m| m.body == "public-context-primer"));
    assert!(visible
        .messages
        .iter()
        .all(|m| !m.body.contains("violet-current-question")));
    assert_eq!(
        client
            .send_message(
                "original-question",
                &f.cid,
                "original-question-message",
                draft.clone()
            )
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(driver.effects(&job), 1);
    drop(peer);
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
    drop(driver);
    let driver = Arc::new(LocalProvider::open_fixture(f.remote.clone()).unwrap());
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    assert_eq!(
        client
            .send_message(
                "original-question",
                &f.cid,
                "original-question-message",
                draft
            )
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(driver.effects(&job), 1);
    let contact = driver
        .independent_message("independent-contact", "independent amber proposal")
        .unwrap();
    let incoming = Worker::receive(
        server.service.as_ref(),
        driver.as_ref(),
        "contributed-route",
        &contact,
    )
    .unwrap();
    assert_eq!(
        Worker::receive(
            server.service.as_ref(),
            driver.as_ref(),
            "contributed-route",
            &contact
        )
        .unwrap(),
        incoming
    );
    let page = client
        .history(
            &f.cid,
            serde_json::from_value(json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .filter(|v| v.body == "independent amber proposal")
            .count(),
        1
    );
    let mut peer = CanonicalHttpClient::open(http(&server, &f.peer_token))
        .await
        .unwrap();
    let visible = peer
        .history(
            &f.cid,
            serde_json::from_value(json!({"limit":100})).unwrap(),
        )
        .await
        .unwrap();
    assert!(!visible
        .messages
        .iter()
        .any(|v| v.body.contains("amber proposal")));
    drop(peer);
    drop(client);
    server.shutdown().await.unwrap();
    let _ = (&f.temp, &f.root_pid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contributed_stock_host_uncertainty_reconciles_original_and_endpoint_withdrawal_refuses_ingress(
) {
    let f = setup().await;
    let driver = Arc::new(LocalProvider::open_fixture(f.remote.clone()).unwrap());
    driver.inject(Injection::UncertainMessage);
    let server = f.start(driver.clone()).await;
    let mut client = CanonicalHttpClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let mut app = ApplicationClient::open(http(&server, &f.operator.credential))
        .await
        .unwrap();
    let draft = MessageDraft::text(
        MessageAudience::Participants(vec![f.ai_pid.clone()]),
        "uncertain-original-input",
    );
    let accepted = client
        .send_message(
            "uncertain-original",
            &f.cid,
            "uncertain-original-message",
            draft.clone(),
        )
        .await
        .unwrap();
    let job = delivered(&mut app, &f.cid, "uncertain").await;
    assert_eq!(driver.effects(&job), 1);
    assert_eq!(
        client
            .send_message(
                "uncertain-original",
                &f.cid,
                "uncertain-original-message",
                draft
            )
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(driver.effects(&job), 1);
    Worker::reconcile(server.service.as_ref(), driver.as_ref(), &job)
        .await
        .unwrap();
    assert_eq!(delivered(&mut app, &f.cid, "delivered").await, job);
    assert_eq!(driver.effects(&job), 1);
    let contact = driver
        .independent_message(
            "withdrawn-contact",
            "must not enter after endpoint withdrawal",
        )
        .unwrap();
    app.provision(ProvisioningCommand::RevokeEndpoint {
        operation_id: "withdraw-route-endpoint".into(),
        endpoint_id: f.endpoint_id.clone(),
        expected_revision: 1,
    })
    .await
    .unwrap();
    assert!(Worker::receive(
        server.service.as_ref(),
        driver.as_ref(),
        "contributed-route",
        &contact
    )
    .is_err());
    assert!(
        Worker::reconcile(server.service.as_ref(), driver.as_ref(), &job)
            .await
            .is_err()
    );
    assert_eq!(driver.effects(&job), 1);
    drop(client);
    drop(app);
    server.shutdown().await.unwrap();
}
