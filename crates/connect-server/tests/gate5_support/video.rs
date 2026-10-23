//! Disposable accepted AV session. Fixture setup is public store API only;
//! participation and media use the public native SDK.
#![allow(clippy::unwrap_used, dead_code)]
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, WebRtcMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store};

pub struct Fixture {
    pub temporary: tempfile::TempDir,
    pub server: RunningServer,
    pub left: ConnectClient,
    pub right: ConnectClient,
    pub conversation: String,
    pub session: String,
    pub left_id: String,
    pub right_id: String,
    pub left_token: String,
    pub right_token: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("state");
        init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
        let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
        let mut left = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
        let mut right = ClientConfig::load(&directory.join("agent.toml")).unwrap();
        let store = Store::open(&config.database, config.limits.clone()).unwrap();
        let expiry = chrono::Utc::now().timestamp() + 3600;
        let lk = store
            .issue(
                &left.identity_id,
                Scope::communications(vec![right.identity_id.clone()]),
                expiry,
            )
            .unwrap();
        let rk = store
            .issue(
                &right.identity_id,
                Scope::communications(vec![left.identity_id.clone()]),
                expiry,
            )
            .unwrap();
        let conversation = match store
            .execute(
                &lk.id,
                &Operation::Create {
                    request_id: "video-create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        right.identity_id.clone(),
                    ]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("conversation"),
        };
        let session = match store
            .execute(
                &lk.id,
                &Operation::SessionInvite {
                    request_id: "video-invite".into(),
                    conversation_id: conversation.clone(),
                    participant_id: right.identity_id.clone(),
                    media: vec!["audio".into(), "video".into()],
                },
            )
            .unwrap()
        {
            Outcome::Session(s) => s.id,
            _ => panic!("session"),
        };
        store
            .execute(
                &rk.id,
                &Operation::SessionAccept {
                    request_id: "video-accept".into(),
                    conversation_id: conversation.clone(),
                    session_id: session.clone(),
                },
            )
            .unwrap();
        config.webrtc = Some(WebRtcMediaConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            route: InboundMediaRoute {
                credential_id: lk.id.clone(),
                conversation_id: conversation.clone(),
                assistant_identity_id: right.identity_id.clone(),
                session_id: Some(session.clone()),
            },
        });
        drop(store);
        let server =
            RunningServer::start_local(config, Some("127.0.0.1:0".parse().unwrap()), false)
                .await
                .unwrap();
        left.endpoint = server.address;
        let left_token = lk.token.clone();
        let right_token = rk.token.clone();
        left.credential = lk.token;
        left.credential_id = lk.id;
        right.endpoint = server.address;
        right.credential = rk.token;
        right.credential_id = rk.id;
        let left_id = left.identity_id.clone();
        let right_id = right.identity_id.clone();
        Self {
            temporary,
            server,
            left: ConnectClient::open(left).await.unwrap(),
            right: ConnectClient::open(right).await.unwrap(),
            conversation,
            session,
            left_id,
            right_id,
            left_token,
            right_token,
        }
    }
}
