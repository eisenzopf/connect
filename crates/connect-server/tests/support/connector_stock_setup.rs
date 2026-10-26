//! Current public SDK/provisioning plus protected stock connector composition.
//! No application Store access or altered service dispatch in this journey.
#![allow(clippy::unwrap_used)]
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig, ApplicationOperation},
    canonical::CanonicalHttpClient,
    ClientConfig, MessageDraft,
};
use connect_connector_api::Connector;
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
};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(super) fn http(server: &RunningCanonicalServer, token: &str) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint: server.http_address.unwrap(),
        credential: token.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        timeout_ms: 10000,
    }
}
pub(super) struct Setup {
    pub(super) temp: tempfile::TempDir,
    pub(super) config: CanonicalServerConfig,
    pub(super) operator: ClientConfig,
    pub(super) peer_token: String,
    pub(super) cid: String,
    pub(super) root_pid: String,
    pub(super) ai_pid: String,
    pub(super) peer_pid: String,
    pub(super) route: MessageRoute,
    pub(super) remote: PathBuf,
    pub(super) endpoint_id: String,
}
pub(super) async fn setup(connector: &str, account_reference: &str) -> Setup {
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
            address: format!("connector://{connector}/fixture-peer"),
            capabilities: vec!["messaging".into()],
            secret_ref: Some(account_reference.into()),
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
                MessageAudience::Participants(vec![peer_pid.clone()]),
                "exclude-from-connector-secret",
            ),
        )
        .await
        .unwrap();
    config.provider_messages = Some(CanonicalMessagesConfig {
        operator_credential_id: operator.credential_id.clone(),
        accounts: vec![MessageAccount {
            connector: connector.into(),
            secret_reference: account_reference.into(),
        }],
        endpoints: vec![ProviderMessageEndpoint {
            route_id: "contributed-route".into(),
            connector: connector.into(),
            transport: EndpointTransport::Connector,
            endpoint_id: endpoint_id.clone(),
            source_endpoint_id: None,
            account_credential_id: operator.credential_id.clone(),
            secret_reference: account_reference.into(),
        }],
        telnyx_callbacks: None,
    });
    let route = MessageRoute {
        id: "contributed-route".into(),
        identity_id: ai,
        credential_id: delegate.credential_id,
        conversation_id: room.id.clone(),
        connector: connector.into(),
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
        peer_pid,
        route,
        remote,
        endpoint_id,
    }
}
impl Setup {
    pub(super) async fn start(&self, driver: Arc<dyn Connector>) -> RunningCanonicalServer {
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
pub(super) async fn delivered(
    app: &mut ApplicationClient,
    cid: &str,
    connector: &str,
    state: &str,
) -> String {
    let mut observed = Vec::<String>::new();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
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
            observed = page
                .deliveries
                .iter()
                .map(|v| format!("{}:{}", v.provider, v.state))
                .collect();
            if let Some(row) = page
                .deliveries
                .iter()
                .find(|v| v.provider == connector && v.state == state)
            {
                return row.id.strip_prefix("provider-job:").unwrap().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    match result {
        Ok(id) => id,
        Err(_) => {
            let health = app
                .execute(ApplicationOperation::try_from(Operation::Health {}).unwrap())
                .await;
            panic!(
                "No {connector} delivery reached {state}; observed {observed:?}; health {health:?}"
            )
        }
    }
}
