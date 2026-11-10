//! Real signed WSS SDP, Core reservation and RTC; no media readiness claim.
use super::*;
#[path = "webrtc/admission.rs"]
mod admission;
#[path = "webrtc/cleanup.rs"]
mod cleanup;
#[path = "webrtc/subscriptions.rs"]
mod subscriptions;
use connect_protocol::{
    session_control::{Action, Request},
    transitions::{OperationAction, OperationState},
};
use connect_server::canonical_webrtc::SignalingHost;
use connect_service::canonical_webrtc::Host;
use rvoip_core::capability::CodecInfo;
use rvoip_webrtc::{
    peer::{PeerRole, RvoipPeerConnection},
    WebRtcConfig,
};

fn audio_config() -> WebRtcConfig {
    let mut c = WebRtcConfig::loopback();
    c.capabilities.audio_codecs = vec![CodecInfo::from_name_with_defaults("g.711-mu")];
    c
}
async fn provision(f: &Fixture) -> String {
    provision_config(f, &f.client, 0).await
}
async fn provision_config(f: &Fixture, config: &CanonicalClientConfig, index: usize) -> String {
    provision_media_config(f, config, index, &["audio"]).await
}
async fn provision_media_config(
    f: &Fixture,
    config: &CanonicalClientConfig,
    index: usize,
    kinds: &[&str],
) -> String {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("wss-webrtc-endpoint-{index}"),
                identity_id: config.identity_id.clone(),
                device: DeviceSpec {
                    id: config.device_id.clone(),
                    kind: config.device_kind.clone(),
                    platform: config.device_platform.clone(),
                },
                transport: EndpointTransport::Webrtc,
                address: format!("https://localhost/webrtc/{index}"),
                capabilities: kinds.iter().map(|k| (*k).into()).collect(),
                secret_ref: None,
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
                operation_id: format!("wss-webrtc-verify-{index}"),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "designated-loopback-wss".into(),
            },
        )
        .unwrap();
    endpoint_id
}
async fn solo(f: &Fixture) -> String {
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input = connect_client::conversations::CreateRequest::persistent(vec![]);
    input.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("wss-webrtc-room", input)
        .await
        .unwrap();
    native
        .session_command(Request {
            operation_id: "wss-webrtc-session".into(),
            conversation_id: room.id.clone(),
            session_id: "wss-webrtc-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    native.shutdown().await.unwrap();
    room.id
}
fn offer(cid: &str, sdp: &str) -> Value {
    let mut e = envelope(
        "connection.offer",
        json!({"substrate":"websocket+webrtc","capabilities":{"audio_codecs":[{"name":"g.711-mu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":1,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"original-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"substrate_setup":{"kind":"websocket+webrtc","sdp":sdp}}),
    );
    e["cid"] = json!(cid);
    e["sid"] = json!("wss-webrtc-session");
    e["connid"] = json!("wss-webrtc-connection");
    e["operation_id"] = json!("wss-webrtc-offer");
    e["expected_revision"] = json!(0);
    e
}
fn status(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.server
        .store
        .execute_operation_action(
            &f.server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id,
            OperationAction::Get {
                operation_id: "wss-webrtc-offer".into(),
            },
        )
        .unwrap()
}
async fn retired(f: &Fixture, host: &SignalingHost) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if host.available_slots() == 1
                && host.adapter().route_count() == 0
                && status(f).state == OperationState::Failed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_wss_webrtc_answers_original_scope_once_and_joins_rtc_before_retirement() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = configured(algorithm, 8, None).await;
        let host = SignalingHost::new(audio_config(), 1).unwrap();
        f.server
            .service
            .install_canonical_webrtc(host.clone())
            .unwrap();
        assert!(f
            .server
            .service
            .install_canonical_webrtc(host.clone())
            .is_err());
        provision(&f).await;
        let cid = solo(&f).await;
        let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
            .await
            .unwrap();
        let sdp = caller.create_offer_and_gather().await.unwrap();
        let mut ws = Peer::open(&f).await;
        // Configuration seals after actual physical admission.
        assert!(f
            .server
            .service
            .install_canonical_webrtc(host.clone())
            .is_err());
        let original = offer(&cid, &sdp);
        let id = ws.send(original.clone(), true).await;
        let answer = ws.read().await;
        assert_eq!(
            answer.kind(),
            "connection.answer",
            "{:?}",
            answer.original()
        );
        assert_eq!(answer.original()["in_reply_to"], id);
        for scope in ["cid", "sid", "connid", "operation_id"] {
            assert_eq!(answer.original()[scope], original[scope]);
        }
        assert_eq!(
            answer.original()["payload"]["streams_answered"][0]["id"],
            "original-audio"
        );
        assert_eq!(
            answer.original()["payload"]["streams_answered"][0]["codec"]["params"]["payload_type"],
            0
        );
        assert_eq!(host.available_slots(), 0);
        assert_eq!(host.adapter().route_count(), 1);
        assert_eq!(host.adapter().total_allocations(), 1);
        caller
            .set_remote_answer(
                answer.original()["payload"]["substrate_setup"]["sdp"]
                    .as_str()
                    .unwrap(),
            )
            .await
            .unwrap();
        caller.wait_connected(Duration::from_secs(5)).await.unwrap();
        assert_eq!(status(&f).state, OperationState::Preparing);
        assert!(status(&f).result.is_none());
        assert_eq!(
            f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .connections,
            1,
            "SDP uses one dormant Core reservation; it is not media readiness"
        );
        let mut replay = original.clone();
        replay["id"] = json!(uuid::Uuid::new_v4().to_string());
        let replay_id = ws.send(replay, true).await;
        let mut observed = false;
        for _ in 0..128 {
            let e = ws.read().await;
            assert_eq!(e.original()["in_reply_to"], replay_id);
            assert_ne!(e.kind(), "connection.answer");
            if e.kind() == "operation.updated" {
                assert_eq!(e.original()["payload"]["state"], "preparing");
                observed = true;
            }
            if e.kind() == "ack" {
                break;
            }
        }
        assert!(observed);
        assert_eq!(host.adapter().route_count(), 1);
        assert_eq!(host.adapter().total_allocations(), 1);
        assert_eq!(
            host.available_slots(),
            0,
            "replay cannot allocate a second peer"
        );
        ws.socket.close(None).await.unwrap();
        drop(ws);
        retired(&f, &host).await;
        caller.close().await.unwrap();
        assert_eq!(status(&f).effects.len(), 1);
        f.server.shutdown().await.unwrap();
    }
}

struct AlteredHost {
    inner: Arc<SignalingHost>,
    field: &'static str,
}
struct AlteredAttempt {
    inner: Arc<dyn connect_service::canonical_webrtc::Attempt>,
    field: &'static str,
}
impl connect_service::canonical_webrtc::Host for AlteredHost {
    fn adapter(&self) -> Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter> {
        use connect_service::canonical_webrtc::Host;
        self.inner.adapter()
    }
    fn reserve_physical(
        &self,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: tokio_util::sync::CancellationToken,
    ) -> connect_protocol::Result<Arc<rvoip_webrtc::canonical::WebRtcPhysicalBinding>> {
        use connect_service::canonical_webrtc::Host;
        self.inner.reserve_physical(peer, physical)
    }
    fn start(
        &self,
        p: connect_service::canonical_webrtc::Preparation,
    ) -> Arc<dyn connect_service::canonical_webrtc::Attempt> {
        use connect_service::canonical_webrtc::Host;
        Arc::new(AlteredAttempt {
            inner: self.inner.start(p),
            field: self.field,
        })
    }
}
#[async_trait::async_trait]
impl connect_service::canonical_webrtc::Attempt for AlteredAttempt {
    fn original_attempt(&self) -> &connect_store::NativeOfferLease {
        self.inner.original_attempt()
    }
    fn release_invocation(&self) {
        self.inner.release_invocation();
    }
    async fn answer(
        &self,
        deadline: tokio::time::Instant,
    ) -> connect_protocol::Result<rvoip_uctp::v2::payloads::ConnectionAnswer> {
        let result = self.inner.answer(deadline).await;
        if let Err(error) = &result {
            eprintln!("RTC backend answer refused: {}", error.code);
        }
        let mut value = serde_json::to_value(result?).unwrap();
        match self.field {
            "id" => value["streams_answered"][0]["id"] = json!("another-stream"),
            "codec" => value["streams_answered"][0]["codec"]["name"] = json!("pcma"),
            "participant" => value["by_participant"] = json!("another-participant"),
            _ => unreachable!(),
        }
        Ok(serde_json::from_value(value).unwrap())
    }
    async fn shutdown(&self) -> connect_protocol::Result<()> {
        self.inner.shutdown().await
    }
    async fn closed(&self) -> connect_protocol::Result<()> {
        self.inner.closed().await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn canonical_wss_refuses_engine_answer_outside_current_stream_codec_or_member_plan() {
    for field in ["id", "codec", "participant"] {
        let f = configured("EdDSA", 8, None).await;
        let host = SignalingHost::new(audio_config(), 1).unwrap();
        f.server
            .service
            .install_canonical_webrtc(Arc::new(AlteredHost {
                inner: host.clone(),
                field,
            }))
            .unwrap();
        provision(&f).await;
        let cid = solo(&f).await;
        let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
            .await
            .unwrap();
        let sdp = caller.create_offer_and_gather().await.unwrap();
        let mut ws = Peer::open(&f).await;
        let id = ws.send(offer(&cid, &sdp), true).await;
        let refusal = ws.read().await;
        assert_eq!(refusal.kind(), "error", "{:?}", refusal.original());
        assert_eq!(refusal.original()["in_reply_to"], id);
        assert_eq!(
            refusal.original()["payload"]["code"],
            488,
            "{:?}",
            refusal.original()["payload"]
        );
        retired(&f, &host).await;
        assert!(!caller.is_connected());
        // A bounded unrelated control round trip still works on this physical.
        ws.create(&f).await;
        caller.close().await.unwrap();
        ws.socket.close(None).await.unwrap();
        drop(ws);
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_endpoint_revocation_after_signed_sdp_retires_rtc_and_preserves_wss_control() {
    let f = configured("EdDSA", 8, None).await;
    let host = SignalingHost::new(audio_config(), 1).unwrap();
    f.server
        .service
        .install_canonical_webrtc(host.clone())
        .unwrap();
    let endpoint = provision(&f).await;
    let cid = solo(&f).await;
    let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
        .await
        .unwrap();
    let sdp = caller.create_offer_and_gather().await.unwrap();
    let mut ws = Peer::open(&f).await;
    let original = offer(&cid, &sdp);
    ws.send(original.clone(), true).await;
    let answer = ws.read().await;
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    caller
        .set_remote_answer(
            answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .unwrap(),
        )
        .await
        .unwrap();
    caller.wait_connected(Duration::from_secs(5)).await.unwrap();
    let store = f.server.store.clone();
    let admin = store.authenticate(&f.admin).unwrap().credential_id;
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            store.execute_provisioning(
                &admin,
                ProvisioningCommand::RevokeEndpoint {
                    operation_id: "revoke-wss-webrtc".into(),
                    endpoint_id: endpoint,
                    expected_revision: 1,
                },
            )
        }),
    )
    .await
    .expect("SDP completion must release its originating Store reader")
    .unwrap()
    .unwrap();
    retired(&f, &host).await;
    assert_eq!(status(&f).effects.len(), 1);
    assert!(status(&f).result.is_none());
    ws.create(&f).await;
    let mut replay = original;
    replay["id"] = json!(uuid::Uuid::new_v4().to_string());
    let replay_id = ws.send(replay, true).await;
    // Exact historical replay remains readable under current credential/member
    // authorization. It cannot allocate media through the revoked Endpoint.
    let mut observed = false;
    for _ in 0..128 {
        let frame = ws.read().await;
        assert_eq!(frame.original()["in_reply_to"], replay_id);
        assert_ne!(frame.kind(), "connection.answer");
        if frame.kind() == "operation.updated" {
            assert_eq!(frame.original()["payload"]["state"], "failed");
            observed = true;
        }
        if frame.kind() == "ack" {
            break;
        }
    }
    assert!(observed);
    assert_eq!(host.available_slots(), 1);
    caller.close().await.unwrap();
    ws.socket.close(None).await.unwrap();
    drop(ws);
    f.server.shutdown().await.unwrap();
}

// The real SignalingHost and its original Peer worker stay unchanged. This
// wrapper gates only the original answer observation at the existing Host seam.
struct QuarantinedAnswerHost {
    inner: Arc<SignalingHost>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
struct QuarantinedAnswerAttempt {
    inner: Arc<dyn connect_service::canonical_webrtc::Attempt>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
impl Host for QuarantinedAnswerHost {
    fn adapter(&self) -> Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter> {
        self.inner.adapter()
    }
    fn reserve_physical(
        &self,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: tokio_util::sync::CancellationToken,
    ) -> connect_protocol::Result<Arc<rvoip_webrtc::canonical::WebRtcPhysicalBinding>> {
        self.inner.reserve_physical(peer, physical)
    }
    fn start(
        &self,
        preparation: connect_service::canonical_webrtc::Preparation,
    ) -> Arc<dyn connect_service::canonical_webrtc::Attempt> {
        Arc::new(QuarantinedAnswerAttempt {
            inner: self.inner.start(preparation),
            entered: self.entered.clone(),
            release: self.release.clone(),
        })
    }
}
#[async_trait::async_trait]
impl connect_service::canonical_webrtc::Attempt for QuarantinedAnswerAttempt {
    fn original_attempt(&self) -> &connect_store::NativeOfferLease {
        self.inner.original_attempt()
    }
    fn release_invocation(&self) {
        self.inner.release_invocation();
    }
    async fn answer(
        &self,
        deadline: tokio::time::Instant,
    ) -> connect_protocol::Result<rvoip_uctp::v2::payloads::ConnectionAnswer> {
        self.entered.notify_one();
        tokio::time::timeout_at(deadline, self.release.notified())
            .await
            .map_err(|_| {
                connect_protocol::Error::new("timeout", "Original answer barrier deadline")
            })?;
        self.inner.answer(deadline).await
    }
    async fn shutdown(&self) -> connect_protocol::Result<()> {
        self.inner.shutdown().await
    }
    async fn closed(&self) -> connect_protocol::Result<()> {
        self.inner.closed().await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_wss_quarantined_sdp_answer_progresses_under_real_domain_writer() {
    use rvoip_core::adapter::ConnectionAdapter;
    for algorithm in ["EdDSA", "ES256"] {
        let f = configured(algorithm, 8, None).await;
        let host = SignalingHost::new(audio_config(), 1).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        f.server
            .service
            .install_canonical_webrtc(Arc::new(QuarantinedAnswerHost {
                inner: host.clone(),
                entered: entered.clone(),
                release: release.clone(),
            }))
            .unwrap();
        provision(&f).await;
        let cid = solo(&f).await;
        let caller = RvoipPeerConnection::new(&audio_config(), PeerRole::Offerer)
            .await
            .unwrap();
        let sdp = caller.create_offer_and_gather().await.unwrap();
        let mut ws = Peer::open(&f).await;
        let original = offer(&cid, &sdp);
        let id = ws.send(original.clone(), true).await;
        let mut publisher = None;
        // No assertion/panic on the exercised negative paths: preserve the
        // failure, then release and join every original owner below.
        let outcome: std::result::Result<(), String> = async {
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .map_err(|_| "original SDP answer barrier deadline".to_owned())?;
            let writer = f.server.store.try_acquire_authority(&[
                connect_store::AuthorityScope::identity(
                    &f.client.identity_id,
                    connect_store::AuthorityMode::Write,
                )
                .map_err(|e| format!("identity scope: {e:?}"))?,
                connect_store::AuthorityScope::conversation(
                    &cid,
                    connect_store::AuthorityMode::Write,
                )
                .map_err(|e| format!("conversation scope: {e:?}"))?,
            ]);
            let released_domain = writer.is_ok();
            // Keep the real Write through the original backend SDP answer AND
            // signed WSS response. A deferred reader retry cannot satisfy this.
            release.notify_one();
            let answer = tokio::time::timeout(Duration::from_secs(5), async {
                let text = loop {
                    match ws.socket.next().await {
                        Some(Ok(Message::Text(text))) => break text,
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                        other => return Err(format!("original WSS answer frame: {other:?}")),
                    }
                };
                let answer =
                    Envelope::parse(text.as_bytes()).map_err(|e| format!("answer parse: {e:?}"))?;
                ws.verifier
                    .verify_with_context(answer.original(), &ws.context)
                    .await
                    .map_err(|e| format!("answer signature: {e:?}"))?;
                Ok(answer)
            })
            .await
            .map_err(|_| "original signed answer deadline".to_owned())??;
            let exact_answer = answer.kind() == "connection.answer"
                && answer.original()["in_reply_to"] == id
                && ["cid", "sid", "connid", "operation_id"]
                    .iter()
                    .all(|key| answer.original()[*key] == original[*key]);
            // Final current activation must acquire its own fresh Write. This
            // test writer must be gone BEFORE DTLS/first-primary can activate.
            drop(writer);
            if !exact_answer {
                return Err(format!("wrong resource answer: {}", answer.kind()));
            }
            let sdp = answer.original()["payload"]["substrate_setup"]["sdp"]
                .as_str()
                .ok_or_else(|| "resource answer missing SDP".to_owned())?;
            caller
                .set_remote_answer(sdp)
                .await
                .map_err(|e| format!("remote SDP: {e:?}"))?;
            caller
                .wait_connected(Duration::from_secs(5))
                .await
                .map_err(|e| format!("DTLS: {e:?}"))?;
            let session = f
                .server
                .service
                .canonical_session_metadata_for_test("wss-webrtc-session")
                .map_err(|e| format!("original Session missing: {e:?}"))?;
            let credential = f
                .server
                .store
                .authenticate(&f.client.credential)
                .map_err(|e| format!("current credential: {e:?}"))?
                .credential_id;
            let record = || {
                f.server
                    .store
                    .execute_operation_action(
                        &credential,
                        OperationAction::Get {
                            operation_id: "wss-webrtc-offer".into(),
                        },
                    )
                    .map_err(|e| format!("original operation: {e:?}"))
            };
            if record()?.state != OperationState::Preparing
                || !session
                    .read()
                    .map_err(|e| format!("Core read: {e:?}"))?
                    .connections
                    .is_empty()
            {
                return Err("DTLS alone published live media before first primary".into());
            }
            publisher = Some(admission::publish(&caller));
            let completed = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let value = record()?;
                    match value.state {
                        OperationState::Completed => return Ok::<_, String>(value),
                        OperationState::Failed
                        | OperationState::Cancelled
                        | OperationState::Unknown => {
                            return Err(format!("original activation refused: {:?}", value.state))
                        }
                        _ => tokio::time::sleep(Duration::from_millis(10)).await,
                    }
                }
            })
            .await
            .map_err(|_| "original first-primary activation deadline".to_owned())??;
            let core_id = {
                let state = session
                    .read()
                    .map_err(|e| format!("activated Core read: {e:?}"))?;
                if state.connections.len() != 1 || state.participants.len() != 1 {
                    return Err("original activation did not publish exactly one owner".into());
                }
                state
                    .connections
                    .keys()
                    .next()
                    .cloned()
                    .ok_or_else(|| "Core owner absent".to_owned())?
            };
            let controller = host
                .adapter()
                .staged_controller(&core_id)
                .ok_or_else(|| "original controller missing".to_owned())?;
            let bindings = controller.active_bindings();
            if completed.effects.len() != 1
                || completed.connection_id.as_deref() != Some("wss-webrtc-connection")
                || completed.result.as_ref().map(|v| &v["streams"])
                    != Some(&json!(["original-audio"]))
                || host.adapter().total_allocations() != 1
                || controller.revision() != 1
                || bindings.len() != 1
                || !bindings[0].source_ready
                || !bindings[0].sink_ready
                || bindings[0].negotiated.requested.logical_id.as_str() != "original-audio"
            {
                return Err(
                    "original authorized activation/readiness differed from its exact plan".into(),
                );
            }
            if !released_domain {
                return Err(
                    "actual signed RTC SDP preparation retained an original domain reader".into(),
                );
            }
            Ok(())
        }
        .await;
        // Also release on barrier timeout; never leave the real worker parked.
        release.notify_one();
        let publisher_joined = if let Some((stop, send)) = publisher {
            stop.cancel();
            send.await.map(|sent| sent > 0).unwrap_or(false)
        } else {
            true
        };
        let wss_closed = ws.socket.close(None).await;
        drop(ws);
        let caller_closed = caller.close().await;
        let retired = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if host.available_slots() == 1 && host.adapter().route_count() == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // Shutdown joins the existing server/Core/RTC workers even when the
        // observation above failed; do not assert before this actual owner join.
        let server_closed = f.server.shutdown().await;
        assert!(publisher_joined && wss_closed.is_ok() && caller_closed.is_ok()
            && retired.is_ok() && server_closed.is_ok(),
            "{algorithm}: cleanup failed; original outcome={outcome:?}, WSS={wss_closed:?}, caller={caller_closed:?}, retirement={retired:?}, server={server_closed:?}");
        assert!(outcome.is_ok(), "{algorithm}: {outcome:?}");
    }
}
