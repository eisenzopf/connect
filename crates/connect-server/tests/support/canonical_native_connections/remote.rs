//! Explicit Remote profile on isolated loopback. The UDP NAT witness changes
//! both endpoint and server-visible paths. This is not distinct-host qualification.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
const NAME: &str = "native.fixture.invalid";
#[path = "remote/rotation.rs"]
mod rotation;

async fn configured(algorithm: &str, complete_chain: bool) -> Fixture {
    configured_at(algorithm, complete_chain, "127.0.0.1:0".parse().unwrap()).await
}
async fn configured_at(
    algorithm: &str,
    complete_chain: bool,
    bind: std::net::SocketAddr,
) -> Fixture {
    configured_at_with_issuer(algorithm, complete_chain, bind)
        .await
        .0
}
async fn configured_at_with_issuer(
    algorithm: &str,
    complete_chain: bool,
    bind: std::net::SocketAddr,
) -> (Fixture, rcgen::CertifiedIssuer<'static, rcgen::KeyPair>) {
    let f = Fixture::with_native_dtmf_scope(algorithm, 12).await;
    let mut root_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    root_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Connect isolated root");
    root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    root_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let root =
        rcgen::CertifiedIssuer::self_signed(root_params, rcgen::KeyPair::generate().unwrap())
            .unwrap();
    let mut intermediate_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    intermediate_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Connect isolated intermediate");
    intermediate_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    intermediate_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let intermediate = rcgen::CertifiedIssuer::signed_by(
        intermediate_params,
        rcgen::KeyPair::generate().unwrap(),
        &root,
    )
    .unwrap();
    let mut leaf_params = rcgen::CertificateParams::new(vec![NAME.into()]).unwrap();
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &intermediate).unwrap();
    let certificate = f.temp.path().join("remote-chain.pem");
    let private_key = f.temp.path().join("remote-tls-key.pem");
    let trust = f.temp.path().join("remote-root.pem");
    std::fs::write(
        &certificate,
        if complete_chain {
            format!("{}{}", leaf.pem(), intermediate.pem())
        } else {
            leaf.pem()
        },
    )
    .unwrap();
    std::fs::write(&private_key, leaf_key.serialize_pem()).unwrap();
    std::fs::write(&trust, root.pem()).unwrap();
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
    config.network_profile = connect_client::NativeNetworkProfile::Remote;
    // Keep default tests isolated; named remote/hosting profiles are separate.
    config.bind = bind;
    config.certificate = certificate;
    config.private_key = private_key;
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    client.network_profile = connect_client::NativeNetworkProfile::Remote;
    client.server_name = NAME.into();
    client.trust_certificate = trust;
    (
        Fixture {
            temp,
            config,
            server,
            client,
            peer,
            admin,
            key_revision,
        },
        intermediate,
    )
}

struct Nat {
    address: std::net::SocketAddr,
    paths: Arc<Mutex<BTreeMap<std::net::SocketAddr, std::net::SocketAddr>>>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Nat {
    async fn new(server: std::net::SocketAddr) -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address = socket.local_addr().unwrap();
        let paths = Arc::new(Mutex::new(BTreeMap::new()));
        let seen = paths.clone();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let mut mappings: BTreeMap<std::net::SocketAddr, Arc<UdpSocket>> = BTreeMap::new();
            let mut readers = tokio::task::JoinSet::new();
            let mut packet = [0; 65_535];
            loop {
                let (length, from) = tokio::select! {_ =stopped.cancelled()=>break,r=socket.recv_from(&mut packet)=>r.unwrap()};
                if !mappings.contains_key(&from) {
                    assert!(mappings.len() < 4, "NAT fixture path budget exceeded");
                    let upstream = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
                    seen.lock()
                        .unwrap()
                        .insert(from, upstream.local_addr().unwrap());
                    let replies = upstream.clone();
                    let client = socket.clone();
                    let cancel = stopped.clone();
                    readers.spawn(async move {
                        let mut packet=[0;65_535];
                        loop {
                            let (length,source)=tokio::select!{_=cancel.cancelled()=>break,r=replies.recv_from(&mut packet)=>r.unwrap()};
                            assert_eq!(source,server,"fixture accepted a foreign server");
                            client.send_to(&packet[..length],from).await.unwrap();
                        }
                    });
                    mappings.insert(from, upstream);
                }
                mappings[&from]
                    .send_to(&packet[..length], server)
                    .await
                    .unwrap();
            }
            readers.abort_all();
            while readers.join_next().await.is_some() {}
        });
        Self {
            address,
            paths,
            stop,
            task,
        }
    }
    async fn wait_paths(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.paths.lock().unwrap().len() != count {
                assert!(!self.task.is_finished(), "NAT witness terminated");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual QUIC path change was not witnessed");
        let paths = self.paths.lock().unwrap();
        assert_eq!(
            paths.values().copied().collect::<BTreeSet<_>>().len(),
            count
        );
    }
    async fn shutdown(&mut self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for Nat {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_ca_chain_and_native_duplex_survive_real_nat_rebinding_eddsa() {
    migration("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_ca_chain_and_native_duplex_survive_real_nat_rebinding_es256() {
    migration("ES256").await;
}
async fn migration(algorithm: &str) {
    let mut f = configured(algorithm, true).await;
    let mut nat = Nat::new(f.server.address).await;
    f.client.endpoint = nat.address;
    let mut scene = codecs::setup_started_dtmf(f, true).await;
    nat.wait_paths(2).await;
    {
        let (source, target) = scene.clients.split_at_mut(1);
        codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 100).await;
        codecs::decoded_audio(&target[0], &mut source[0], scene.th, 0, scene.sh, 0, 200).await;
    }
    let db = rusqlite::Connection::open_with_flags(
        &scene.f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let bindings = || {
        db.prepare(
            "SELECT id,generation,observed_runtime_id,readiness_json FROM connections ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let before = bindings();
    assert_eq!(before.len(), 2);
    for (index, client) in scene.clients.iter_mut().enumerate() {
        let bound = client.rebind_network("0.0.0.0:0".parse().unwrap()).unwrap();
        assert!(bound.port() != 0);
        let id = client.keepalive().await.unwrap();
        reply(client, &id, "ack").await;
        nat.wait_paths(3 + index).await;
    }
    {
        let (source, target) = scene.clients.split_at_mut(1);
        codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 300).await;
        codecs::decoded_audio(&target[0], &mut source[0], scene.th, 0, scene.sh, 0, 400).await;
    }
    assert_eq!(
        bindings(),
        before,
        "path migration replaced logical media bindings"
    );
    eprintln!(
        "native-remote-witness {}",
        json!({"algorithm":algorithm,
        "nat_paths":nat.paths.lock().unwrap().len(),"logical_bindings_preserved":true,
        "decoded_directions_before_and_after":4})
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT max(attempt) FROM operation_effects", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        scene
            .f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        2
    );
    drop(db);
    for client in scene.clients {
        client.shutdown().await.unwrap();
    }
    scene.f.server.shutdown().await.unwrap();
    nat.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_refuses_wrong_host_and_unconfigured_roots_before_commands() {
    let f = configured("EdDSA", true).await;
    let mut wrong = f.client.clone();
    wrong.server_name = "foreign.fixture.invalid".into();
    assert!(CanonicalClient::open(wrong).await.is_err());
    let unrelated = rcgen::generate_simple_self_signed(vec![NAME.into()]).unwrap();
    let path = f.temp.path().join("unconfigured-root.der");
    std::fs::write(&path, unrelated.cert.der()).unwrap();
    let mut wrong = f.client.clone();
    wrong.trust_certificate = path;
    assert!(CanonicalClient::open(wrong).await.is_err());
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let id = client.keepalive().await.unwrap();
    reply(&mut client, &id, "ack").await;
    client.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_requires_its_complete_intermediate_chain() {
    let f = configured("EdDSA", false).await;
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    f.server.shutdown().await.unwrap();
}

fn fixture_endpoint(mut tls: rustls::ServerConfig) -> quinn::Endpoint {
    tls.alpn_protocols = vec![rvoip_uctp::v2::RAW_QUIC_ALPN.to_vec()];
    quinn::Endpoint::server(
        quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap(),
        )),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_missing_invalid_and_oversized_trust_never_opens_commands() {
    let f = configured("EdDSA", true).await;
    for (label, bytes) in [
        ("missing", None),
        ("empty", Some(Vec::new())),
        ("invalid", Some(b"not a certificate".to_vec())),
        ("oversized", Some(vec![0; 256 * 1024 + 1])),
    ] {
        let path = f.temp.path().join(format!("{label}-trust.pem"));
        if let Some(bytes) = bytes {
            std::fs::write(&path, bytes).unwrap();
        }
        let mut config = f.client.clone();
        config.trust_certificate = path;
        assert!(
            matches!(
                CanonicalClient::open(config).await,
                Err(ClientError::Configuration)
            ),
            "{label}"
        );
    }
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let id = client.keepalive().await.unwrap();
    reply(&mut client, &id, "ack").await;
    client.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

fn credential_config(f: &Fixture, label: &str, expires: i64) -> CanonicalClientConfig {
    let credential = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::collaboration(vec!["*".into()]),
            expires,
        )
        .unwrap();
    let mut config = f.client.clone();
    config.credential = credential.token;
    config.signing_key_id = format!("remote-{label}-key");
    config.signing_private_key = f.temp.path().join(format!("remote-{label}-key.der"));
    let public = signing(&config.signing_private_key, "EdDSA");
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: format!("remote-{label}-enrollment"),
                identity_id: config.identity_id.clone(),
                credential_id: credential.id,
                device_id: config.device_id.clone(),
                key_id: config.signing_key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    config
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_refuses_expired_and_revoked_credentials_without_media() {
    let f = configured("EdDSA", true).await;
    let revoked = credential_config(&f, "revocation", chrono::Utc::now().timestamp() + 600);
    let mut before = CanonicalClient::open(revoked.clone()).await.unwrap();
    let id = before.keepalive().await.unwrap();
    reply(&mut before, &id, "ack").await;
    before.shutdown().await.unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let id = f
        .server
        .store
        .authenticate(&revoked.credential)
        .unwrap()
        .credential_id;
    f.server.store.revoke(&admin, &id).unwrap();
    assert!(CanonicalClient::open(revoked).await.is_err());
    let expires = chrono::Utc::now().timestamp() + 3;
    let expired = credential_config(&f, "expiry", expires);
    let mut before = CanonicalClient::open(expired.clone()).await.unwrap();
    let id = before.keepalive().await.unwrap();
    reply(&mut before, &id, "ack").await;
    before.shutdown().await.unwrap();
    while chrono::Utc::now().timestamp() < expires {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(CanonicalClient::open(expired).await.is_err());
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let id = client.keepalive().await.unwrap();
    reply(&mut client, &id, "ack").await;
    client.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_one_open_deadline_spans_delayed_challenge_and_stalled_auth() {
    use rvoip_auth_core::sig9421::Sig9421Signer;
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = configured("EdDSA", true).await;
    let tls =
        connect_server::tls::server_config(&f.config.certificate, &f.config.private_key).unwrap();
    let endpoint = fixture_endpoint((*tls).clone());
    let mut config = f.client.clone();
    config.endpoint = endpoint.local_addr().unwrap();
    config.timeout_ms = 1500;
    let signer = Sig9421Signer::ed25519(
        &config.server_key_id,
        &std::fs::read(&f.config.signing_private_key).unwrap(),
    )
    .unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let stage = reached.clone();
    let server = tokio::spawn(async move {
        let connection = endpoint.accept().await.unwrap().await.unwrap();
        let (mut writer, mut reader) = connection.accept_bi().await.unwrap();
        async fn read(reader: &mut quinn::RecvStream) -> Envelope {
            let mut prefix = [0; 4];
            reader.read_exact(&mut prefix).await.unwrap();
            let size = u32::from_be_bytes(prefix) as usize;
            assert!(size > 0 && size <= rvoip_uctp::v2::MAX_ENVELOPE_BYTES);
            let mut raw = vec![0; size];
            reader.read_exact(&mut raw).await.unwrap();
            Envelope::parse(&raw).unwrap()
        }
        let hello = read(&mut reader).await;
        assert_eq!(hello.kind(), "auth.hello");
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let challenge = json!({"v":2,"type":"auth.challenge","id":"remote-delayed-challenge","ts":chrono::Utc::now(),"in_reply_to":hello.id(),"payload":{"nonce":"bounded-auth-fixture","accepted_methods":["bearer"],"server_capabilities":{"application_profiles":["connect/2"]}}});
        let bytes = serde_json::to_vec(&signer.sign(&challenge).unwrap()).unwrap();
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&bytes).await.unwrap();
        assert_eq!(read(&mut reader).await.kind(), "auth.response");
        stage.store(true, Ordering::Release);
        // A per-stage deadline would wait another1500ms. The one open budget
        // must instead retire this real TLS peer at about1500ms total.
        tokio::time::timeout(Duration::from_secs(2), connection.closed())
            .await
            .expect("timed-out client kept its authentication peer alive");
        endpoint.close(0u32.into(), b"isolated authentication fixture complete");
    });
    let started = tokio::time::Instant::now();
    assert!(matches!(
        CanonicalClient::open(config).await,
        Err(ClientError::Timeout)
    ));
    assert!(
        reached.load(Ordering::Acquire),
        "did not reach delayed second authentication stage"
    );
    assert!(
        started.elapsed() < Duration::from_millis(2100),
        "open renewed its deadline between authentication stages"
    );
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_profile_expired_tls_is_refused_by_server_and_actual_client_verifier() {
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let f = configured("EdDSA", true).await;
    let mut ca = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca = rcgen::CertifiedIssuer::self_signed(ca, rcgen::KeyPair::generate().unwrap()).unwrap();
    let mut params = rcgen::CertificateParams::new(vec![NAME.into()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2020, 1, 2);
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.signed_by(&key, &ca).unwrap();
    let leaf = f.temp.path().join("expired-leaf.pem");
    let private = f.temp.path().join("expired-key.pem");
    let trust = f.temp.path().join("expiry-root.pem");
    std::fs::write(&leaf, cert.pem()).unwrap();
    std::fs::write(&private, key.serialize_pem()).unwrap();
    std::fs::write(&trust, ca.pem()).unwrap();
    let mut server = f.config.clone();
    server.certificate = leaf;
    server.private_key = private;
    assert!(RunningCanonicalServer::start(server).await.is_err());
    // The isolated raw server intentionally presents expired bytes so the real
    // SDK's normal verifier is also exercised. No verification bypass is used.
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .unwrap();
    tls.alpn_protocols = vec![rvoip_uctp::v2::RAW_QUIC_ALPN.to_vec()];
    let endpoint = fixture_endpoint(tls);
    let mut config = f.client.clone();
    config.endpoint = endpoint.local_addr().unwrap();
    config.trust_certificate = trust;
    let peer = tokio::spawn(async move {
        let incoming = endpoint.accept().await.unwrap();
        assert!(incoming.await.is_err());
    });
    assert!(CanonicalClient::open(config).await.is_err());
    tokio::time::timeout(Duration::from_secs(3), peer)
        .await
        .unwrap()
        .unwrap();
    let mut original = CanonicalClient::open(f.client.clone()).await.unwrap();
    let id = original.keepalive().await.unwrap();
    reply(&mut original, &id, "ack").await;
    original.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Child-only isolated guest fixture; invoked by the distinct-host runner"]
async fn distinct_host_server_child() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::path::PathBuf::from(
        std::env::var_os("CONNECT_G6_DISTINCT_NATIVE_DIR").expect("child fixture directory"),
    );
    let dir = dir.canonicalize().unwrap();
    assert!(dir.starts_with(
        std::env::current_dir()
            .unwrap()
            .join(".local/gate-6/native-remote")
            .canonicalize()
            .unwrap()
    ));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let algorithm = std::env::var("CONNECT_G6_DISTINCT_NATIVE_ALGORITHM").unwrap();
    assert!(matches!(algorithm.as_str(), "EdDSA" | "ES256"));
    let f = configured_at(&algorithm, true, "0.0.0.0:0".parse().unwrap()).await;
    let mut client = f.client.clone();
    for (source, name) in [
        (&client.signing_private_key, "client-key.der"),
        (&client.server_public_key, "server-key.pub"),
        (&client.trust_certificate, "trust.pem"),
    ] {
        std::fs::copy(source, dir.join(name)).unwrap();
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    client.signing_private_key = "client-key.der".into();
    client.server_public_key = "server-key.pub".into();
    client.trust_certificate = "trust.pem".into();
    // The guest resolves only its designated host before opening; TLS continues
    // to verify native.fixture.invalid and these exact configured trust roots.
    client.endpoint =
        std::net::SocketAddr::new("127.0.0.1".parse().unwrap(), f.server.address.port());
    std::fs::write(dir.join("client.toml"), toml::to_string(&client).unwrap()).unwrap();
    std::fs::set_permissions(
        dir.join("client.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::write(dir.join("ready-writing.json"),serde_json::to_vec(&json!({"algorithm":algorithm,"port":f.server.address.port(),"configuration":"client.toml"})).unwrap()).unwrap();
    std::fs::rename(dir.join("ready-writing.json"), dir.join("ready.json")).unwrap();
    tokio::time::timeout(Duration::from_secs(180), async {
        while !dir.join("done").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (operations, kind, state): (u64, Option<String>, Option<String>) = db.query_row(
        "SELECT count(*),min(command_type),min(state) FROM operations WHERE operation_id='distinct-host-create' AND owner_id=?1",
        [&f.client.identity_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(
        (operations, kind.as_deref(), state.as_deref()),
        (1, Some("create"), Some("completed")),
        "guest reconnect did not preserve a single durable create mutation"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(db);
    tokio::time::timeout(Duration::from_secs(5), async {
        while f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .physical_peers
            != 0
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    eprintln!(
        "distinct-native-server-witness {}",
        json!({"algorithm":algorithm,"durable_create_operations":operations,"old_physical_peers_drained":true,"distinct_host_media_qualified":false})
    );
    f.server.shutdown().await.unwrap();
}
