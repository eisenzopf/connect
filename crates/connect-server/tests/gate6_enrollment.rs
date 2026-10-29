//! Isolated real HTTPS enrollment surfaces; no remote or provider execution.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::{enrollment::*, provisioning::*};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
    hosting::{HttpSurface, HttpsListenerConfig},
};
use connect_store::Scope;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[path = "support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;
#[path = "support/enrollment_transport.rs"]
mod enrollment_transport;

struct Hosted {
    f: Fixture,
    http: reqwest::Client,
}
impl Hosted {
    async fn new() -> Self {
        let mut f = Fixture::new("EdDSA", 8).await;
        f.server.shutdown().await.unwrap();
        let mut ca = rcgen::CertificateParams::default();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        ca.distinguished_name
            .push(rcgen::DnType::CommonName, "Enrollment fixture root");
        let issuer =
            rcgen::CertifiedIssuer::self_signed(ca, rcgen::KeyPair::generate().unwrap()).unwrap();
        let mut leaf =
            rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        leaf.distinguished_name
            .push(rcgen::DnType::CommonName, "Enrollment fixture leaf");
        leaf.use_authority_key_identifier_extension = true;
        leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let pair = rcgen::KeyPair::generate().unwrap();
        let cert = leaf.signed_by(&pair, &issuer).unwrap();
        std::fs::write(&f.config.certificate, cert.pem()).unwrap();
        std::fs::write(&f.config.private_key, pair.serialize_pem()).unwrap();
        f.config.https = [
            HttpSurface::Application,
            HttpSurface::Administration,
            HttpSurface::Health,
        ]
        .into_iter()
        .map(|surface| HttpsListenerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            surface,
            public_origin: None,
            allowed_origins: if surface == HttpSurface::Application {
                vec![
                    "https://ui.fixture.invalid".into(),
                    "https://other.fixture.invalid".into(),
                ]
            } else {
                vec![]
            },
            trusted_proxies: vec![],
            telnyx_callbacks: None,
        })
        .collect();
        f.config.webtransport = Some(
            connect_server::canonical_webtransport::CanonicalWebTransportConfig {
                mount_path: "/connect".into(),
                allowed_origins: vec![
                    "https://ui.fixture.invalid".into(),
                    "https://other.fixture.invalid".into(),
                ],
                advertised_url: None,
            },
        );
        f.config.websocket = Some(
            connect_server::canonical_websocket::CanonicalWebSocketConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                mount_path: "/connect".into(),
                allowed_origins: vec![
                    "https://ui.fixture.invalid".into(),
                    "https://other.fixture.invalid".into(),
                ],
                advertised_url: None,
            },
        );
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        let root = f.temp.path().join("enrollment-root.pem");
        std::fs::write(&root, issuer.pem()).unwrap();
        f.client.trust_certificate = root;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(7))
            .pool_max_idle_per_host(0)
            .add_root_certificate(reqwest::Certificate::from_pem(issuer.pem().as_bytes()).unwrap())
            .build()
            .unwrap();
        Self { f, http }
    }
    fn url(&self, surface: HttpSurface, path: &str) -> String {
        format!(
            "{}{path}",
            self.f
                .server
                .https_endpoints
                .iter()
                .find(|e| e.surface == surface)
                .unwrap()
                .origin
        )
    }
    async fn approve(&self, request: &EnrollmentRequest) -> EnrollmentOffer {
        let r = self
            .http
            .post(self.url(HttpSurface::Administration, "/v2/enrollments"))
            .bearer_auth(&self.f.admin)
            .json(request)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["cache-control"], "no-store");
        assert_eq!(r.headers()["connection"], "close");
        let r: EnrollmentResponse<EnrollmentOffer> = r.json().await.unwrap();
        assert_eq!(r.profile, PROFILE);
        r.result.unwrap()
    }
    async fn claim(&self, claim: &EnrollmentClaim, origin: Option<&str>) -> reqwest::Response {
        let r = self
            .http
            .post(self.url(
                HttpSurface::Application,
                &format!("/v2/enrollments/{}/claim", claim.enrollment_id),
            ))
            .json(claim);
        let r = match origin {
            Some(o) => r.header("Origin", o),
            None => r,
        };
        r.send().await.unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrolled_browser_credential_cannot_open_a_native_quic_peer() {
    let h = Hosted::new().await;
    let mut control = h.f.client.clone();
    control.endpoint = h.f.server.address;
    CanonicalClient::open(control)
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
    let key = Key::new(SigningAlgorithm::EdDSA);
    let positive = request(
        &h,
        &key,
        SigningAlgorithm::EdDSA,
        EnrollmentTransport::Quic,
        "native-positive",
    );
    let offer = h.approve(&positive).await;
    let issued = h
        .claim(&key.claim(&offer), None)
        .await
        .json::<EnrollmentResponse<EnrollmentCredential>>()
        .await
        .unwrap()
        .result
        .unwrap();
    CanonicalClient::open(native_config(&h, &positive, &issued))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
    let request = request(
        &h,
        &key,
        SigningAlgorithm::EdDSA,
        EnrollmentTransport::Webtransport,
        "native-refusal",
    );
    let offer = h.approve(&request).await;
    let issued = h
        .claim(&key.claim(&offer), request.browser_origin.as_deref())
        .await
        .json::<EnrollmentResponse<EnrollmentCredential>>()
        .await
        .unwrap()
        .result
        .unwrap();
    let native = native_config(&h, &request, &issued);
    let opened = CanonicalClient::open(native).await;
    if let Ok(peer) = opened {
        peer.shutdown().await.unwrap();
        h.f.server.shutdown().await.unwrap();
        panic!("browser enrollment admitted a QUIC peer");
    } else if let Err(error) = opened {
        assert!(!matches!(error, connect_client::ClientError::Configuration));
    }
    h.f.server.shutdown().await.unwrap();
}
fn native_config(
    h: &Hosted,
    request: &EnrollmentRequest,
    issued: &EnrollmentCredential,
) -> CanonicalClientConfig {
    let mut native = h.f.client.clone();
    native.endpoint = h.f.server.address;
    native.credential = issued.credential.credential.clone();
    native.device_id = request.device.id.clone();
    native.device_kind = request.device.kind.clone();
    native.device_platform = request.device.platform.clone();
    native.signing_key_id = request.key_id.clone();
    native.signing_algorithm = request.algorithm.as_str().into();
    native.signing_private_key = h.f.temp.path().join(format!("{}.der", request.key_id));
    // Synthetic standard PKCS#8 fixtures; never an account or provider key.
    let pkcs8 = match request.algorithm {
        SigningAlgorithm::EdDSA => {
            let mut pkcs8 = vec![
                0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
                0x04, 0x20,
            ];
            pkcs8.extend_from_slice(&[42; 32]);
            pkcs8.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
            pkcs8.extend_from_slice(&SigningKey::from_bytes(&[42; 32]).verifying_key().to_bytes());
            rvoip_auth_core::sig9421::Sig9421Signer::ed25519(&native.signing_key_id, &pkcs8)
                .unwrap();
            pkcs8
        }
        SigningAlgorithm::ES256 => {
            let mut pkcs8 = vec![
                0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
                0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04,
                0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
            ];
            pkcs8.extend_from_slice(&[43; 32]);
            pkcs8.extend_from_slice(&[0xa1, 0x44, 0x03, 0x42, 0x00]);
            pkcs8.extend(URL_SAFE_NO_PAD.decode(&request.public_key).unwrap());
            rvoip_auth_core::sig9421::Sig9421Signer::es256(&native.signing_key_id, &pkcs8).unwrap();
            pkcs8
        }
    };
    std::fs::write(&native.signing_private_key, pkcs8).unwrap();
    native
}
enum Key {
    Ed(SigningKey),
    P256(p256::ecdsa::SigningKey),
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrolled_browser_http_requires_its_exact_origin_on_later_requests() {
    let h = Hosted::new().await;
    // The extra ceiling cannot replace a named route's body/authentication
    // policy for an invalid token that cannot identify any enrolled authority.
    assert_eq!(
        h.http
            .post(h.url(HttpSurface::Application, "/v1/operations"))
            .bearer_auth("invalid")
            .header("Content-Type", "application/json")
            .body("{")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let key = Key::new(SigningAlgorithm::EdDSA);
    let request = request(
        &h,
        &key,
        SigningAlgorithm::EdDSA,
        EnrollmentTransport::Webtransport,
        "http-origin",
    );
    let offer = h.approve(&request).await;
    let issued = h
        .claim(&key.claim(&offer), request.browser_origin.as_deref())
        .await
        .json::<EnrollmentResponse<EnrollmentCredential>>()
        .await
        .unwrap()
        .result
        .unwrap();
    let url = h.url(HttpSurface::Application, "/v2/discovery");
    for origin in [None, Some("https://other.fixture.invalid")] {
        let mut call = h.http.get(&url).bearer_auth(&issued.credential.credential);
        if let Some(origin) = origin {
            call = call.header("Origin", origin);
        }
        assert_eq!(call.send().await.unwrap().status(), 403);
    }
    assert_eq!(
        h.http
            .get(&url)
            .bearer_auth(&issued.credential.credential)
            .header("Origin", request.browser_origin.as_ref().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    h.f.server.shutdown().await.unwrap();
}
impl Key {
    fn new(algorithm: SigningAlgorithm) -> Self {
        match algorithm {
            SigningAlgorithm::EdDSA => Self::Ed(SigningKey::from_bytes(&[42; 32])),
            SigningAlgorithm::ES256 => {
                Self::P256(p256::ecdsa::SigningKey::from_slice(&[43; 32]).unwrap())
            }
        }
    }
    fn public(&self) -> String {
        URL_SAFE_NO_PAD.encode(match self {
            Self::Ed(k) => k.verifying_key().to_bytes().to_vec(),
            Self::P256(k) => k
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
        })
    }
    fn claim(&self, offer: &EnrollmentOffer) -> EnrollmentClaim {
        let bytes = offer.proof_message().unwrap();
        let signature = match self {
            Self::Ed(k) => k.sign(&bytes).to_bytes().to_vec(),
            Self::P256(k) => {
                let s: p256::ecdsa::Signature = k.sign(&bytes);
                s.to_bytes().to_vec()
            }
        };
        EnrollmentClaim {
            operation_id: offer.request.completion_operation_id.clone(),
            enrollment_id: offer.enrollment_id.clone(),
            identity_id: offer.request.identity_id.clone(),
            device_id: offer.request.device.id.clone(),
            endpoint_id: offer.endpoint_id.clone(),
            key_id: offer.request.key_id.clone(),
            transport: offer.request.transport,
            browser_origin: offer.request.browser_origin.clone(),
            challenge: offer.challenge.clone(),
            signature: URL_SAFE_NO_PAD.encode(signature),
        }
    }
}
fn request(
    h: &Hosted,
    key: &Key,
    algorithm: SigningAlgorithm,
    transport: EnrollmentTransport,
    id: &str,
) -> EnrollmentRequest {
    let browser = transport != EnrollmentTransport::Quic;
    EnrollmentRequest {
        operation_id: format!("approve-{id}"),
        completion_operation_id: format!("complete-{id}"),
        identity_id: h.f.client.identity_id.clone(),
        device: DeviceSpec {
            id: format!("device-{id}"),
            kind: if browser { "browser" } else { "desktop" }.into(),
            platform: "https-fixture".into(),
        },
        key_id: format!("key-{id}"),
        algorithm,
        public_key: key.public(),
        transport,
        endpoint_address: if browser {
            format!("https://device.fixture.invalid/{id}")
        } else {
            format!("quic://device.fixture.invalid:4443/{id}")
        },
        capabilities: vec!["control".into(), "data".into()],
        browser_origin: browser.then(|| "https://ui.fixture.invalid".into()),
        operations: vec!["operation.get".into()],
        recipients: vec![h.f.peer.identity_id.clone()],
        credential_expires_at: chrono::Utc::now().timestamp() + 240,
        challenge_expires_at: chrono::Utc::now().timestamp() + 120,
    }
}
fn counts(h: &Hosted) -> (u64, u64, u64, u64) {
    let c = rusqlite::Connection::open_with_flags(
        h.f.server.store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    c.query_row("SELECT (SELECT COUNT(*) FROM credentials),(SELECT COUNT(*) FROM devices),(SELECT COUNT(*) FROM endpoints),(SELECT COUNT(*) FROM peer_signing_keys)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn https_enrollment_admits_real_proofs_all_profiles_and_recovers_after_restart() {
    let mut h = Hosted::new().await;
    for (a, algorithm) in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256]
        .into_iter()
        .enumerate()
    {
        for (t, transport) in [
            EnrollmentTransport::Quic,
            EnrollmentTransport::Webtransport,
            EnrollmentTransport::WssWebrtc,
        ]
        .into_iter()
        .enumerate()
        {
            let key = Key::new(algorithm);
            let request = request(&h, &key, algorithm, transport, &format!("{a}-{t}"));
            let before = counts(&h);
            let offer = h.approve(&request).await;
            assert_eq!(counts(&h), before);
            assert_eq!(h.approve(&request).await, offer);
            let claim = key.claim(&offer);
            let r = h.claim(&claim, request.browser_origin.as_deref()).await;
            assert_eq!(r.status(), 200);
            if transport != EnrollmentTransport::Quic {
                assert_eq!(
                    r.headers()["access-control-allow-origin"],
                    "https://ui.fixture.invalid"
                );
            }
            assert_eq!(r.headers()["pragma"], "no-cache");
            let wire_issued = r.json::<Value>().await.unwrap();
            let issued = serde_json::from_value::<EnrollmentResponse<EnrollmentCredential>>(
                wire_issued.clone(),
            )
            .unwrap()
            .result
            .unwrap();
            assert_eq!(
                counts(&h),
                (before.0 + 1, before.1 + 1, before.2 + 1, before.3 + 1)
            );
            assert_eq!(issued.credential.operations, request.operations);
            assert_eq!(issued.credential.expires_at, request.credential_expires_at);
            assert!(!h
                .f
                .server
                .store
                .authenticate(&issued.credential.credential)
                .unwrap()
                .permits("admin"));
            let recovered = h
                .claim(&claim, request.browser_origin.as_deref())
                .await
                .json::<EnrollmentResponse<EnrollmentCredential>>()
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(recovered, issued);
            h.f.config.bind = h.f.server.address;
            h.f.config.http_bind = h.f.server.http_address;
            h.f.server.shutdown().await.unwrap();
            h.f.server = RunningCanonicalServer::start(h.f.config.clone())
                .await
                .unwrap();
            let recovered = h
                .claim(&claim, request.browser_origin.as_deref())
                .await
                .json::<EnrollmentResponse<EnrollmentCredential>>()
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(recovered, issued);
            let revoke_url = h.url(
                HttpSurface::Administration,
                &format!("/v2/enrollments/{}/revoke", offer.enrollment_id),
            );
            for _ in 0..2 {
                let r = h
                    .http
                    .post(&revoke_url)
                    .bearer_auth(&h.f.admin)
                    .json(&EnrollmentRevokeRequest {
                        operation_id: format!("revoke-{a}-{t}"),
                    })
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 200);
                let wire_revoke = r.json::<Value>().await.unwrap();
                assert!(
                    serde_json::from_value::<EnrollmentResponse<EnrollmentRevocation>>(
                        wire_revoke.clone()
                    )
                    .unwrap()
                    .result
                    .unwrap()
                    .revoked
                );
                if let Ok(directory) = std::env::var("CONNECT_ENROLLMENT_SAMPLES") {
                    let directory = std::path::PathBuf::from(directory);
                    assert!(directory.is_absolute());
                    std::fs::create_dir_all(&directory).unwrap();
                    let sample = json!({"request":&request,"claim":&claim,
                        "revoke":EnrollmentRevokeRequest { operation_id:format!("revoke-{a}-{t}") },
                        "offer_response":EnrollmentResponse { profile:PROFILE.into(),result:Ok(&offer) },
                        "credential_response":wire_issued,"revoke_response":wire_revoke});
                    std::fs::write(
                        directory.join(format!("scene-{a}-{t}.json")),
                        serde_json::to_vec(&sample).unwrap(),
                    )
                    .unwrap();
                }
            }
            assert_eq!(
                h.claim(&claim, request.browser_origin.as_deref())
                    .await
                    .status(),
                403
            );
            assert!(h
                .f
                .server
                .store
                .authenticate(&issued.credential.credential)
                .is_err());
        }
    }
    h.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn https_enrollment_isolates_admin_origin_route_and_proof_authority() {
    let h = Hosted::new().await;
    let key = Key::new(SigningAlgorithm::EdDSA);
    let request = request(
        &h,
        &key,
        SigningAlgorithm::EdDSA,
        EnrollmentTransport::Webtransport,
        "refusals",
    );
    let admin = h.url(HttpSurface::Administration, "/v2/enrollments");
    for (token, extra, status) in [
        (None, None, 401),
        (Some(h.f.client.credential.as_str()), None, 403),
        (
            Some(h.f.admin.as_str()),
            Some(("Origin", "https://ui.fixture.invalid")),
            403,
        ),
        (Some(h.f.admin.as_str()), Some(("X-Call-ID", "forged")), 403),
    ] {
        let mut r = h
            .http
            .post(&admin)
            .header("Content-Type", "application/json")
            .body("{");
        if let Some(token) = token {
            r = r.bearer_auth(token);
        }
        if let Some((k, v)) = extra {
            r = r.header(k, v);
        }
        assert_eq!(r.send().await.unwrap().status(), status);
    }
    assert_eq!(
        h.http
            .post(h.url(HttpSurface::Application, "/v2/enrollments"))
            .bearer_auth(&h.f.admin)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let offer = h.approve(&request).await;
    let claim = key.claim(&offer);
    let before = counts(&h);
    for origin in [
        None,
        Some("https://other.fixture.invalid"),
        Some("https://unlisted.fixture.invalid"),
    ] {
        assert_eq!(h.claim(&claim, origin).await.status(), 403);
    }
    let url = h.url(
        HttpSurface::Application,
        &format!("/v2/enrollments/{}/claim", claim.enrollment_id),
    );
    for r in [
        h.http.post(&url).bearer_auth(&h.f.admin),
        h.http.post(&url).header("X-Chat-ID", "forged"),
        h.http.post(format!("{url}?challenge=forbidden")),
    ] {
        assert_eq!(
            r.header("Origin", "https://ui.fixture.invalid")
                .json(&claim)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let mut wrong = claim.clone();
    wrong.endpoint_id = "ep_wrong".into();
    assert_eq!(
        h.claim(&wrong, request.browser_origin.as_deref())
            .await
            .status(),
        403
    );
    assert_eq!(
        h.http
            .post(h.url(
                HttpSurface::Administration,
                &format!("/v2/enrollments/{}/claim", claim.enrollment_id)
            ))
            .json(&claim)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let mut stolen = claim.clone();
    stolen.signature = URL_SAFE_NO_PAD.encode([0; 64]);
    for _ in 0..5 {
        assert_eq!(
            h.claim(&stolen, request.browser_origin.as_deref())
                .await
                .status(),
            403
        );
    }
    assert_eq!(
        h.claim(&claim, request.browser_origin.as_deref())
            .await
            .status(),
        403
    );
    assert_eq!(counts(&h), before);
    h.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrollment_bounds_body_deadline_and_recovers_a_committed_lost_response() {
    let h = Hosted::new().await;
    let key = Key::new(SigningAlgorithm::EdDSA);
    let request = request(
        &h,
        &key,
        SigningAlgorithm::EdDSA,
        EnrollmentTransport::Quic,
        "lost",
    );
    let address = h.f.server.http_address.unwrap();
    let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
    slow.write_all(format!("POST /v2/enrollments HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{{",h.f.admin).as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(7), slow.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes.starts_with(b"HTTP/1.1 503"));
    let oversized = " ".repeat(65_537);
    let r = h
        .http
        .post(h.url(HttpSurface::Administration, "/v2/enrollments"))
        .bearer_auth(&h.f.admin)
        .header("Content-Type", "application/json")
        .body(oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
    let offer = h.approve(&request).await;
    let claim = key.claim(&offer);
    let body = serde_json::to_vec(&claim).unwrap();
    // Read only the successful response header, proving commit, then lose its body.
    let mut lost = tokio::net::TcpStream::connect(address).await.unwrap();
    lost.write_all(format!("POST /v2/enrollments/{}/claim HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",offer.enrollment_id,body.len()).as_bytes()).await.unwrap();
    lost.write_all(&body).await.unwrap();
    let mut header = Vec::new();
    let mut byte = [0; 1];
    while !header.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(Duration::from_secs(7), lost.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        header.push(byte[0]);
        assert!(header.len() < 16_384);
    }
    assert!(header.starts_with(b"HTTP/1.1 200"));
    drop(lost);
    let before = counts(&h);
    let issued = h
        .claim(&claim, None)
        .await
        .json::<EnrollmentResponse<EnrollmentCredential>>()
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(counts(&h), before);
    let c = rusqlite::Connection::open_with_flags(
        h.f.server.store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let receipt: String = c
        .query_row(
            "SELECT result_json FROM operations WHERE operation_id=?1",
            [&request.completion_operation_id],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: Value = serde_json::from_str(&receipt).unwrap();
    assert_eq!(receipt["credential_id"], issued.credential.credential_id);
    assert!(receipt.get("credential").is_none());
    assert!(receipt.get("challenge").is_none());
    drop(c);
    h.f.server.shutdown().await.unwrap();
}
