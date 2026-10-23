//! Canonical acceptance owns a fresh public provider job; no manual start call.
#[path = "public_assistant/startup.rs"]
mod startup;
use super::*;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use connect_protocol::routing::{ReachabilityPolicy, RouteMedium};
use connect_server::{assistant_vapi::*, private_vapi::ProviderSecrets};
use connect_store::{DiscoveryRuntime, EndpointProfile, EndpointRuntime, VoiceRoute};
use sha2::Digest;
use std::sync::{
    atomic::{AtomicI64, AtomicUsize, Ordering},
    Arc,
};
#[derive(Clone)]
struct Provider {
    base: String,
    creates: Arc<AtomicUsize>,
    exports: Arc<AtomicUsize>,
    ends: Arc<AtomicUsize>,
    audio: Arc<AtomicUsize>,
    unconfirmed_end: bool,
    record: Option<(String, bool, bool)>,
    record_exports: Arc<AtomicUsize>,
    record_deadline: Arc<AtomicI64>,
}
async fn create_call(
    State(s): State<Provider>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer supervised-fixture-token"
    );
    assert_eq!(
        s.creates.fetch_add(1, Ordering::SeqCst),
        0,
        "duplicate public creation"
    );
    assert_eq!(body["assistantId"], "12345678-1234-1234-1234-123456789012");
    assert_eq!(body["assistantOverrides"]["model"]["tools"], json!([]));
    Json(
        json!({"id":"supervised-public-call","transport":{"websocketCallUrl":format!("{}/supervised-public-call/transport",s.base.replace("http:","ws:"))}}),
    )
}
async fn upgrade(
    State(s): State<Provider>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer supervised-fixture-token"
    );
    ws.on_upgrade(move |ws| socket(s, ws))
}
async fn socket(s: Provider, mut ws: WebSocket) {
    while let Some(Ok(frame)) = ws.recv().await {
        match frame {
            Message::Text(text) => {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["type"] == "add-message" {
                    assert_eq!(s.exports.fetch_add(1, Ordering::SeqCst), 0);
                    let text = v["message"]["content"].as_str().unwrap();
                    assert!(text.contains("supervised public history marker"));
                    if let Some((uuid, permitted, expiring)) = &s.record {
                        let view: connect_protocol::ContextView =
                            serde_json::from_str(text).unwrap();
                        if *permitted {
                            let item = view
                                .items
                                .iter()
                                .find(|item| item.source_id == *uuid)
                                .unwrap();
                            let record = item.record.as_ref().unwrap();
                            assert_eq!(
                                record.state,
                                connect_protocol::ContextRecordState::Available
                            );
                            assert_eq!(record.uuid, *uuid);
                            assert_eq!(record.session_id, "supervised-record-source");
                            assert_eq!(record.revision, 1);
                            assert_eq!(
                                serde_json::from_str::<Value>(&item.body).unwrap()["uuid"],
                                *uuid
                            );
                            assert_eq!(
                                record.content_hash.as_deref(),
                                Some(
                                    format!(
                                        "sha512-{}",
                                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                                            .encode(sha2::Sha512::digest(item.body.as_bytes()))
                                    )
                                    .as_str()
                                )
                            );
                            let reference = serde_json::to_string(record).unwrap();
                            assert!(
                                !reference.contains("body_hash")
                                    && !reference.contains("manifest_hash")
                            );
                            if *expiring {
                                let deadline = record.valid_until.unwrap();
                                assert!(deadline > chrono::Utc::now().timestamp());
                                s.record_deadline.store(deadline, Ordering::SeqCst);
                            }
                            s.record_exports.fetch_add(1, Ordering::SeqCst);
                        } else {
                            assert!(!view.items.iter().any(|item| item.record.is_some()));
                            assert!(
                                !text.contains(uuid),
                                "no owner fallback for missing record_get"
                            );
                        }
                    }
                    ws.send(Message::Text(json!({"type":"conversation-update","messagesOpenAIFormatted":[v["message"].clone()]}).to_string().into())).await.unwrap();
                }
                if v["type"] == "end-call" {
                    s.ends.fetch_add(1, Ordering::SeqCst);
                    if s.unconfirmed_end {
                        return;
                    }
                    let _ = ws
                        .send(Message::Text(
                            json!({"type":"status-update","status":"ended"})
                                .to_string()
                                .into(),
                        ))
                        .await;
                }
            }
            Message::Binary(bytes) => {
                if bytes.iter().any(|b| !matches!(*b, 0xff | 0x7f)) {
                    s.audio.fetch_add(1, Ordering::SeqCst);
                }
            }
            Message::Close(_) => return,
            _ => {}
        }
    }
}
#[derive(Clone, Copy)]
enum Case {
    Ready,
    RevokeContext,
    EndUnconfirmed,
    RequireFresh,
    MissingContext,
    MissingRuntime,
    RecordDeleted,
    RecordExpired,
    RecordNoPermission,
}
impl Case {
    fn started(self) -> bool {
        matches!(
            self,
            Self::Ready
                | Self::RevokeContext
                | Self::EndUnconfirmed
                | Self::RecordDeleted
                | Self::RecordExpired
                | Self::RecordNoPermission
        )
    }
    fn record_source(self) -> bool {
        matches!(
            self,
            Self::RecordDeleted | Self::RecordExpired | Self::RecordNoPermission
        )
    }
}
pub(super) async fn prior_record(
    client: &mut CanonicalClient,
    cid: &str,
    participants: Vec<String>,
) -> String {
    client
        .session_command(Request {
            operation_id: "supervised-record-invite".into(),
            conversation_id: cid.into(),
            session_id: "supervised-record-source".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants,
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    client
        .decide_session_invitation(connect_protocol::session_decisions::Request {
            operation_id: "supervised-record-cancel".into(),
            conversation_id: cid.into(),
            session_id: "supervised-record-source".into(),
            decision: connect_protocol::session_decisions::Decision::Cancel,
            by_participant: None,
            expected_revision: None,
            reason_code: 487,
            reason: "metadata outcome without capture".into(),
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let page = client
            .conversation_records(
                cid,
                rvoip_uctp::v2::payloads::ConversationGet {
                    page: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap();
        if let Some(uuid) = page.records.into_iter().find_map(|r| match r {
            connect_protocol::events::RecordAvailability::Ready {
                session_id,
                uuid,
                revision,
            } if session_id == "supervised-record-source" => {
                assert_eq!(revision, 1);
                Some(uuid)
            }
            _ => None,
        }) {
            return uuid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "prior source must become ready through the public SDK"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test]
async fn accepted_public_ai_starts_one_owned_provider_and_drains_before_server_retirement() {
    case(Case::Ready, false).await;
}
#[tokio::test]
async fn public_ai_unknown_reachability_refuses_without_provider_io() {
    case(Case::RequireFresh, false).await;
}
#[tokio::test]
async fn public_ai_acceptance_without_context_permission_has_no_owner_fallback() {
    case(Case::MissingContext, false).await;
}
#[tokio::test]
async fn public_ai_without_installed_endpoint_profile_has_no_provider_io() {
    case(Case::MissingRuntime, false).await;
}
#[tokio::test]
async fn http_public_ai_acceptance_starts_one_owned_signed_provider_peer() {
    case(Case::Ready, true).await;
}
async fn case(mode: Case, http: bool) {
    let codecs::Running {
        f,
        mut clients,
        cid,
        sh: _,
        th,
    } = codecs::setup().await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Identity { identity_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Identity {
                operation_id: "supervised-ai".into(),
                alias: "supervised-ai".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let connect_client::membership::MembershipOutcome::Participant { participant, .. } = clients[1]
        .participant_change(
            &cid,
            "supervised-member",
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":identity_id,"kind":"ai","role":"participant"}),
            ),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let record = if mode.record_source() {
        if matches!(mode, Case::RecordExpired) {
            f.server
                .store
                .configure_record_retention(&connect_protocol::record_retention::Policy {
                    name: "native-context-expiry".into(),
                    metadata: connect_protocol::record_retention::Lifetime::After { seconds: 30 },
                    ..Default::default()
                })
                .unwrap();
        }
        let participants = room
            .participants
            .iter()
            .filter(|p| p.identity_id != f.client.identity_id)
            .map(|p| p.participant_id.clone())
            .chain(std::iter::once(participant.participant_id.clone()))
            .collect();
        let uuid = prior_record(&mut clients[1], &cid, participants).await;
        if matches!(mode, Case::RecordExpired) {
            // A later default cannot extend this already frozen source lifetime.
            f.server
                .store
                .configure_record_retention(&Default::default())
                .unwrap();
        }
        Some(uuid)
    } else {
        None
    };
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
    if matches!(mode, Case::MissingContext) {
        scope.operations.retain(|s| s != "context_view");
    }
    if matches!(mode, Case::RecordNoPermission) {
        scope.operations.retain(|s| s != "record_get");
    }
    let config = party_with_scope(&f, &identity_id, 301, vec!["audio".into()], scope);
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: "supervised-vapi-endpoint".into(),
                identity_id: identity_id.clone(),
                device: DeviceSpec {
                    id: "supervised-vapi-service".into(),
                    kind: "service".into(),
                    platform: "loopback-fixture".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: "12345678-1234-1234-1234-123456789012".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("supervised_fixture_secret".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-supervised-vapi".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "designated-local-fixture".into(),
            },
        )
        .unwrap();
    f.server
        .store
        .configure_voice_route(
            &admin,
            &VoiceRoute {
                id: "supervised-public-voice".into(),
                conversation_id: cid.clone(),
                identity_id: identity_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550100".into()],
                enabled: true,
            },
        )
        .unwrap();
    clients[1]
        .send_message(
            "supervised-history",
            &cid,
            "supervised-history",
            connect_client::messages::MessageDraft::text_all("supervised public history marker"),
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        base: format!("http://{}", listener.local_addr().unwrap()),
        creates: Arc::new(AtomicUsize::new(0)),
        exports: Arc::new(AtomicUsize::new(0)),
        ends: Arc::new(AtomicUsize::new(0)),
        audio: Arc::new(AtomicUsize::new(0)),
        unconfirmed_end: matches!(mode, Case::EndUnconfirmed),
        record: record.as_ref().map(|uuid| {
            (
                uuid.clone(),
                !matches!(mode, Case::RecordNoPermission),
                matches!(mode, Case::RecordExpired),
            )
        }),
        record_exports: Arc::new(AtomicUsize::new(0)),
        record_deadline: Arc::new(AtomicI64::new(0)),
    };
    let app = Router::new()
        .route("/call", post(create_call))
        .route("/supervised-public-call/transport", get(upgrade))
        .with_state(p.clone());
    let fixture = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let endpoints = EndpointRuntime::new(if matches!(mode, Case::MissingRuntime) {
        vec![]
    } else {
        vec![EndpointProfile {
            transport: EndpointTransport::VapiVoice,
            media: vec![RouteMedium::Audio],
        }]
    })
    .unwrap()
    .with_secret_refs(vec![(
        EndpointTransport::VapiVoice,
        "supervised_fixture_secret".into(),
    )])
    .unwrap();
    let secrets: ProviderSecrets = Arc::new(|reference| {
        assert_eq!(reference, "supervised_fixture_secret");
        connect_server::secrets::SecretValue::new("supervised-fixture-token".into())
            .map_err(|_| connect_protocol::Error::forbidden())
    });
    let host = PublicVapiCoordinator::install(
        &f.server.service,
        vec![PublicVapiTarget {
            native_peer: config.clone(),
            provider_endpoint: endpoint_id,
            profile: AssistantVapiProfile {
                api_base: format!("{}/", p.base).parse().unwrap(),
                websocket_origin: format!("{}/", p.base.replace("http:", "ws:"))
                    .parse()
                    .unwrap(),
                loopback_fixture: true,
                model: "designated-supervised-model".into(),
                application_prompt: "Use only the supplied authorized public history.".into(),
                context_timing: AssistantProviderTiming::AfterCreation,
                observations: None,
            },
            routing: DiscoveryRuntime {
                endpoints,
                ..Default::default()
            },
            reachability: if matches!(mode, Case::RequireFresh) {
                ReachabilityPolicy::RequireFresh
            } else {
                ReachabilityPolicy::AllowUnknown
            },
        }],
        secrets,
    )
    .unwrap();
    let owner = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let revision = || {
        f.server
            .store
            .session_view(&owner, &cid, "native-session")
            .unwrap()
            .topology_revision
    };
    clients[1]
        .session_command(Request {
            operation_id: "supervised-invite".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision()),
            action: Action::Invite {
                participants: vec![participant.participant_id.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: f
                    .server
                    .store
                    .session_view(&owner, &cid, "native-session")
                    .unwrap()
                    .capabilities_offer
                    .unwrap(),
            },
        })
        .await
        .unwrap();
    assert_eq!(host.job_count(), 0, "invitation cannot auto-accept");
    let mut http_config = canonical_http_config(&f);
    http_config.credential = config.credential.clone();
    let mut http_ai = CanonicalHttpClient::open(http_config).await.unwrap();
    let ai_credential = f
        .server
        .store
        .authenticate(&config.credential)
        .unwrap()
        .credential_id;
    let mut ai = CanonicalClient::open(config).await.unwrap();
    let accept = Request {
        operation_id: "supervised-accept".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: Some(participant.participant_id),
        expected_revision: Some(revision()),
        action: Action::Accept {
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    if http {
        http_ai.session_command(accept.clone()).await.unwrap();
    } else {
        ai.session_command(accept.clone()).await.unwrap();
    }
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match host.status(
            &cid,
            "native-session",
            &identity_id,
            participant.membership_epoch,
        ) {
            Some(AssistantVapiStatus::ContextReady) => {
                assert!(mode.started());
                break;
            }
            Some(AssistantVapiStatus::Stopped { problem }) => {
                assert!(!mode.started(), "public initialization: {problem:?}");
                assert_eq!(
                    problem.as_deref(),
                    Some(if matches!(mode, Case::MissingContext) {
                        "forbidden"
                    } else {
                        "endpoint-unavailable"
                    })
                );
                assert_eq!(p.creates.load(Ordering::SeqCst), 0);
                break;
            }
            _ => {}
        }
        assert!(
            tokio::time::Instant::now() < until,
            "public job did not settle"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(host.job_count(), 1);
    if http {
        assert_eq!(
            http_ai.session_command(accept.clone()).await.unwrap(),
            http_ai.session_command(accept).await.unwrap()
        );
    } else {
        assert_eq!(
            ai.session_command(accept.clone()).await.unwrap(),
            ai.session_command(accept).await.unwrap()
        );
    }
    if mode.started() {
        use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
        let clock = tokio::time::Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        // A context ACK is not media readiness. Keep a real clocked sender
        // until the provider observes audio, within this bounded fixture lane.
        for seq in 0..400u32 {
            tick.tick().await;
            clients[1]
                .send_native_rtp_datagram(
                    &pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: th,
                        seq: 140000 + seq,
                        rtp: RtpMediaPayload {
                            payload: (0..160)
                                .map(|n| if n % 16 < 8 { 0x90 } else { 0x10 })
                                .collect::<Vec<_>>()
                                .into(),
                            payload_type: 0,
                            sequence_number: seq as u16,
                            timestamp: (clock.elapsed().as_micros() * 8 / 1000) as u32,
                            ssrc: 0x4736_0001,
                        },
                    })
                    .unwrap(),
                )
                .unwrap();
            if seq >= 39 && p.audio.load(Ordering::SeqCst) > 1 {
                break;
            }
        }
        assert!(
            p.audio.load(Ordering::SeqCst) > 1,
            "owned public peer did not forward actual native audio; record deadline {}, now {}, status {:?}",
            p.record_deadline.load(Ordering::SeqCst),
            chrono::Utc::now().timestamp(),
            host.status(&cid, "native-session", &identity_id, participant.membership_epoch)
        );
        assert_eq!(p.creates.load(Ordering::SeqCst), 1);
        assert_eq!(p.exports.load(Ordering::SeqCst), 1);
    }
    if mode.record_source() {
        assert_eq!(
            p.record_exports.load(Ordering::SeqCst),
            if matches!(mode, Case::RecordNoPermission) {
                0
            } else {
                1
            }
        );
    }
    if matches!(mode, Case::RecordDeleted) {
        let mut config = canonical_http_config(&f);
        config.credential = f.admin.clone();
        let mut eraser = connect_client::application::ApplicationClient::open(config)
            .await
            .unwrap();
        eraser
            .mutate_record(connect_protocol::record_mutations::Request {
                operation_id: "supervised-record-delete".into(),
                conversation_id: cid.clone(),
                session_id: "supervised-record-source".into(),
                uuid: record.clone().unwrap(),
                expected_revision: 1,
                action: connect_protocol::record_mutations::Action::Delete,
            })
            .await
            .unwrap();
    }
    if matches!(
        mode,
        Case::RevokeContext | Case::RecordDeleted | Case::RecordExpired
    ) {
        if matches!(mode, Case::RevokeContext) {
            let store = f.server.store.clone();
            let administrator = admin.clone();
            tokio::task::spawn_blocking(move || {
                store.revoke(&store.revalidate(&administrator)?, &ai_credential)
            })
            .await
            .unwrap()
            .unwrap();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
        loop {
            if let Some(AssistantVapiStatus::Stopped { problem }) = host.status(
                &cid,
                "native-session",
                &identity_id,
                participant.membership_epoch,
            ) {
                assert!(
                    problem.is_some(),
                    "authority loss must remain an operation failure"
                );
                assert_eq!(
                    p.ends.load(Ordering::SeqCst),
                    1,
                    "original provider must actually terminate"
                );
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "authority-loss teardown did not finish"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(p.creates.load(Ordering::SeqCst), 1);
        assert_eq!(p.exports.load(Ordering::SeqCst), 1);
        if mode.record_source() {
            // Source loss must not be confused with expiry/revocation of the AI.
            assert!(ai
                .list_conversations(Default::default())
                .await
                .unwrap()
                .conversations
                .iter()
                .any(|room| room.id == cid));
        }
        if matches!(mode, Case::RecordExpired) {
            assert!(p.record_deadline.load(Ordering::SeqCst) > 0);
            assert!(chrono::Utc::now().timestamp() >= p.record_deadline.load(Ordering::SeqCst));
        }
    }
    if matches!(mode, Case::EndUnconfirmed) {
        for _ in 0..2 {
            let failure = f
                .server
                .service
                .drain_session_initialization()
                .await
                .unwrap_err();
            assert_eq!(
                failure.code, "public-provider-unconfirmed",
                "repeated drain cannot erase missing terminal evidence"
            );
        }
    }
    let shutdown = f.server.shutdown().await;
    if matches!(mode, Case::EndUnconfirmed) {
        let error = shutdown.unwrap_err();
        assert!(
            error.to_string().contains("public-provider-unconfirmed"),
            "actual missing terminal evidence must remain unconfirmed: {error:?}"
        );
    } else {
        assert!(shutdown.is_ok(), "owned public shutdown: {shutdown:?}");
    }
    assert_eq!(
        p.ends.load(Ordering::SeqCst),
        if mode.started() { 1 } else { 0 }
    );
    ai.close();
    for c in clients {
        c.close();
    }
    fixture.abort();
}

#[tokio::test]
async fn public_ai_authority_loss_keeps_operation_failure_but_confirms_original_owned_teardown() {
    case(Case::RevokeContext, false).await;
}

#[tokio::test]
async fn public_ai_missing_provider_terminal_evidence_cannot_report_clean_owned_teardown() {
    case(Case::EndUnconfirmed, false).await;
}

#[tokio::test]
async fn public_ai_record_context_exports_exact_source_then_erasure_stops_original_memory() {
    for http in [false, true] {
        case(Case::RecordDeleted, http).await;
    }
}
#[tokio::test]
async fn public_ai_record_context_without_record_permission_has_no_owner_fallback() {
    case(Case::RecordNoPermission, false).await;
}
#[tokio::test]
async fn public_ai_record_context_retention_stops_original_memory_without_revoking_identity() {
    case(Case::RecordExpired, false).await;
}
