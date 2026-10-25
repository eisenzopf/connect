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

async fn configured(algorithm: &str, complete_chain: bool) -> Fixture {
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
    config.bind = "127.0.0.1:0".parse().unwrap();
    config.certificate = certificate;
    config.private_key = private_key;
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    client.network_profile = connect_client::NativeNetworkProfile::Remote;
    client.server_name = NAME.into();
    client.trust_certificate = trust;
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
