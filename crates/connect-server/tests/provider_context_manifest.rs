//! Preserve canonical public context through manifest admission and the real Vapi SDK.
#![allow(clippy::unwrap_used)]
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig, ApplicationOperation},
    canonical::CanonicalHttpClient,
    ClientConfig, MessageDraft,
};
use connect_connectors::Vapi;
use connect_protocol::ContextView;
use connect_protocol::{Operation, Outcome};
use connect_server::providers::Worker;
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::{Route, Scope};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";

struct Harness {
    _temp: tempfile::TempDir,
    server: RunningCanonicalServer,
    owner: CanonicalHttpClient,
    application: ApplicationClient,
    owner_config: ClientConfig,
    peer_id: String,
    cid: String,
}
impl Harness {
    async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let old = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let mut owner_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
        let peer = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let signing = temp.path().join("canonical-server.der");
        std::fs::write(&signing, key.serialize_der()).unwrap();
        let server = RunningCanonicalServer::start(CanonicalServerConfig {
            network_profile: Default::default(),
            webtransport: None,
            websocket: None,
            session_reconnect_grace_seconds: 30,
            bind: old.bind,
            database: old.database,
            certificate: old.certificate,
            private_key: old.private_key,
            signing_key_id: "context-manifest-server".into(),
            signing_algorithm: "EdDSA".into(),
            signing_private_key: signing,
            signing_key_not_before: chrono::Utc::now() - chrono::Duration::seconds(1),
            signing_key_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            http_bind: Some("127.0.0.1:0".parse().unwrap()),
            record_https: None,
            record_retention: Default::default(),
            user_assurance: Vec::new(),
            telephone: None,
            public_vapi: None,
            provider_voice: None,
            provider_messages: None,
            limits: old.limits,
        })
        .await
        .unwrap();
        let credential = server
            .store
            .issue(
                &owner_config.identity_id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        owner_config.credential_id = credential.id;
        owner_config.credential = credential.token;
        let config = ApplicationClientConfig {
            endpoint: server.http_address.unwrap(),
            credential: owner_config.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 10_000,
        };
        let mut owner = CanonicalHttpClient::open(config.clone()).await.unwrap();
        let application = ApplicationClient::open(config).await.unwrap();
        let mut request = connect_client::conversations::CreateRequest::persistent(vec![peer
            .identity_id
            .clone()]);
        request.tenant_id = Some("tnt_default".into());
        let cid = owner
            .create_conversation("context-manifest-conversation", request)
            .await
            .unwrap()
            .id;
        Self {
            _temp: temp,
            server,
            owner,
            application,
            owner_config,
            peer_id: peer.identity_id,
            cid,
        }
    }
    async fn close(self) {
        self.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn escaped_ordinary_context_over_128k_reaches_vapi_intact_with_source_provenance() {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/assistant/{ASSISTANT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ASSISTANT,
            "model": {"messages": [{"role": "system", "content": "History: {{connect_context}}"}]}
        })))
        .expect(1)
        .mount(&provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(body_partial_json(
            json!({"input": "Use the attributed history"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "owned-loopback-chat",
            "output": [{"role": "assistant", "content": "Fixture answer"}]
        })))
        .expect(1)
        .mount(&provider)
        .await;
    let adapter = Arc::new(Vapi::new("owned-loopback-key", &provider.uri()).unwrap());
    let mut h = Harness::start().await;
    let body = "\u{1}".repeat(8_000);
    let mut original = Vec::new();
    // Each public send fits the established wire and message limits. Select the
    // route afterward so these are history, rather than oversized provider input.
    for index in 0..3 {
        original.push(
            h.owner
                .send_message(
                    &format!("escaped-history-{index}"),
                    &h.cid,
                    &format!("escaped-history-message-{index}"),
                    MessageDraft::text_all(&body),
                )
                .await
                .unwrap(),
        );
    }
    let delegate = h
        .server
        .store
        .issue(
            &h.peer_id,
            Scope::messaging(vec![h.owner_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    h.server
        .store
        .configure_route(
            &h.owner_config.credential_id,
            &Route {
                id: "escaped-context-vapi".into(),
                identity_id: h.peer_id.clone(),
                credential_id: delegate.id,
                conversation_id: h.cid.clone(),
                connector: "vapi".into(),
                target: ASSISTANT.into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let worker = Worker::with_drivers(h.server.service.clone(), vec![adapter])
        .await
        .unwrap();
    let question = h
        .owner
        .send_message(
            "escaped-context-question",
            &h.cid,
            "escaped-context-question-message",
            MessageDraft::text_all("Use the attributed history"),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let Outcome::ProviderStatus { deliveries: jobs } = h
                .application
                .execute(
                    ApplicationOperation::try_from(Operation::ProviderStatus {
                        conversation_id: h.cid.clone(),
                    })
                    .unwrap(),
                )
                .await
                .unwrap()
            else {
                panic!("provider status")
            };
            if jobs.len() == 1 && jobs[0].state == "completed" {
                break;
            }
            assert!(!jobs.iter().any(|job| job.state == "failed"));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let requests = provider.received_requests().await.unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    let request: Value = posts[0].body_json().unwrap();
    assert_eq!(request["assistantId"], ASSISTANT);
    assert_eq!(request["input"], "Use the attributed history");
    assert!(request.get("previousChatId").is_none());
    let context = request["assistantOverrides"]["variableValues"]["connect_context"]
        .as_str()
        .unwrap();
    let (_, history) = context.split_once("History JSON: ").unwrap();
    assert!(history.len() > 128_000);
    let selected: ContextView = serde_json::from_str(history).unwrap();
    assert_eq!(selected.conversation_id, h.cid);
    for receipt in &original {
        let source = selected
            .items
            .iter()
            .find(|item| item.source_id == receipt.message_id)
            .unwrap();
        assert_eq!(source.body, body);
        assert_eq!(source.actor_id, h.owner_config.identity_id);
        assert!(source.sequence > 0 && !source.kind.is_empty());
    }
    assert!(selected
        .items
        .iter()
        .any(|item| item.source_id == question.message_id));
    assert!(selected
        .items
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert!(!request.to_string().contains("owned-loopback-key"));
    assert_eq!(
        h.owner
            .send_message(
                "escaped-context-question",
                &h.cid,
                "escaped-context-question-message",
                MessageDraft::text_all("Use the attributed history")
            )
            .await
            .unwrap()
            .message_id,
        question.message_id
    );
    worker.shutdown().await.unwrap();
    assert_eq!(
        provider
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    h.close().await;
}
