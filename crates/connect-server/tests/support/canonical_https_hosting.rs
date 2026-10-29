//! Actual isolated HTTPS surfaces; no provider, model or remote account execution.
use super::*;
use base64::engine::general_purpose::STANDARD;
use connect_server::hosting::{HttpSurface, HttpsListenerConfig, TelnyxCallbacks};
use ed25519_dalek::{Signer, SigningKey};
use reqwest::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[path = "canonical_https_hosting/node.rs"]
mod node;
#[path = "canonical_https_hosting/transports.rs"]
mod transports;

struct Tls {
    issuer: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
    certificate: std::path::PathBuf,
    key: std::path::PathBuf,
    root: std::path::PathBuf,
}
impl Tls {
    fn new(f: &Fixture) -> Self {
        let mut ca = rcgen::CertificateParams::default();
        ca.distinguished_name
            .push(rcgen::DnType::CommonName, "Connect HTTPS fixture root");
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        let issuer =
            rcgen::CertifiedIssuer::self_signed(ca, rcgen::KeyPair::generate().unwrap()).unwrap();
        let root = f.temp.path().join("https-root.pem");
        std::fs::write(&root, issuer.pem()).unwrap();
        Self {
            issuer,
            certificate: f.config.certificate.clone(),
            key: f.config.private_key.clone(),
            root,
        }
    }
    fn replace(&self) -> Vec<u8> {
        let mut p =
            rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        // OpenSSL treats a leaf with the same subject as its issuer as
        // self-issued. Keep normal chain verification on every client.
        p.distinguished_name
            .push(rcgen::DnType::CommonName, "Connect HTTPS fixture server");
        let now = std::time::SystemTime::now();
        p.not_before = (now - Duration::from_secs(60)).into();
        p.not_after = (now + Duration::from_secs(3600)).into();
        p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        // Platform X.509 strict verification (including Python 3.13+) requires
        // the leaf to identify the issuing CA. Keep verification enabled.
        p.use_authority_key_identifier_extension = true;
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = p.signed_by(&key, &self.issuer).unwrap();
        std::fs::write(&self.certificate, certificate.pem()).unwrap();
        std::fs::write(&self.key, key.serialize_pem()).unwrap();
        certificate.der().to_vec()
    }
    fn http(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(0)
            .tls_info(true)
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(&self.root).unwrap()).unwrap(),
            )
            .build()
            .unwrap()
    }
}
struct Hosted {
    f: Fixture,
    tls: Tls,
    key: SigningKey,
}
fn binding(surface: HttpSurface) -> HttpsListenerConfig {
    HttpsListenerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        surface,
        public_origin: None,
        allowed_origins: if surface == HttpSurface::Application {
            vec!["https://ui.fixture.invalid".into()]
        } else {
            vec![]
        },
        trusted_proxies: vec![],
        telnyx_callbacks: None,
    }
}
impl Hosted {
    async fn new(proxies: bool, connections: usize) -> Self {
        Self::with_proxies(
            if proxies {
                vec!["127.0.0.1".parse().unwrap()]
            } else {
                vec![]
            },
            connections,
        )
        .await
    }
    async fn with_proxies(proxies: Vec<std::net::IpAddr>, connections: usize) -> Self {
        Self::configured(proxies, connections, |_| {}).await
    }
    async fn configured(
        proxies: Vec<std::net::IpAddr>,
        connections: usize,
        modify: impl FnOnce(&mut CanonicalServerConfig),
    ) -> Self {
        Self::configured_algorithm("EdDSA", proxies, connections, modify).await
    }
    async fn configured_algorithm(
        algorithm: &str,
        proxies: Vec<std::net::IpAddr>,
        connections: usize,
        modify: impl FnOnce(&mut CanonicalServerConfig),
    ) -> Self {
        let f = Fixture::new(algorithm, connections).await;
        let tls = Tls::new(&f);
        tls.replace();
        let key = SigningKey::from_bytes(&[93; 32]);
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
        config.https = [
            HttpSurface::Application,
            HttpSurface::Administration,
            HttpSurface::Callbacks,
            HttpSurface::Health,
        ]
        .into_iter()
        .map(|surface| {
            let mut v = binding(surface);
            v.trusted_proxies = proxies.clone();
            if surface == HttpSurface::Callbacks {
                v.telnyx_callbacks = Some(TelnyxCallbacks {
                    public_key: STANDARD.encode(key.verifying_key().to_bytes()),
                    routes: vec!["explicit-fixture-route".into()],
                });
            }
            v
        })
        .collect();
        modify(&mut config);
        let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
        client.endpoint = server.address;
        client.trust_certificate = tls.root.clone();
        Self {
            f: Fixture {
                temp,
                config,
                server,
                client,
                peer,
                admin,
                key_revision,
            },
            tls,
            key,
        }
    }
    fn endpoint(&self, surface: HttpSurface) -> &connect_server::hosting::HttpsEndpoint {
        self.f
            .server
            .https_endpoints
            .iter()
            .find(|e| e.surface == surface)
            .unwrap()
    }
    fn url(&self, surface: HttpSurface, path: &str) -> String {
        format!("{}{path}", self.endpoint(surface).origin)
    }
}
fn provision(operation: &str) -> ProvisioningCommand {
    ProvisioningCommand::Identity {
        operation_id: operation.into(),
        alias: operation.into(),
        kind: IdentityKind::Human,
        display_name: None,
    }
}
fn leaf(response: &reqwest::Response) -> Vec<u8> {
    response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .unwrap()
        .peer_certificate()
        .unwrap()
        .to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_https_surfaces_enforce_current_auth_origins_and_tls_only_credentials() {
    let h = Hosted::new(false, 16).await;
    let http = h.tls.http();
    let app = h.url(HttpSurface::Application, "/v2/discovery");
    assert_eq!(
        http.get(&app).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.get(&app)
            .header("Origin", "https://ui.fixture.invalid")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let reply = http
        .get(&app)
        .bearer_auth(&h.f.client.credential)
        .header("Origin", "https://ui.fixture.invalid")
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(
        reply.headers()["access-control-allow-origin"],
        "https://ui.fixture.invalid"
    );
    let _: connect_protocol::canonical_discovery::TransportDiscovery = reply.json().await.unwrap();
    for origin in ["https://foreign.fixture.invalid", "null"] {
        assert_eq!(
            http.get(&app)
                .bearer_auth(&h.f.client.credential)
                .header("Origin", origin)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let cors = http
        .request(reqwest::Method::OPTIONS, &app)
        .header("Origin", "https://ui.fixture.invalid")
        .header("Access-Control-Request-Method", "GET")
        .header(
            "Access-Control-Request-Headers",
            "authorization,content-type",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(cors.status(), StatusCode::OK);
    assert_eq!(
        cors.headers()["access-control-allow-origin"],
        "https://ui.fixture.invalid"
    );
    for surface in [
        HttpSurface::Administration,
        HttpSurface::Callbacks,
        HttpSurface::Health,
    ] {
        assert_eq!(
            http.get(h.url(surface, "/v2/discovery"))
                .bearer_auth(&h.f.admin)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let admin = h.url(HttpSurface::Administration, "/v2/identities");
    assert_eq!(
        http.post(&admin)
            .body("malformed")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.post(&admin)
            .bearer_auth(&h.f.client.credential)
            .body("malformed")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        http.post(&admin)
            .header("x-authenticated-user", "operator")
            .body("malformed")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for surface in [
        HttpSurface::Application,
        HttpSurface::Callbacks,
        HttpSurface::Health,
    ] {
        assert_eq!(
            http.post(h.url(surface, "/v2/identities"))
                .bearer_auth(&h.f.admin)
                .json(&provision("surface-must-not-create"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    for _ in 0..2 {
        let reply = http
            .post(&admin)
            .bearer_auth(&h.f.admin)
            .json(&provision("hosted-admin"))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        assert!(reply
            .json::<ProvisioningResponse>()
            .await
            .unwrap()
            .result
            .is_ok());
    }
    let database = rusqlite::Connection::open(&h.f.config.database).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM identities WHERE alias='hosted-admin'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM identities WHERE alias='surface-must-not-create'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let callback = h.url(HttpSurface::Callbacks, "/webhooks/telnyx");
    assert_eq!(
        http.post(&callback)
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let raw=serde_json::to_vec(&json!({"data":{"id":"https-signature-only","event_type":"fixture.unsupported","payload":{}}})).unwrap();
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature = h.key.sign(&[timestamp.as_bytes(), b"|", &raw].concat());
    for tamper in [false, true] {
        let reply = http
            .post(&callback)
            .header("telnyx-timestamp", &timestamp)
            .header(
                "telnyx-signature-ed25519",
                STANDARD.encode(signature.to_bytes()),
            )
            .body(if tamper { b"{}".to_vec() } else { raw.clone() })
            .send()
            .await
            .unwrap();
        assert_eq!(
            reply.status(),
            if tamper {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::NO_CONTENT
            }
        );
    }
    let health = h.url(HttpSurface::Health, "/healthz");
    let first = http.get(&health).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let before = leaf(&first);
    drop(first);
    let replacement = h.tls.replace();
    assert_ne!(before, replacement);
    assert!(h.f.server.reload_tls_certificates().await.unwrap().changed);
    // A fresh client prevents TLS ticket resumption from reporting its original
    // peer certificate. Inspect a new full handshake, not cached session identity.
    let after = h.tls.http().get(&health).send().await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    assert_eq!(leaf(&after), replacement);
    drop(after);
    std::fs::write(&h.tls.key, b"invalid replacement").unwrap();
    assert!(h.f.server.reload_tls_certificates().await.is_err());
    let retained = h.tls.http().get(&health).send().await.unwrap();
    assert_eq!(retained.status(), StatusCode::OK);
    assert_eq!(leaf(&retained), replacement);
    drop(retained);
    // Even a valid fixture operator credential cannot reach HTTP decoding on TLS.
    let endpoint = h.endpoint(HttpSurface::Administration);
    let body = serde_json::to_string(&provision("accidental-plaintext")).unwrap();
    let mut plain = tokio::net::TcpStream::connect(endpoint.address)
        .await
        .unwrap();
    plain.write_all(format!("POST /v2/identities HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",endpoint.address,h.f.admin,body.len(),body).as_bytes()).await.unwrap();
    let mut bytes = [0; 256];
    let read = tokio::time::timeout(Duration::from_secs(3), plain.read(&mut bytes))
        .await
        .unwrap();
    if let Ok(n) = read {
        assert!(!bytes[..n].starts_with(b"HTTP/1."));
    }
    drop(plain);
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM identities WHERE alias='accidental-plaintext'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    // Revoke through the production administrative route, then reauthorize a read.
    let credential =
        h.f.server
            .store
            .authenticate(&h.f.client.credential)
            .unwrap()
            .credential_id;
    let revoke = ProvisioningCommand::RevokeCredential {
        operation_id: "https-revoke".into(),
        credential_id: credential.clone(),
    };
    let reply = http
        .post(h.url(
            HttpSurface::Administration,
            &format!("/v2/credentials/{credential}/revoke"),
        ))
        .bearer_auth(&h.f.admin)
        .json(&revoke)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(
        http.get(&app)
            .bearer_auth(&h.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    drop(database);
    drop(http);
    h.f.server.shutdown().await.unwrap();
    eprintln!(
        "https-hosting-surfaces {}",
        json!({"isolated_surfaces":4,"current_auth":true,"exact_origins":true,"signature_only_callback":true,"plaintext_credentials_refused":true,"https_rotation":true,"joined":true})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_https_trusts_only_the_configured_immediate_proxy_without_identity_delegation() {
    for (proxies, trusted) in [
        (vec![], false),
        (vec!["127.0.0.1".parse().unwrap()], true),
        (vec!["203.0.113.7".parse().unwrap()], false),
    ] {
        let h = Hosted::with_proxies(proxies, 16).await;
        let http = h.tls.http();
        let app = h.url(HttpSurface::Application, "/v2/discovery");
        let authority = h.endpoint(HttpSurface::Application).address.to_string();
        let forwarded = || {
            http.get(&app)
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-host", &authority)
                .header("x-forwarded-for", "203.0.113.7")
        };
        assert_eq!(
            forwarded()
                .bearer_auth(&h.f.client.credential)
                .send()
                .await
                .unwrap()
                .status(),
            if trusted {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        assert_eq!(
            forwarded().send().await.unwrap().status(),
            if trusted {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            }
        );
        for (name, value) in [
            ("x-forwarded-proto", "http"),
            ("x-forwarded-host", "foreign.fixture.invalid"),
            ("x-forwarded-for", "203.0.113.7,203.0.113.8"),
            ("forwarded", "for=203.0.113.7;proto=https"),
            ("x-authenticated-user", "operator"),
        ] {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("x-forwarded-proto", "https".parse().unwrap());
            headers.insert("x-forwarded-host", authority.parse().unwrap());
            headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            assert_eq!(
                http.get(&app)
                    .bearer_auth(&h.f.client.credential)
                    .headers(headers)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            http.get(&app)
                .bearer_auth(&h.f.client.credential)
                .header("Host", "foreign.fixture.invalid")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        drop(http);
        h.f.server.shutdown().await.unwrap();
    }
    eprintln!(
        "https-proxy-policy {}",
        json!({"immediate_peer_checked":true,"forwarded_identity_refused":true,"forwarding_does_not_authenticate":true,"one_hop_only":true,"joined":true})
    );
}

#[tokio::test]
async fn public_https_configuration_refuses_implicit_exposure_insecure_or_ambiguous_contacts() {
    use connect_client::NativeNetworkProfile::{Local, Remote};
    let f = Fixture::new("EdDSA", 4).await;
    let mut config = f.config.clone();
    let mut valid = binding(HttpSurface::Application);
    valid.bind = "0.0.0.0:443".parse().unwrap();
    valid.public_origin = Some("https://host.fixture.invalid".into());
    assert!(valid.validate(Remote).is_ok());
    assert!(valid.validate(Local).is_err());
    for contact in [
        None,
        Some("http://host.fixture.invalid"),
        Some("https://host.fixture.invalid/"),
        Some("https://user@host.fixture.invalid"),
        Some("https://*.fixture.invalid"),
        Some("https://host.fixture.invalid:0"),
    ] {
        let mut bad = valid.clone();
        bad.public_origin = contact.map(str::to_owned);
        assert!(bad.validate(Remote).is_err());
    }
    for origins in [
        vec!["*".into()],
        vec!["http://ui.fixture.invalid".into()],
        vec!["https://ui.fixture.invalid".into(); 2],
    ] {
        let mut bad = valid.clone();
        bad.allowed_origins = origins;
        assert!(bad.validate(Remote).is_err());
    }
    let mut duplicate = binding(HttpSurface::Application);
    config.https = vec![duplicate.clone(), duplicate.clone()];
    assert!(config.validate().is_err());
    duplicate.trusted_proxies = vec!["0.0.0.0".parse().unwrap()];
    assert!(duplicate.validate(Local).is_err());
    config.https = vec![binding(HttpSurface::Callbacks)];
    assert!(config.validate().is_err());
    config.https = vec![binding(HttpSurface::Application)];
    config.https[0].public_origin = Some("https://uncovered.fixture.invalid".into());
    let Err(error) = RunningCanonicalServer::start(config).await else {
        panic!("uncovered TLS identity admitted")
    };
    assert!(
        error.to_string().contains("Configured HTTPS name"),
        "{error}"
    );
    f.server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_serve_sighup_rotates_https_and_sigterm_joins_after_invalid_reload() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let Hosted { f, tls, .. } = Hosted::new(false, 8).await;
    let mut config = f.config.clone();
    config.https = vec![binding(HttpSurface::Health)];
    let file = f.temp.path().join("serve-https.toml");
    init::private_write(&file, toml::to_string(&config).unwrap().as_bytes()).unwrap();
    f.server.shutdown().await.unwrap();
    let stderr = f.temp.path().join("serve-https-stderr.log");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .arg("serve")
        .arg("--config")
        .arg(&file)
        .env("RUST_LOG", "warn")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let address = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("owned CLI ready output");
            assert!(!line.contains(&f.admin));
            if line.starts_with("https_surface=Health ") {
                break line
                    .split("address=")
                    .nth(1)
                    .unwrap()
                    .parse::<std::net::SocketAddr>()
                    .unwrap();
            }
        }
    })
    .await
    .unwrap();
    let http = tls.http();
    let url = format!("https://{address}/healthz");
    let first = http.get(&url).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let before = leaf(&first);
    drop(first);
    let replacement = tls.replace();
    assert_ne!(before, replacement);
    let signal = |name: &str, id: u32| {
        let status = std::process::Command::new("/bin/kill")
            .args([name, &id.to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    };
    assert!(child.try_wait().unwrap().is_none());
    signal("-HUP", child.id().unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("owned CLI reload output");
            if line == "tls_reload revision=2 changed=true" {
                break;
            }
        }
    })
    .await
    .unwrap();
    let changed = tls.http().get(&url).send().await.unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(leaf(&changed), replacement);
    drop(changed);
    std::fs::write(&tls.key, b"incomplete operator replacement").unwrap();
    assert!(child.try_wait().unwrap().is_none());
    signal("-HUP", child.id().unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(child.try_wait().unwrap().is_none());
            if std::fs::read_to_string(&stderr)
                .unwrap()
                .contains("TLS reload refused")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let retained = tls.http().get(&url).send().await.unwrap();
    assert_eq!(retained.status(), StatusCode::OK);
    assert_eq!(leaf(&retained), replacement);
    drop(retained);
    drop(http);
    assert!(child.try_wait().unwrap().is_none());
    signal("-TERM", child.id().unwrap());
    assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    assert!(tokio::net::TcpListener::bind(address).await.is_ok());
    let raw = std::fs::read_to_string(&stderr).unwrap();
    assert!(!raw.contains(&f.admin));
    assert!(!raw.contains("incomplete operator replacement"));
    eprintln!(
        "https-cli-reload {}",
        json!({"sighup_changed":true,"invalid_reload_kept_last_good":true,"sigterm_exit_success":true,"port_released":true,"joined":true})
    );
}
