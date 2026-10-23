//! Actual signed native capability IO with bounded QUIC receive credit.
use super::*;
use rvoip_auth_core::sig9421::{
    ResolvedVerificationKey, Sig9421Signer, Sig9421Verifier, SignatureVerificationContext,
    StaticKeyResolver,
};
use rvoip_uctp::v2::{Envelope, MAX_ENVELOPE_BYTES, RAW_QUIC_ALPN};
use std::sync::Arc;

const WINDOW: u32 = 4096;
pub(super) struct NativePeer {
    _endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    writer: quinn::SendStream,
    reader: quinn::RecvStream,
    signer: Sig9421Signer,
    verifier: Sig9421Verifier,
    verification: SignatureVerificationContext,
    expires_at: i64,
    session_token: String,
}
impl NativePeer {
    async fn control(&mut self) -> Envelope {
        for _ in 0..128 {
            let frame = self.frame(None).await;
            if !matches!(
                frame.kind(),
                "connection.ready" | "stream.opened" | "stream.closed" | "connection.quality"
            ) {
                return frame;
            }
        }
        panic!("missing control behind lifecycle hints");
    }
    pub(super) async fn barrier(&mut self) {
        let request = envelope(
            "auth.keepalive",
            json!({"session_token":self.session_token}),
        );
        let id = request["id"].clone();
        self.send(request, true).await;
        let frame = self.control().await;
        assert_eq!(frame.kind(), "ack");
        assert_eq!(frame.original()["in_reply_to"], id);
    }
    pub(super) async fn native(&mut self, cid: &str) {
        let capabilities = json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
        let scoped = |mut frame: Value| {
            frame["cid"] = json!(cid);
            frame["sid"] = json!("native-session");
            frame["connid"] = json!("capability-slow");
            frame["operation_id"] = json!("capability-slow-offer");
            frame
        };
        let mut offer = scoped(envelope(
            "connection.offer",
            json!({"substrate":"quic","capabilities":capabilities,"streams_offered":[{"id":"capability-slow-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]}),
        ));
        offer["expected_revision"] = json!(0);
        self.send(offer, true).await;
        let admitted = self.control().await;
        assert_eq!(
            admitted.kind(),
            "connection.answer",
            "{:?}",
            admitted.original()
        );
        let request = self.control().await;
        assert_eq!(request.kind(), "connection.offer");
        for field in ["cid", "sid", "connid", "operation_id"] {
            assert_eq!(
                request.original()[field],
                scoped(envelope("auth.keepalive", json!({})))[field]
            );
        }
        let p = &request.original()["payload"];
        let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
        let mut answer = scoped(envelope(
            "connection.answer",
            json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities,"streams_answered":streams,"substrate_setup":p["substrate_setup"]}),
        ));
        answer["in_reply_to"] = json!(request.id());
        self.send(answer, true).await;
        self.barrier().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_capability_partial_quic_write_keeps_both_authorities_until_sink_retirement(
) {
    partial_case(Mode::Deadline).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_capability_healthy_partial_quic_write_finishes_before_latest_descriptor_and_keeps_peer_usable(
) {
    partial_case(Mode::Complete).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_capability_recipient_revoke_waits_for_actual_partial_writer_and_preserves_other_physical_peers(
) {
    partial_case(Mode::RecipientRevoke).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_capability_source_revoke_waits_for_actual_partial_writer_and_blocks_further_advertisements(
) {
    partial_case(Mode::SourceRevoke).await;
}
enum Mode {
    Complete,
    Deadline,
    RecipientRevoke,
    SourceRevoke,
}
async fn partial_case(mode: Mode) {
    let complete = matches!(mode, Mode::Complete);
    let revoke_recipient = matches!(mode, Mode::RecipientRevoke);
    let revoke_source = matches!(mode, Mode::SourceRevoke);
    use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        let (mut scene, ()) =
            codecs::setup_started(f, (), "codec-target-offer", "codec-source-offer").await;
        let config = party_with_capabilities(
            &scene.f,
            &scene.f.peer.identity_id,
            104,
            true,
            vec!["audio".into()],
        );
        let mut peer = NativePeer::open(&config).await;
        peer.native(&scene.cid).await;
        wait_native_operation(
            &scene.f,
            &config.credential,
            &scene.cid,
            "capability-slow-offer",
        )
        .await;
        // Complete all original control bytes before deliberately stopping reads.
        peer.barrier().await;
        let mut descriptor = json!({"audio_codecs":[{"name":"pcmu","params":{}}],"future":{}});
        for i in 0..8 {
            descriptor["future"][format!("bounded_{i}")] = json!("x".repeat(1500));
        }
        let pid = super::capabilities::participant(&scene.f, "codec-target");
        assert_eq!(
            scene.clients[1]
                .advertise_native_capabilities(
                    super::capabilities::scope(&scene.cid, "blocked-descriptor", 2),
                    &pid,
                    descriptor.clone(),
                    "endpoint-change",
                )
                .await
                .unwrap()
                .state,
            rvoip_uctp::v2::payloads::OperationState::Completed
        );
        let length = peer.prefix().await;
        assert!(
            length > WINDOW as usize * 2,
            "expected actual large descriptor frame: {length}"
        );
        let latest = json!({"audio_codecs":[],"future":{"latest_after_blocked":true}});
        if complete {
            assert_eq!(
                scene.clients[1]
                    .advertise_native_capabilities(
                        super::capabilities::scope(&scene.cid, "blocked-latest-descriptor", 2),
                        &pid,
                        latest.clone(),
                        "network-adapt",
                    )
                    .await
                    .unwrap()
                    .state,
                rvoip_uctp::v2::payloads::OperationState::Completed
            );
        }
        let scopes = [&scene.f.client.identity_id, &config.identity_id]
            .into_iter()
            .map(|id| AuthorityScope::identity(id, AuthorityMode::Write).unwrap())
            .collect::<Vec<_>>();
        for scope in &scopes {
            assert!(
                matches!(scene.f.server.store.try_acquire_authority(std::slice::from_ref(scope)),Err(e) if e.code=="authority-busy"),
                "partial reliable write lost source or recipient authority custody"
            );
        }
        let store = scene.f.server.store.clone();
        let admin = store.authenticate(&scene.f.admin).unwrap();
        let credential = store
            .authenticate(if revoke_source {
                &scene.f.client.credential
            } else {
                &config.credential
            })
            .unwrap()
            .credential_id;
        let retiring_credential = credential.clone();
        let writing = tokio::task::spawn_blocking(move || {
            if revoke_recipient || revoke_source {
                store.revoke(&admin, &retiring_credential).unwrap();
            } else {
                let held = store
                    .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(5)))
                    .unwrap();
                drop(held);
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !writing.is_finished(),
            "authority writer overtook unfinished real IO"
        );
        if complete {
            let frame = peer.frame(Some(length)).await;
            assert_eq!(frame.kind(), "capability.advertise");
            assert_eq!(frame.original()["connid"], "codec-target");
            assert_eq!(frame.original()["payload"]["capabilities"], descriptor);
        }
        tokio::time::timeout(Duration::from_secs(3), writing)
            .await
            .unwrap()
            .unwrap();
        if revoke_recipient || revoke_source {
            assert!(scene.f.server.store.revalidate(&credential).is_err());
        }
        if complete {
            let frame = peer.control().await;
            assert_eq!(frame.kind(), "capability.advertise");
            assert_eq!(frame.original()["payload"]["capabilities"], latest);
            peer.barrier().await;
        } else {
            let mut body = vec![0; length];
            assert!(
                matches!(
                    tokio::time::timeout(Duration::from_secs(3), peer.reader.read_exact(&mut body))
                        .await,
                    Ok(Err(_))
                ),
                "abandoned partial frame remained readable after custody release"
            );
        }
        drop(peer);
        if revoke_source {
            // The normal recipient may have finished the old hint before
            // revocation queued, or its fresh admission may be fenced out.
            // A completed declaration does not guarantee remote delivery.
            let id = scene.clients[0].keepalive().await.unwrap();
            let mut prior = 0;
            loop {
                let frame = scene.clients[0].receive().await.unwrap();
                match frame.kind() {
                    "ack" => {
                        assert_eq!(frame.original()["in_reply_to"], id);
                        break;
                    }
                    "capability.advertise" => {
                        prior += 1;
                        assert!(prior <= 1);
                        assert_eq!(frame.original()["connid"], "codec-target");
                        assert_eq!(frame.original()["payload"]["capabilities"], descriptor);
                        lifecycle::observe(algorithm, &frame);
                    }
                    "connection.ready" | "stream.opened" | "stream.closed"
                    | "connection.quality" => {}
                    _ => panic!(
                        "unexpected frame after source revocation: {:?}",
                        frame.original()
                    ),
                }
            }
            assert!(scene.clients[1]
                .advertise_native_capabilities(
                    super::capabilities::scope(&scene.cid, "post-source-revoke", 2),
                    &pid,
                    json!({"future":{"forbidden_after_revoke":true}}),
                    "endpoint-change",
                )
                .await
                .is_err());
            for _ in 0..3 {
                super::capabilities::barrier_without_advertisement(&mut scene.clients[0]).await;
            }
        } else {
            // This separate original peer remains current when only the slow
            // physical destination retires, and receives its complete hint.
            loop {
                let frame =
                    super::capabilities::next_advertisement(&mut scene.clients[0], algorithm).await;
                assert_eq!(frame.original()["connid"], "codec-target");
                if !complete || frame.original()["payload"]["capabilities"] == latest {
                    break;
                }
                assert_eq!(frame.original()["payload"]["capabilities"], descriptor);
            }
            assert_audio_reaches_other_participant(
                &mut scene.clients,
                scene.sh,
                &[scene.th],
                93000,
            )
            .await;
            scene.clients.swap(0, 1);
            assert_audio_reaches_other_participant(
                &mut scene.clients,
                scene.th,
                &[scene.sh],
                94000,
            )
            .await;
        }
        for client in scene.clients {
            client.shutdown().await.unwrap();
        }
        scene.f.server.shutdown().await.unwrap();
    }
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
impl Drop for NativePeer {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"capability fixture complete");
    }
}
impl NativePeer {
    pub(super) async fn open(config: &CanonicalClientConfig) -> Self {
        Self::with_receive_window(config, WINDOW).await
    }
    pub(super) async fn with_receive_window(config: &CanonicalClientConfig, window: u32) -> Self {
        config.validate().unwrap();
        let _ = rustls::crypto::ring::default_provider().install_default();
        let secret = std::fs::read(&config.signing_private_key).unwrap();
        let signer = match config.signing_algorithm.as_str() {
            "EdDSA" => Sig9421Signer::ed25519(&config.signing_key_id, &secret),
            "ES256" => Sig9421Signer::es256(&config.signing_key_id, &secret),
            _ => panic!("fixture signing algorithm"),
        }
        .unwrap();
        let verification =
            SignatureVerificationContext::new("connect-server", config.server_key_id.clone())
                .unwrap();
        let mut keys = StaticKeyResolver::new();
        keys.insert_contextual(
            &verification,
            &config.server_key_id,
            ResolvedVerificationKey {
                public_key: std::fs::read(&config.server_public_key).unwrap(),
                algorithm: config.server_algorithm.clone(),
                not_before: Some(config.server_key_not_before),
                expires_at: Some(config.server_key_expires_at),
                revoked: false,
            },
        );
        let verifier = Sig9421Verifier::new(Arc::new(keys));
        let cert = rustls::pki_types::CertificateDer::from(
            std::fs::read(&config.trust_certificate).unwrap(),
        );
        let mut tls = rvoip_uctp::substrate::dev_client_config_trusting(&cert).unwrap();
        tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(Some(64 * 1024));
        transport.datagram_send_buffer_size(64 * 1024);
        transport.stream_receive_window(window.into());
        transport.receive_window(window.into());
        transport.max_concurrent_bidi_streams(0u32.into());
        transport.max_concurrent_uni_streams(0u32.into());
        transport.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
        let mut client = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
        ));
        client.transport_config(Arc::new(transport));
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        endpoint.set_default_client_config(client);
        let connection = tokio::time::timeout(
            Duration::from_secs(5),
            endpoint
                .connect(config.endpoint, &config.server_name)
                .unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        let (writer, reader) = connection.open_bi().await.unwrap();
        let mut peer = Self {
            _endpoint: endpoint,
            connection,
            writer,
            reader,
            signer,
            verifier,
            verification,
            expires_at: 0,
            session_token: String::new(),
        };
        let hello = envelope(
            "auth.hello",
            json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":"gate6-capability-quic-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]}),
        );
        peer.send(hello.clone(), false).await;
        let challenge = peer.frame(None).await;
        assert_eq!(challenge.kind(), "auth.challenge");
        assert_eq!(challenge.original()["in_reply_to"], hello["id"]);
        assert!(challenge.original()["payload"]["accepted_methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "bearer"));
        assert!(
            challenge.original()["payload"]["server_capabilities"]["application_profiles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "connect/2")
        );
        let mut response = envelope(
            "auth.response",
            json!({"method":"bearer","credential":config.credential}),
        );
        response["in_reply_to"] = json!(challenge.id());
        peer.send(response.clone(), true).await;
        let session = peer.frame(None).await;
        assert_eq!(session.kind(), "auth.session");
        assert_eq!(session.original()["in_reply_to"], response["id"]);
        assert_eq!(
            session.original()["payload"]["identity_id"],
            config.identity_id
        );
        assert_eq!(
            session.original()["payload"]["application_profile"],
            "connect/2"
        );
        assert_eq!(session.original()["payload"]["assurance"], "identified");
        peer.session_token = session.original()["payload"]["session_token"]
            .as_str()
            .unwrap()
            .to_owned();
        peer.expires_at = chrono::DateTime::parse_from_rfc3339(
            session.original()["payload"]["expires_at"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
        .timestamp();
        assert!(peer.expires_at > chrono::Utc::now().timestamp());
        peer
    }
    async fn send(&mut self, value: Value, signed: bool) {
        let checked = Envelope::from_value(if signed {
            self.signer.sign(&value).unwrap()
        } else {
            value
        })
        .unwrap();
        let bytes = serde_json::to_vec(checked.original()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            self.writer
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .unwrap();
            self.writer.write_all(&bytes).await.unwrap();
        })
        .await
        .unwrap();
    }
    async fn prefix(&mut self) -> usize {
        let mut size = [0; 4];
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_exact(&mut size))
            .await
            .unwrap()
            .unwrap();
        let size = u32::from_be_bytes(size) as usize;
        assert!((1..=MAX_ENVELOPE_BYTES).contains(&size));
        size
    }
    async fn frame(&mut self, size: Option<usize>) -> Envelope {
        let size = match size {
            Some(size) => size,
            None => self.prefix().await,
        };
        let mut bytes = vec![0; size];
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let frame = Envelope::parse(&bytes).unwrap();
        self.verifier
            .verify_with_context(frame.original(), &self.verification)
            .await
            .unwrap();
        frame
    }
    pub(super) async fn pause_on_dtmf(mut self) -> (Self, String) {
        // Keep unrelated hints/control flowing. Stop only after reading the
        // actual DTMF frame ID, leaving more body bytes than remaining credit.
        for _ in 0..128 {
            let size = self.prefix().await;
            let head = size.min(192);
            let mut bytes = vec![0; head];
            tokio::time::timeout(Duration::from_secs(5), self.reader.read_exact(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            let id = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| s.split_once("\"id\":\"").map(|(_, rest)| rest))
                .and_then(|s| s.split_once('"').map(|(id, _)| id.to_owned()));
            if let Some(id) = id.filter(|id| id.starts_with("dtmf_")) {
                assert!(
                    size > 256 + head + 4,
                    "DTMF must exceed all available receive credit"
                );
                return (self, id);
            }
            bytes.resize(size, 0);
            tokio::time::timeout(
                Duration::from_secs(5),
                self.reader.read_exact(&mut bytes[head..]),
            )
            .await
            .unwrap()
            .unwrap();
            let frame = Envelope::parse(&bytes).unwrap();
            self.verifier
                .verify_with_context(frame.original(), &self.verification)
                .await
                .unwrap();
            assert_ne!(
                frame.kind(),
                "dtmf.received",
                "DTMF frame ID was not in its bounded prefix"
            );
            eprintln!("drained {} before controlled partial DTMF", frame.kind());
        }
        panic!("DTMF missing behind bounded control frames")
    }
}
