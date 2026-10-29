//! Independent bounded signed peers exercise the real accepting WT/WSS drivers.
use super::*;
use futures_util::{SinkExt, StreamExt};
use rvoip_auth_core::sig9421::{
    ResolvedVerificationKey, Sig9421Signer, Sig9421Verifier, SignatureVerificationContext,
    StaticKeyResolver,
};
use rvoip_uctp::v2::Envelope;
use std::sync::Arc;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    WebSocketStream,
};

type Socket = WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;
enum Wire {
    Wss(Box<Socket>),
    Wt {
        endpoint: quinn::Endpoint,
        connection: quinn::Connection,
        session: web_transport_quinn::Session,
        writer: web_transport_quinn::SendStream,
        reader: web_transport_quinn::RecvStream,
        buffered: Vec<u8>,
    },
}
fn roots(h: &Hosted) -> rustls::RootCertStore {
    let pem = std::fs::read_to_string(&h.f.client.trust_certificate).unwrap();
    let encoded = pem
        .lines()
        .filter(|v| !v.starts_with("-----"))
        .collect::<String>();
    let der = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(der))
        .unwrap();
    roots
}
impl Wire {
    async fn open(h: &Hosted, transport: EnrollmentTransport, origin: &str) -> Self {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots(h))
            .with_no_client_auth();
        match transport {
            EnrollmentTransport::WssWebrtc => {
                let tcp = tokio::net::TcpStream::connect(h.f.server.websocket_address.unwrap())
                    .await
                    .unwrap();
                let tls = tokio_rustls::TlsConnector::from(Arc::new(tls))
                    .connect(
                        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                        tcp,
                    )
                    .await
                    .unwrap();
                let mut request =
                    h.f.server
                        .websocket_url
                        .as_ref()
                        .unwrap()
                        .into_client_request()
                        .unwrap();
                request
                    .headers_mut()
                    .insert("Origin", origin.parse().unwrap());
                let (socket, _) = tokio_tungstenite::client_async(request, tls).await.unwrap();
                Self::Wss(Box::new(socket))
            }
            EnrollmentTransport::Webtransport => {
                let mut tls = tls;
                tls.alpn_protocols = vec![b"h3".to_vec()];
                let mut config = quinn::ClientConfig::new(Arc::new(
                    quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
                ));
                let mut limits = quinn::TransportConfig::default();
                limits.max_concurrent_bidi_streams(8u32.into());
                limits.max_concurrent_uni_streams(8u32.into());
                limits.datagram_receive_buffer_size(Some(64 * 1024));
                limits.datagram_send_buffer_size(64 * 1024);
                config.transport_config(Arc::new(limits));
                let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
                endpoint.set_default_client_config(config);
                let connection = endpoint
                    .connect(h.f.server.address, "localhost")
                    .unwrap()
                    .await
                    .unwrap();
                let mut request = web_transport_quinn::proto::ConnectRequest::from(
                    url::Url::parse(&format!(
                        "https://localhost:{}/connect",
                        h.f.server.address.port()
                    ))
                    .unwrap(),
                );
                request.headers.append("Origin", origin.parse().unwrap());
                let session = web_transport_quinn::Session::connect(connection.clone(), request)
                    .await
                    .unwrap();
                let (writer, reader) = session.open_bi().await.unwrap();
                Self::Wt {
                    endpoint,
                    connection,
                    session,
                    writer,
                    reader,
                    buffered: vec![],
                }
            }
            _ => panic!("browser transport fixture"),
        }
    }
    async fn send(&mut self, value: Value) {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES);
        match self {
            Self::Wss(socket) => socket
                .send(Message::Text(String::from_utf8(bytes).unwrap().into()))
                .await
                .unwrap(),
            Self::Wt { writer, .. } => {
                let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
                frame.extend(bytes);
                writer.write_all(&frame).await.unwrap();
            }
        }
    }
    async fn read(&mut self) -> Option<Envelope> {
        match self {
            Self::Wss(socket) => {
                for _ in 0..16 {
                    match socket.next().await? {
                        Ok(Message::Text(text)) => return Envelope::parse(text.as_bytes()).ok(),
                        Ok(Message::Ping(_) | Message::Pong(_)) => {}
                        _ => return None,
                    }
                }
                None
            }
            Self::Wt {
                reader, buffered, ..
            } => loop {
                if buffered.len() >= 4 {
                    let size = u32::from_be_bytes(buffered[..4].try_into().unwrap()) as usize;
                    assert!(size > 0 && size <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES);
                    if buffered.len() >= size + 4 {
                        let bytes = buffered[4..size + 4].to_vec();
                        buffered.drain(..size + 4);
                        return Envelope::parse(&bytes).ok();
                    }
                }
                let mut chunk = [0u8; 65_536];
                let size = reader.read(&mut chunk).await.ok()??;
                buffered.extend_from_slice(&chunk[..size]);
                assert!(buffered.len() <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES + 65_540);
            },
        }
    }
    async fn close(self) {
        match self {
            Self::Wss(mut socket) => {
                let _ = socket.close().await;
            }
            Self::Wt {
                endpoint,
                connection,
                session,
                writer,
                reader,
                ..
            } => {
                connection.close(0u32.into(), b"enrollment fixture complete");
                drop((session, writer, reader));
                tokio::time::timeout(Duration::from_secs(2), endpoint.wait_idle())
                    .await
                    .unwrap();
            }
        }
    }
}
fn envelope(kind: &str, payload: Value) -> Value {
    json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":payload})
}
async fn accepted(
    h: &Hosted,
    config: &CanonicalClientConfig,
    transport: EnrollmentTransport,
    origin: &str,
) -> bool {
    let mut wire = tokio::time::timeout(Duration::from_secs(5), Wire::open(h, transport, origin))
        .await
        .unwrap();
    let secret = std::fs::read(&config.signing_private_key).unwrap();
    let signer = if config.signing_algorithm == "ES256" {
        Sig9421Signer::es256(&config.signing_key_id, &secret)
    } else {
        Sig9421Signer::ed25519(&config.signing_key_id, &secret)
    }
    .unwrap();
    let context =
        SignatureVerificationContext::new("connect-server", config.server_key_id.clone()).unwrap();
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
    let verifier = Sig9421Verifier::new(Arc::new(resolver));
    let hello = envelope(
        "auth.hello",
        json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":"enrollment-origin-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]}),
    );
    tokio::time::timeout(Duration::from_secs(5), wire.send(hello))
        .await
        .unwrap();
    let challenge = tokio::time::timeout(Duration::from_secs(5), wire.read())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(challenge.kind(), "auth.challenge");
    verifier
        .verify_with_context(challenge.original(), &context)
        .await
        .unwrap();
    let mut response = envelope(
        "auth.response",
        json!({"method":"bearer","credential":config.credential}),
    );
    response["in_reply_to"] = json!(challenge.id());
    tokio::time::timeout(
        Duration::from_secs(5),
        wire.send(signer.sign(&response).unwrap()),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), wire.read())
        .await
        .unwrap();
    let accepted = if let Some(frame) = result {
        verifier
            .verify_with_context(frame.original(), &context)
            .await
            .unwrap();
        frame.kind() == "auth.session"
            && frame.original()["payload"]["identity_id"] == config.identity_id
    } else {
        false
    };
    tokio::time::timeout(Duration::from_secs(3), wire.close())
        .await
        .unwrap();
    accepted
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrolled_wt_wss_admission_checks_the_actual_transport_and_accepted_origin() {
    let h = Hosted::new().await;
    for (a, algorithm) in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256]
        .into_iter()
        .enumerate()
    {
        let key = Key::new(algorithm);
        for (t, approved) in [
            EnrollmentTransport::Webtransport,
            EnrollmentTransport::WssWebrtc,
        ]
        .into_iter()
        .enumerate()
        {
            let request = request(&h, &key, algorithm, approved, &format!("physical-{a}-{t}"));
            let offer = h.approve(&request).await;
            let issued = h
                .claim(&key.claim(&offer), request.browser_origin.as_deref())
                .await
                .json::<EnrollmentResponse<EnrollmentCredential>>()
                .await
                .unwrap()
                .result
                .unwrap();
            let config = native_config(&h, &request, &issued);
            for actual in [
                EnrollmentTransport::Webtransport,
                EnrollmentTransport::WssWebrtc,
            ] {
                for origin in [
                    "https://ui.fixture.invalid",
                    "https://other.fixture.invalid",
                ] {
                    assert_eq!(
                        accepted(&h, &config, actual, origin).await,
                        actual == approved && origin == "https://ui.fixture.invalid"
                    );
                }
            }
        }
    }
    h.f.server.shutdown().await.unwrap();
}
