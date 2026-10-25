#![recursion_limit = "256"]
//! Independent HTTP3 peer against the actual Connect canonical listener.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::provisioning::*;
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use rvoip_auth_core::sig9421::{
    ResolvedVerificationKey, Sig9421Signer, Sig9421Verifier, SignatureVerificationContext,
    StaticKeyResolver,
};
use rvoip_uctp::v2::Envelope;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
const ORIGIN: &str = "https://browser.fixture.invalid";
const MOUNT: &str = "/connect";

async fn fixture(algorithm: &str) -> Fixture {
    let f = Fixture::with_native_stream_scope(algorithm, 8).await;
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = f;
    server.shutdown().await.unwrap();
    config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: MOUNT.into(),
            allowed_origins: vec![ORIGIN.into()],
            advertised_url: None,
        },
    );
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    assert_eq!(
        server.webtransport_url.as_deref(),
        Some(format!("https://{}{MOUNT}", server.address).as_str())
    );
    Fixture {
        temp,
        config,
        server,
        client,
        peer,
        admin,
        key_revision,
    }
}

struct Http3Peer {
    endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    _session: web_transport_quinn::Session,
    writer: web_transport_quinn::SendStream,
    reader: web_transport_quinn::RecvStream,
    buffered: Vec<u8>,
    signer: Sig9421Signer,
    verifier: Sig9421Verifier,
    context: SignatureVerificationContext,
}
impl Drop for Http3Peer {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"independent HTTP3 fixture complete");
    }
}
async fn transport(
    f: &Fixture,
    path: &str,
    origins: &[&str],
) -> Result<
    (
        quinn::Endpoint,
        quinn::Connection,
        web_transport_quinn::Session,
    ),
    String,
> {
    transport_mode(f, path, origins, true).await
}
async fn transport_mode(
    f: &Fixture,
    path: &str,
    origins: &[&str],
    datagrams: bool,
) -> Result<
    (
        quinn::Endpoint,
        quinn::Connection,
        web_transport_quinn::Session,
    ),
    String,
> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(
            std::fs::read(&f.client.trust_certificate).unwrap(),
        ))
        .unwrap();
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let mut config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
    ));
    let mut limits = quinn::TransportConfig::default();
    limits.max_concurrent_bidi_streams(8u32.into());
    limits.max_concurrent_uni_streams(8u32.into());
    limits.datagram_receive_buffer_size(datagrams.then_some(64 * 1024));
    limits.datagram_send_buffer_size(64 * 1024);
    config.transport_config(Arc::new(limits));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(config);
    let connection = tokio::time::timeout(
        Duration::from_secs(5),
        endpoint.connect(f.server.address, "localhost").unwrap(),
    )
    .await
    .map_err(|_| "TLS timeout")?
    .map_err(|_| "TLS refusal")?;
    let mut request = web_transport_quinn::proto::ConnectRequest::from(
        url::Url::parse(&format!(
            "https://localhost:{}{path}",
            f.server.address.port()
        ))
        .unwrap(),
    );
    for origin in origins {
        request.headers.append("origin", origin.parse().unwrap());
    }
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        web_transport_quinn::Session::connect(connection.clone(), request),
    )
    .await;
    match result {
        Ok(Ok(session)) => Ok((endpoint, connection, session)),
        _ => {
            let remote_reason = connection
                .close_reason()
                .map(|error| error.to_string())
                .unwrap_or_default();
            connection.close(0u32.into(), b"expected HTTP3 upgrade refusal");
            let _ = tokio::time::timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
            Err(format!("HTTP3 upgrade refused: {remote_reason}"))
        }
    }
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
impl Http3Peer {
    async fn open(f: &Fixture) -> Self {
        Self::open_config(f, &f.client).await
    }
    async fn wire(f: &Fixture, config: &CanonicalClientConfig, datagrams: bool) -> Self {
        let (endpoint, connection, session) = transport_mode(f, MOUNT, &[ORIGIN], datagrams)
            .await
            .unwrap();
        let (writer, reader) = session.open_bi().await.unwrap();
        let secret = std::fs::read(&config.signing_private_key).unwrap();
        let signer = if config.signing_algorithm == "ES256" {
            Sig9421Signer::es256(&config.signing_key_id, &secret)
        } else {
            Sig9421Signer::ed25519(&config.signing_key_id, &secret)
        }
        .unwrap();
        let context =
            SignatureVerificationContext::new("connect-server", config.server_key_id.clone())
                .unwrap();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert_contextual(
            &context,
            &config.server_key_id,
            ResolvedVerificationKey {
                public_key: std::fs::read(&config.server_public_key).unwrap(),
                algorithm: config.server_algorithm.clone(),
                not_before: Some(config.server_key_not_before),
                expires_at: Some(config.server_key_expires_at),
                revoked: false,
            },
        );
        Self {
            endpoint,
            connection,
            _session: session,
            writer,
            reader,
            buffered: Vec::new(),
            signer,
            verifier: Sig9421Verifier::new(Arc::new(resolver)),
            context,
        }
    }
    async fn open_config(f: &Fixture, config: &CanonicalClientConfig) -> Self {
        Self::open_mode(f, config, true).await
    }
    async fn open_mode(f: &Fixture, config: &CanonicalClientConfig, datagrams: bool) -> Self {
        let mut peer = Self::wire(f, config, datagrams).await;
        let hello = envelope(
            "auth.hello",
            json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":"independent-http3-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]}),
        );
        let id = peer.send(hello, false).await;
        let challenge = peer.read().await;
        assert_eq!(challenge.kind(), "auth.challenge");
        assert_eq!(challenge.original()["in_reply_to"], id);
        assert!(
            challenge.original()["payload"]["server_capabilities"]["application_profiles"]
                .as_array()
                .unwrap()
                .contains(&json!("connect/2"))
        );
        let mut response = envelope(
            "auth.response",
            json!({"method":"bearer","credential":config.credential}),
        );
        response["in_reply_to"] = json!(challenge.id());
        let id = peer.send(response, true).await;
        let authenticated = peer.read().await;
        assert_eq!(authenticated.kind(), "auth.session");
        assert_eq!(authenticated.original()["in_reply_to"], id);
        assert_eq!(
            authenticated.original()["payload"]["identity_id"],
            config.identity_id
        );
        assert_eq!(
            authenticated.original()["payload"]["application_profile"],
            "connect/2"
        );
        peer
    }
    async fn send(&mut self, value: Value, signed: bool) -> String {
        let id = value["id"].as_str().unwrap().to_owned();
        let value = if signed {
            self.signer.sign(&value).unwrap()
        } else {
            value
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(!bytes.is_empty() && bytes.len() <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES);
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        tokio::time::timeout(Duration::from_secs(5), self.writer.write_all(&frame))
            .await
            .unwrap()
            .unwrap();
        id
    }
    async fn read(&mut self) -> Envelope {
        let bytes = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.buffered.len() >= 4 {
                    let size = u32::from_be_bytes(self.buffered[..4].try_into().unwrap()) as usize;
                    assert!(size > 0 && size <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES);
                    if self.buffered.len() >= size + 4 {
                        let bytes = self.buffered[4..size + 4].to_vec();
                        self.buffered.drain(..size + 4);
                        return bytes;
                    }
                }
                let mut chunk = [0; 65536];
                let size = self
                    .reader
                    .read(&mut chunk)
                    .await
                    .unwrap()
                    .expect("unexpected HTTP3 EOF");
                self.buffered.extend_from_slice(&chunk[..size]);
                assert!(self.buffered.len() <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES + 65540);
            }
        })
        .await
        .expect("bounded canonical HTTP3 frame deadline");
        let frame = Envelope::parse(&bytes).unwrap();
        self.verifier
            .verify_with_context(frame.original(), &self.context)
            .await
            .unwrap();
        frame
    }
    async fn create(&mut self, input: &connect_client::conversations::CreateRequest) -> String {
        let mut value = envelope("conversation.create", serde_json::to_value(input).unwrap());
        value["operation_id"] = json!("http3-create");
        let id = self.send(value, true).await;
        let mut cid = None;
        let mut bytes = 0;
        for _ in 0..128 {
            let frame = self.read().await;
            assert_eq!(frame.original()["in_reply_to"], id);
            bytes += serde_json::to_vec(frame.original()).unwrap().len();
            assert!(bytes <= 2 * 1024 * 1024);
            if frame.kind() == "conversation.opened" {
                cid = Some(frame.original()["cid"].as_str().unwrap().to_owned());
            }
            assert_ne!(frame.kind(), "error", "{:?}", frame.original());
            if frame.kind() == "ack" {
                let cid = cid.expect("missing durable conversation projection");
                assert_eq!(
                    frame.original()["payload"]["details"]["reply_complete"],
                    true
                );
                let value: connect_protocol::Conversation = serde_json::from_value(
                    frame.original()["payload"]["details"]["conversation"].clone(),
                )
                .unwrap();
                assert_eq!(value.id, cid);
                assert!(value
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.matches("http3-create", &cid)));
                return cid;
            }
        }
        panic!("HTTP3 reply batch exceeded its bound")
    }
    async fn close(&self) {
        self.connection
            .close(0u32.into(), b"independent HTTP3 peer closed");
        tokio::time::timeout(Duration::from_secs(3), self.endpoint.wait_idle())
            .await
            .unwrap();
    }
}
async fn control(algorithm: &str) {
    let f = fixture(algorithm).await;
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let mut first = Http3Peer::open(&f).await;
    let cid = first.create(&input).await;
    first.close().await;
    drop(first);
    let mut second = Http3Peer::open(&f).await;
    assert_eq!(second.create(&input).await, cid);
    second.close().await;
    drop(second);
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    assert_eq!(
        native
            .create_conversation("http3-create", input)
            .await
            .unwrap()
            .id,
        cid
    );
    native.shutdown().await.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE operation_id='http3-create' AND owner_id=?1",
            [&f.client.identity_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_http3_signed_control_and_reconnect_share_native_receipt_eddsa() {
    control("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_http3_signed_control_and_reconnect_share_native_receipt_es256() {
    control("ES256").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_missing_duplicate_origin_and_wrong_mount_refuse_before_authority() {
    let f = fixture("EdDSA").await;
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    for (path, origins) in [
        (MOUNT, vec![]),
        (MOUNT, vec!["https://foreign.fixture.invalid"]),
        (MOUNT, vec![ORIGIN, ORIGIN]),
        ("/wrong", vec![ORIGIN]),
    ] {
        assert!(
            transport(&f, path, &origins).await.is_err(),
            "accepted mount {path:?}, origins {origins:?}"
        );
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .physical_peers
            != 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    let snapshot = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(snapshot.connections, 0);
    assert_eq!(snapshot.media_readers, 0);
    f.server.shutdown().await.unwrap();
}

#[path = "support/canonical_webtransport_media.rs"]
mod canonical_webtransport_media;

#[cfg(unix)]
#[path = "support/canonical_webtransport_browser.rs"]
mod canonical_webtransport_browser;

#[path = "support/canonical_webtransport_discovery.rs"]
mod canonical_webtransport_discovery;

#[path = "support/canonical_webtransport_lifecycle.rs"]
mod canonical_webtransport_lifecycle;

#[path = "support/canonical_webtransport_mixed.rs"]
mod canonical_webtransport_mixed;
