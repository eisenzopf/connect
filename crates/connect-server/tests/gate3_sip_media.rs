#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

mod gate3_support;

use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, SipMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store, VoiceRoute};
use gate3_support::FakeVapi;
use rvoip_sip::{AudioFrame, Config as SipConfig, StreamPeer};
use std::time::Duration;

fn free_udp_address() -> std::net::SocketAddr {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
}

#[tokio::test]
async fn inbound_sip_uses_configured_authority_and_bridges_vapi_audio() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut fake = FakeVapi::start().await;
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
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
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "sip-create".into(),
                participants: vec![agent.identity_id.clone()],
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
                request_id: "sip-context".into(),
                conversation_id: conversation.id.clone(),
                body: "The SIP route fact is amber-four.".into(),
            },
        )
        .unwrap();
    store
        .configure_voice_route(
            &operator.id,
            &VoiceRoute {
                id: "sip-vapi-route".into(),
                conversation_id: conversation.id.clone(),
                identity_id: agent.identity_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+14155550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let sip_address = free_udp_address();
    config.sip = Some(SipMediaConfig {
        bind: sip_address,
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
    assert_eq!(server.sip_address, Some(sip_address));
    let caller_address = free_udp_address();
    let mut caller = StreamPeer::with_config(SipConfig::local("caller", caller_address.port()))
        .await
        .unwrap();
    let call_id = caller
        .invite(format!("sip:connect@{sip_address}"))
        .send()
        .await
        .unwrap();
    let call = tokio::time::timeout(Duration::from_secs(10), caller.wait_for_answered(&call_id))
        .await
        .unwrap()
        .unwrap();
    let context = fake.next("add-message").await.to_string();
    assert!(context.contains("amber-four"));

    let (sender, mut receiver) = call.audio().await.unwrap().split();
    for sequence in 0..4 {
        sender
            .send(AudioFrame::new(vec![1200; 160], 8_000, 1, sequence * 160))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(fake.next("audio").await["size"], 160);
    let returned = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .expect("Vapi audio should reach the SIP caller");
    assert_eq!(returned.sample_rate, 8_000);
    assert!(!returned.samples.is_empty());

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = match server
                .store
                .execute(
                    &app_key.id,
                    &Operation::ContextView {
                        conversation_id: conversation.id.clone(),
                        limit: 100,
                    },
                )
                .unwrap()
            {
                Outcome::ContextView(value) => value,
                other => panic!("expected context, got {other:?}"),
            };
            if context
                .items
                .iter()
                .any(|item| item.kind == "voice_transcript")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();

    // The paired Vapi supervisor may already have converged the SIP dialog
    // after its final report. A second local hangup is idempotent at Connect's
    // durable Session boundary even when the peer handle is already gone.
    let _ = call.hangup().await;
    let context = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = match server
                .store
                .execute(
                    &app_key.id,
                    &Operation::ContextView {
                        conversation_id: conversation.id.clone(),
                        limit: 100,
                    },
                )
                .unwrap()
            {
                Outcome::ContextView(value) => value,
                other => panic!("expected context, got {other:?}"),
            };
            if value.items.iter().any(|item| item.kind == "session_ended") {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "voice_transcript"));
    assert!(
        !conversation.closed,
        "a bounded SIP leg keeps its conversation"
    );

    caller.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

struct OutboundHarness {
    _temporary: tempfile::TempDir,
    fake: FakeVapi,
    server: RunningServer,
    client: ConnectClient,
    conversation_id: String,
    session_id: String,
}

impl OutboundHarness {
    async fn start() -> Self {
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
                    participants: vec![agent.identity_id.clone()],
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
                    body: "The outbound SIP fact is violet-nine.".into(),
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
                assistant_identity_id: agent.identity_id,
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
        }
    }

    async fn shutdown(self) {
        self.client.close().await;
        self.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn sdk_originates_sip_inside_the_durable_session_and_bridges_vapi_audio() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut harness = OutboundHarness::start().await;
    let peer_address = free_udp_address();
    let mut peer = StreamPeer::with_config(SipConfig::local("peer", peer_address.port()))
        .await
        .unwrap();
    let leg = harness
        .client
        .sip_dial(
            "outbound-sip-dial",
            &harness.conversation_id,
            &harness.session_id,
            &format!("sip:peer@{peer_address}"),
        )
        .await
        .unwrap();
    assert_eq!(leg.state, "pending");
    let incoming = tokio::time::timeout(Duration::from_secs(10), peer.wait_for_incoming())
        .await
        .unwrap()
        .unwrap();
    let call = incoming.accept().await.unwrap();
    let context = harness.fake.next("add-message").await.to_string();
    assert!(context.contains("violet-nine"));

    let (sender, mut receiver) = call.audio().await.unwrap().split();
    for sequence in 0..4 {
        sender
            .send(AudioFrame::new(vec![1400; 160], 8_000, 1, sequence * 160))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(harness.fake.next("audio").await["size"], 160);
    let returned = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .expect("Vapi audio should reach the outbound SIP peer");
    assert_eq!(returned.sample_rate, 8_000);

    let active = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = harness
                .client
                .sip_status(&harness.conversation_id, &leg.id)
                .await
                .unwrap();
            if current.state == "active" {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(active.connection_id.is_some());
    harness
        .client
        .sip_end("outbound-sip-end", &harness.conversation_id, &leg.id)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = harness
                .client
                .sip_status(&harness.conversation_id, &leg.id)
                .await
                .unwrap();
            if current.state == "ended" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(sender);
    drop(receiver);
    drop(call);
    peer.shutdown().await.unwrap();
    harness.shutdown().await;
}

#[tokio::test]
async fn sdk_sip_busy_cancel_and_no_answer_have_stable_terminal_states() {
    let mut harness = OutboundHarness::start().await;
    let busy_address = free_udp_address();
    let mut busy_peer = StreamPeer::with_config(SipConfig::local("busy-peer", busy_address.port()))
        .await
        .unwrap();
    let busy = harness
        .client
        .sip_dial(
            "outbound-sip-busy",
            &harness.conversation_id,
            &harness.session_id,
            &format!("sip:peer@{busy_address}"),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), busy_peer.wait_for_incoming())
        .await
        .unwrap()
        .unwrap()
        .reject_busy();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = harness
                .client
                .sip_status(&harness.conversation_id, &busy.id)
                .await
                .unwrap();
            if current.state == "failed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    busy_peer.shutdown().await.unwrap();
    harness.shutdown().await;

    let mut harness = OutboundHarness::start().await;
    let cancel_address = free_udp_address();
    let mut cancel_peer =
        StreamPeer::with_config(SipConfig::local("cancel-peer", cancel_address.port()))
            .await
            .unwrap();
    let cancelled = harness
        .client
        .sip_dial(
            "outbound-sip-cancel-before-answer",
            &harness.conversation_id,
            &harness.session_id,
            &format!("sip:peer@{cancel_address}"),
        )
        .await
        .unwrap();
    let unanswered = tokio::time::timeout(Duration::from_secs(5), cancel_peer.wait_for_incoming())
        .await
        .unwrap()
        .unwrap();
    harness
        .client
        .sip_end(
            "outbound-sip-cancel-before-answer-end",
            &harness.conversation_id,
            &cancelled.id,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = harness
                .client
                .sip_status(&harness.conversation_id, &cancelled.id)
                .await
                .unwrap();
            if current.state == "cancelled" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(unanswered);
    cancel_peer.shutdown().await.unwrap();
    harness.shutdown().await;

    let mut harness = OutboundHarness::start().await;
    let no_answer_address = free_udp_address();
    let mut no_answer_peer =
        StreamPeer::with_config(SipConfig::local("no-answer-peer", no_answer_address.port()))
            .await
            .unwrap();
    let no_answer = harness
        .client
        .sip_dial(
            "outbound-sip-no-answer",
            &harness.conversation_id,
            &harness.session_id,
            &format!("sip:peer@{no_answer_address}"),
        )
        .await
        .unwrap();
    let _unanswered =
        tokio::time::timeout(Duration::from_secs(5), no_answer_peer.wait_for_incoming())
            .await
            .unwrap()
            .unwrap();
    tokio::time::timeout(Duration::from_secs(35), async {
        loop {
            let current = harness
                .client
                .sip_status(&harness.conversation_id, &no_answer.id)
                .await
                .unwrap();
            if current.state == "failed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    no_answer_peer.shutdown().await.unwrap();
    harness.shutdown().await;
}
