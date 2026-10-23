#![allow(clippy::unwrap_used, dead_code)] // Shared loopback fixture; each lane uses a subset.
//! Outbound SIP fixtures use the native SDK and the actual local server worker.
use crate::gate3_support::FakeVapi;
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, SipMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store, VoiceRoute};
fn free_udp_address() -> std::net::SocketAddr {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

pub(crate) struct OutboundHarness {
    _temporary: tempfile::TempDir,
    pub(crate) fake: FakeVapi,
    pub(crate) server: RunningServer,
    pub(crate) client: ConnectClient,
    pub(crate) conversation_id: String,
    pub(crate) session_id: String,
    pub(crate) owner_credential_id: String,
    pub(crate) participant_id: String,
}

impl OutboundHarness {
    pub(crate) async fn start() -> Self {
        let fake = FakeVapi::start().await;
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
        let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        let store = Store::open(&config.database, config.limits.clone()).unwrap();
        let expiry = chrono::Utc::now().timestamp() + 3_600;
        let operator = store
            .issue(&app.identity_id, Scope::operator(), expiry)
            .unwrap();
        let app_key = store
            .issue(
                &app.identity_id,
                Scope::communications(vec![agent.identity_id.clone()]),
                expiry,
            )
            .unwrap();
        let agent_key = store
            .issue(
                &agent.identity_id,
                Scope::communications(vec![app.identity_id.clone()]),
                expiry,
            )
            .unwrap();
        let conversation = match store
            .execute(
                &app_key.id,
                &Operation::Create {
                    request_id: "outbound-sip-create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        agent.identity_id.clone(),
                    ]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(value) => value,
            other => panic!("expected conversation, got {other:?}"),
        };
        store
            .execute(
                &app_key.id,
                &Operation::Send {
                    request_id: "outbound-sip-context".into(),
                    conversation_id: conversation.id.clone(),
                    message: connect_protocol::messages::MessageDraft::text_all(
                        "The outbound SIP fact is violet-nine.",
                    ),
                },
            )
            .unwrap();
        let session = match store
            .execute(
                &app_key.id,
                &Operation::SessionInvite {
                    request_id: "outbound-sip-invite".into(),
                    conversation_id: conversation.id.clone(),
                    participant_id: agent.identity_id.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        {
            Outcome::Session(value) => value,
            other => panic!("expected session, got {other:?}"),
        };
        store
            .execute(
                &agent_key.id,
                &Operation::SessionAccept {
                    request_id: "outbound-sip-accept".into(),
                    conversation_id: conversation.id.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        store
            .configure_voice_route(
                &operator.id,
                &VoiceRoute {
                    id: "outbound-sip-vapi-route".into(),
                    conversation_id: conversation.id.clone(),
                    identity_id: agent.identity_id.clone(),
                    assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                    phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                    destinations: vec!["+14155550101".into()],
                    enabled: true,
                },
            )
            .unwrap();
        config.sip = Some(SipMediaConfig {
            bind: free_udp_address(),
            media_bind_ip: None,
            advertised: None,
            media_public: None,
            outbound: None,
            route: InboundMediaRoute {
                credential_id: app_key.id.clone(),
                conversation_id: conversation.id.clone(),
                assistant_identity_id: agent.identity_id.clone(),
                session_id: None,
            },
        });
        drop(store);

        let server = RunningServer::start_with_vapi_media(config, None, fake.adapter.clone())
            .await
            .unwrap();
        app.endpoint = server.address;
        app.credential_id = app_key.id;
        app.credential = app_key.token;
        let client = ConnectClient::open(app).await.unwrap();
        Self {
            _temporary: temporary,
            fake,
            server,
            client,
            conversation_id: conversation.id,
            session_id: session.id,
            owner_credential_id: operator.id,
            participant_id: agent.identity_id,
        }
    }

    pub(crate) async fn shutdown(self) {
        self.client.close().await;
        self.server.shutdown().await.unwrap();
    }
}
