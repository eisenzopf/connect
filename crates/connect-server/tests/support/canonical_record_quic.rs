//! Actual signed native IO with bounded QUIC flow control, not an abandoned SDK
//! future. The parent supplies a real terminal Session and explicit 100KiB
//! privileged metadata fixture; no captured media or remote host is asserted.
use super::*;
use rvoip_auth_core::sig9421::{
    ResolvedVerificationKey, Sig9421Signer, Sig9421Verifier, SignatureVerificationContext,
    StaticKeyResolver,
};
use rvoip_uctp::v2::{Envelope, MAX_ENVELOPE_BYTES, RAW_QUIC_ALPN};
use std::sync::Arc;

const WINDOW: u32 = 4096;
struct NativePeer {
    _endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    writer: quinn::SendStream,
    reader: quinn::RecvStream,
    signer: Sig9421Signer,
    verifier: Sig9421Verifier,
    verification: SignatureVerificationContext,
    expires_at: i64,
    record_scope: Option<(String, String, String)>,
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
impl Drop for NativePeer {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"record fixture complete");
    }
}
impl NativePeer {
    async fn open(config: &CanonicalClientConfig) -> Self {
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
        transport.stream_receive_window(WINDOW.into());
        transport.receive_window(WINDOW.into());
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
            record_scope: None,
        };
        let hello = envelope(
            "auth.hello",
            json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":"gate6-record-quic-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]}),
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
    async fn raw_frame(&mut self, size: Option<usize>) -> Envelope {
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
    fn readiness(&self, frame: &Envelope) {
        let (cid, sid, uuid) = self
            .record_scope
            .as_ref()
            .expect("Record context must precede an independent hint");
        assert_eq!(frame.kind(), "recording.vcon-ready");
        assert_eq!(frame.original()["cid"], *cid);
        assert_eq!(frame.original()["sid"], *sid);
        assert_eq!(frame.original()["payload"]["vcon_handle"]["uuid"], *uuid);
        for absent in ["connid", "operation_id", "in_reply_to"] {
            assert!(frame.original().get(absent).is_none());
        }
        assert!(frame.original()["payload"].get("vcon").is_none());
    }
    async fn frame(&mut self, size: Option<usize>) -> Envelope {
        let mut size = size;
        for _ in 0..16 {
            let frame = self.raw_frame(size).await;
            if frame.kind() != "recording.vcon-ready" {
                return frame;
            }
            self.readiness(&frame);
            size = None;
        }
        panic!("unexpected readiness duplication")
    }
    async fn blocked(&mut self, r: &RecordFixture) -> (String, usize) {
        self.record_scope = Some((r.cid.clone(), r.sid.clone(), r.uuid.clone()));
        let mut request = envelope("recording.vcon-fetch", json!({"uuid":r.uuid}));
        request["cid"] = json!(r.cid);
        request["sid"] = json!(r.sid);
        let id = request["id"].as_str().unwrap().to_owned();
        self.send(request, true).await;
        let length = loop {
            let length = self.prefix().await;
            if length > 80 * 1024 {
                break length;
            }
            let hint = self.raw_frame(Some(length)).await;
            self.readiness(&hint);
        };
        // Only four prefix bytes have been consumed. Both connection and stream
        // credit are 4KiB, so a >80KiB frame cannot complete write_all. Quinn's
        // write_source applies peer max_data/max_stream_data before admission.
        (id, length)
    }
    async fn incomplete(&mut self, length: usize) {
        let mut bytes = vec![0; length];
        let result =
            tokio::time::timeout(Duration::from_secs(3), self.reader.read_exact(&mut bytes)).await;
        assert!(
            matches!(result, Ok(Err(_))),
            "expired partial Record stream was not retired"
        );
    }
}
async fn queued(r: &RecordFixture) {
    let readers = [&r.f.client.identity_id, &r.f.peer.identity_id]
        .into_iter()
        .map(|id| AuthorityScope::identity(id, AuthorityMode::Read).unwrap())
        .collect::<Vec<_>>();
    let until = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match r.f.server.store.try_acquire_authority(&readers) {
            Err(e) if e.code == "authority-busy" => return,
            Ok(held) => drop(held),
            Err(e) => panic!("native writer admission {}", e.code),
        }
        assert!(
            tokio::time::Instant::now() < until,
            "native authority writer never queued"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn identity_writer(r: &RecordFixture) -> tokio::task::JoinHandle<connect_protocol::Result<()>> {
    let store = r.f.server.store.clone();
    let identity = r.f.client.identity_id.clone();
    tokio::task::spawn_blocking(move || {
        let held = store.acquire_authority(
            &[AuthorityScope::identity(&identity, AuthorityMode::Write)?],
            &AuthorityWait::new(Duration::from_secs(8)),
        )?;
        drop(held);
        Ok(())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_quic_record_backpressure_retains_credential_and_exact_record_until_frame_finishes()
{
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, true).await;
        let mut client = http(&r.f, &r.f.client.credential).await;
        let expected = client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        let mut peer = NativePeer::open(&r.f.client).await;
        let (id, length) = peer.blocked(&r).await;
        let store = r.f.server.store.clone();
        let admin = store.authenticate(&r.f.admin).unwrap();
        let credential = store
            .authenticate(&r.f.client.credential)
            .unwrap()
            .credential_id;
        let revoking = tokio::task::spawn_blocking(move || store.revoke(&admin, &credential));
        let store = r.f.server.store.clone();
        let scope = r.scope();
        let handle = r.handle();
        let deleting = tokio::task::spawn_blocking(move || {
            store.mutate_record(
                &scope,
                "delete-after-quic-write",
                RecordMutationKind::Delete,
                "quic-delete-v1",
                &handle,
                |_| Ok(None),
            )
        });
        queued(&r).await;
        assert!(
            !revoking.is_finished(),
            "credential custody ended before native write"
        );
        assert!(
            !deleting.is_finished(),
            "Record custody ended before native write"
        );
        let frame = peer.frame(Some(length)).await;
        assert_eq!(frame.kind(), "recording.vcon-fetched");
        assert_eq!(frame.original()["in_reply_to"], id);
        assert_eq!(frame.original()["payload"]["delivery"], "inline");
        assert_eq!(frame.original()["payload"]["vcon"], expected.body);
        assert_eq!(
            frame.original()["payload"]["content_hash"],
            expected.content_hash
        );
        tokio::time::timeout(Duration::from_secs(5), revoking)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(5), deleting)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_some());
        assert!(client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        drop(peer);
        r.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_quic_record_peer_and_stream_cancellation_release_partial_write_custody() {
    for reset_stream in [true, false] {
        let r = RecordFixture::new("EdDSA", true).await;
        let mut peer = NativePeer::open(&r.f.client).await;
        peer.blocked(&r).await;
        let writing = identity_writer(&r);
        queued(&r).await;
        assert!(!writing.is_finished());
        if reset_stream {
            peer.reader.stop(0u32.into()).unwrap();
        } else {
            peer.connection.close(0u32.into(), b"cancel blocked Record");
        }
        tokio::time::timeout(Duration::from_secs(5), writing)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(peer);
        let mut client = http(&r.f, &r.f.client.credential).await;
        let data = client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert!(data.body.contains("transport_fixture_padding"));
        r.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_quic_record_credential_expiry_closes_partial_write_and_releases_capacity() {
    let mut r = RecordFixture::with_capacity("EdDSA", true, 1).await;
    let original = r.f.client.credential.clone();
    let short =
        r.f.server
            .store
            .issue(
                &r.f.client.identity_id,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 4,
            )
            .unwrap();
    r.f.key_revision =
        super::super::bind(&r.f, &short.id, r.f.key_revision, "quic-expiry-key-binding");
    r.f.client.credential = short.token;
    let mut peer = NativePeer::open(&r.f.client).await;
    let (_, length) = peer.blocked(&r).await;
    let writing = identity_writer(&r);
    queued(&r).await;
    assert!(!writing.is_finished());
    tokio::time::timeout(Duration::from_secs(7), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(chrono::Utc::now().timestamp() >= peer.expires_at);
    peer.incomplete(length).await;
    drop(peer);
    let mut client = http(&r.f, &original).await;
    assert!(client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .unwrap()
        .body
        .contains("transport_fixture_padding"));
    r.f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_quic_record_source_expiry_closes_partial_write_without_revoking_identity() {
    use connect_protocol::record_retention::{Lifetime, Policy};
    let r = RecordFixture::with_retention(
        "EdDSA",
        true,
        1,
        Some(Policy {
            name: "native-eight-second-metadata".into(),
            metadata: Lifetime::After { seconds: 8 },
            ..Default::default()
        }),
    )
    .await;
    let manifest: Vec<u8> = rusqlite::Connection::open(&r.f.config.database)
        .unwrap()
        .query_row(
            "SELECT manifest FROM session_records WHERE uuid=?1",
            [&r.uuid],
            |r| r.get(0),
        )
        .unwrap();
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    // The SQL scan index rounds up to a whole second. Authority uses the exact
    // frozen source timestamp, so compare against the original manifest value.
    let document = manifest["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["target"]["kind"] == "document")
        .unwrap();
    let expires = chrono::DateTime::parse_from_rfc3339(
        document["metadata_retention"]["until"].as_str().unwrap(),
    )
    .unwrap()
    .with_timezone(&chrono::Utc);
    r.f.server
        .store
        .configure_record_retention(&Policy::default())
        .unwrap();
    let mut peer = NativePeer::open(&r.f.client).await;
    let target = expires - chrono::Duration::seconds(2);
    if let Ok(wait) = (target - chrono::Utc::now()).to_std() {
        tokio::time::sleep(wait).await;
    }
    let (_, length) = peer.blocked(&r).await;
    let writing = identity_writer(&r);
    queued(&r).await;
    assert!(!writing.is_finished());
    tokio::time::timeout(Duration::from_secs(5), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        chrono::Utc::now() >= expires,
        "source custody ended before frozen cutoff"
    );
    peer.incomplete(length).await;
    drop(peer);
    assert!(r
        .f
        .server
        .store
        .authenticate(&r.f.client.credential)
        .is_ok());
    let mut client = http(&r.f, &r.f.client.credential).await;
    client.list_conversations(Default::default()).await.unwrap();
    assert!(client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .is_err());
    r.f.server.shutdown().await.unwrap();
}
