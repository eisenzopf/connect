//! Actual TLS WebSocket control against the shared canonical host. No WebRTC claim.
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    WebSocketStream,
};
type Socket = WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

async fn configured(algorithm: &str, capacity: usize, expiry: Option<chrono::Duration>) -> Fixture {
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = fixture(algorithm).await;
    server.shutdown().await.unwrap();
    config.limits.connections = capacity;
    config.websocket = Some(
        connect_server::canonical_websocket::CanonicalWebSocketConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            mount_path: MOUNT.into(),
            allowed_origins: vec![ORIGIN.into()],
            advertised_url: None,
        },
    );
    if let Some(expiry) = expiry {
        let expiry = chrono::Utc::now() + expiry;
        config.signing_key_expires_at = expiry;
        client.server_key_expires_at = expiry;
    }
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    assert_eq!(
        server.websocket_url,
        Some(format!(
            "wss://{}{MOUNT}",
            server.websocket_address.unwrap()
        ))
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
async fn socket(
    f: &Fixture,
    path: &str,
    origins: &[&str],
    trusted: bool,
) -> Result<Socket, String> {
    let mut roots = rustls::RootCertStore::empty();
    if trusted {
        roots
            .add(rustls::pki_types::CertificateDer::from(
                std::fs::read(&f.client.trust_certificate).unwrap(),
            ))
            .unwrap();
    }
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let work = async {
        let tcp = tokio::net::TcpStream::connect(f.server.websocket_address.unwrap())
            .await
            .map_err(|_| "TCP refused".to_string())?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(tls))
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                tcp,
            )
            .await
            .map_err(|_| "TLS refused".to_string())?;
        let mut request = format!("wss://{}{path}", f.server.websocket_address.unwrap())
            .into_client_request()
            .unwrap();
        for origin in origins {
            request
                .headers_mut()
                .append("origin", origin.parse().unwrap());
        }
        tokio_tungstenite::client_async(request, tls)
            .await
            .map(|(socket, _)| socket)
            .map_err(|_| "Upgrade refused".into())
    };
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .map_err(|_| "TLS/upgrade deadline".to_string())?
}
struct Peer {
    socket: Socket,
    signer: Sig9421Signer,
    verifier: Sig9421Verifier,
    context: SignatureVerificationContext,
}
impl Peer {
    async fn open(f: &Fixture) -> Self {
        Self::open_config(f, &f.client).await
    }
    async fn open_config(f: &Fixture, config: &CanonicalClientConfig) -> Self {
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
        let mut peer = Self {
            socket: socket(f, MOUNT, &[ORIGIN], true).await.unwrap(),
            signer,
            verifier: Sig9421Verifier::new(Arc::new(resolver)),
            context,
        };
        let id=peer.send(envelope("auth.hello",json!({"device":{"id":config.device_id,"kind":config.device_kind,"platform":config.device_platform,"sdk_version":"independent-wss-fixture"},"auth_methods":["bearer"],"application_profiles":["connect/2"]})),false).await;
        let challenge = peer.read().await;
        assert_eq!(challenge.kind(), "auth.challenge");
        assert_eq!(challenge.original()["in_reply_to"], id);
        let mut response = envelope(
            "auth.response",
            json!({"method":"bearer","credential":config.credential}),
        );
        response["in_reply_to"] = json!(challenge.id());
        let id = peer.send(response, true).await;
        let accepted = peer.read().await;
        assert_eq!(accepted.kind(), "auth.session");
        assert_eq!(accepted.original()["in_reply_to"], id);
        assert_eq!(
            accepted.original()["payload"]["application_profile"],
            "connect/2"
        );
        assert_eq!(
            accepted.original()["payload"]["identity_id"],
            config.identity_id
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
        tokio::time::timeout(
            Duration::from_secs(5),
            self.socket
                .send(Message::Text(serde_json::to_string(&value).unwrap().into())),
        )
        .await
        .unwrap()
        .unwrap();
        id
    }
    async fn read(&mut self) -> Envelope {
        let text = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match self.socket.next().await {
                    Some(Ok(Message::Text(text))) => return text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    other => panic!("unexpected WSS frame: {other:?}"),
                }
            }
        })
        .await
        .expect("WSS frame deadline");
        let frame = Envelope::parse(text.as_bytes()).unwrap();
        self.verifier
            .verify_with_context(frame.original(), &self.context)
            .await
            .unwrap();
        frame
    }
    async fn create(&mut self, f: &Fixture) -> String {
        let mut input = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        input.tenant_id = Some("tnt_default".into());
        let mut value = envelope("conversation.create", serde_json::to_value(input).unwrap());
        value["operation_id"] = json!("wss-original-create");
        let id = self.send(value, true).await;
        for _ in 0..128 {
            let frame = self.read().await;
            assert_eq!(frame.original()["in_reply_to"], id);
            assert_ne!(frame.kind(), "error", "{:?}", frame.original());
            if frame.kind() == "ack" {
                let conversation: connect_protocol::Conversation = serde_json::from_value(
                    frame.original()["payload"]["details"]["conversation"].clone(),
                )
                .unwrap();
                assert!(conversation
                    .receipt
                    .as_ref()
                    .unwrap()
                    .matches("wss-original-create", &conversation.id));
                return conversation.id;
            }
        }
        panic!("WSS reply batch exceeded bound")
    }
    async fn closed(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(frame) = self.socket.next().await {
                if frame.is_err() || matches!(frame, Ok(Message::Close(_))) {
                    return;
                }
            }
        })
        .await
        .expect("WSS physical retirement deadline");
    }
}
async fn count(f: &Fixture, want: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .physical_peers
                == want
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shared physical count deadline");
}
async fn control(algorithm: &str) {
    let f = configured(algorithm, 8, None).await;
    let discovery = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap()
        .transport_discovery()
        .await
        .unwrap();
    assert_eq!(
        discovery.websocket.unwrap().url,
        f.server.websocket_url.clone().unwrap()
    );
    assert_eq!(
        discovery.webtransport.unwrap().url,
        f.server.webtransport_url.clone().unwrap()
    );
    let mut first = Peer::open(&f).await;
    let cid = first.create(&f).await;
    first.socket.close(None).await.unwrap();
    drop(first);
    count(&f, 0).await;
    let mut second = Peer::open(&f).await;
    assert_eq!(second.create(&f).await, cid);
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let status = native
        .operation_get(
            "wss-original-create",
            connect_client::canonical::CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        status.state,
        rvoip_uctp::v2::payloads::OperationState::Completed
    );
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE operation_id='wss-original-create' AND owner_id=?1",[&f.client.identity_id],|r|r.get::<_,u64>(0)).unwrap(),1);
    drop(db);
    native.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
    second.closed().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_wss_signed_control_reconnect_and_native_original_receipt_eddsa() {
    control("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_wss_signed_control_reconnect_and_native_original_receipt_es256() {
    control("ES256").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_origin_mount_refusals_do_not_consume_canonical_slots() {
    let f = configured("EdDSA", 8, None).await;
    let before = connect_store::Store::inspect_snapshot(&f.config.database).unwrap();
    for (path, origins, trusted) in [
        (MOUNT, vec![ORIGIN], false),
        (MOUNT, vec![], true),
        (MOUNT, vec!["https://wrong.fixture.invalid"], true),
        (MOUNT, vec![ORIGIN, ORIGIN], true),
        ("/wrong", vec![ORIGIN], true),
    ] {
        assert!(socket(&f, path, &origins, trusted).await.is_err());
    }
    count(&f, 0).await;
    assert_eq!(
        connect_store::Store::inspect_snapshot(&f.config.database).unwrap(),
        before
    );
    let mut peer = Peer::open(&f).await;
    peer.create(&f).await;
    f.server.shutdown().await.unwrap();
    peer.closed().await;
}
async fn http3_original_status(peer: &mut Http3Peer, cid: &str) {
    let mut request = envelope(
        "operation.get",
        json!({"operation_id":"wss-original-create"}),
    );
    request["cid"] = json!(cid);
    let id = peer.send(request, true).await;
    let mut observed = false;
    for _ in 0..128 {
        let frame = peer.read().await;
        assert_eq!(frame.original()["in_reply_to"], id);
        assert_ne!(frame.kind(), "error", "{:?}", frame.original());
        if frame.kind() == "operation.updated" {
            assert_eq!(
                frame.original()["payload"]["operation_id"],
                "wss-original-create"
            );
            assert_eq!(frame.original()["payload"]["state"], "completed");
            observed = true;
        }
        if frame.kind() == "ack" {
            assert!(observed);
            return;
        }
    }
    panic!("HTTP3 original status reply exceeded bound");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn physical_budget_is_shared_across_wss_http3_and_native_and_reuses_retired_slots() {
    let f = configured("EdDSA", 2, None).await;
    let mut ws = Peer::open(&f).await;
    let cid = ws.create(&f).await;
    let mut http3 = Http3Peer::open(&f).await;
    // Core peer views are lazy. Establish held authenticated transports through
    // real signed round trips, then prove the shared limit at actual admission.
    http3_original_status(&mut http3, &cid).await;
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    assert_eq!(ws.create(&f).await, cid);
    http3_original_status(&mut http3, &cid).await;
    http3.close().await;
    drop(http3);
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    assert_eq!(
        native
            .operation_get(
                "wss-original-create",
                connect_client::canonical::CommandScope {
                    conversation_id: Some(cid.clone()),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .state,
        rvoip_uctp::v2::payloads::OperationState::Completed
    );
    native.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
    ws.closed().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_host_closes_live_wss_and_releases_tcp_listener() {
    let f = configured("EdDSA", 8, None).await;
    let mut ws = Peer::open(&f).await;
    let cid = ws.create(&f).await;
    let address = f.server.websocket_address.unwrap();
    let service = f.server.service.clone();
    drop(f.server);
    ws.closed().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::net::TcpListener::bind(address).await.is_ok()
                && service.canonical_runtime_snapshot().unwrap().physical_peers == 0
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!cid.is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signing_expiry_retires_wss_and_linked_native_and_http3() {
    let f = configured("EdDSA", 8, Some(chrono::Duration::seconds(3))).await;
    let mut ws = Peer::open(&f).await;
    let cid = ws.create(&f).await;
    let native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let wt = Http3Peer::open(&f).await;
    ws.closed().await;
    count(&f, 0).await;
    assert!(socket(&f, MOUNT, &[ORIGIN], true).await.is_err());
    assert!(CanonicalClient::open(f.client.clone()).await.is_err());
    assert!(transport(&f, MOUNT, &[ORIGIN]).await.is_err());
    drop(native);
    drop(wt);
    assert!(!cid.is_empty());
    assert!(
        f.server.shutdown().await.is_err(),
        "unexpected intake expiry must remain visible"
    );
}

async fn production_sdk(algorithm: &str) {
    let f = configured(algorithm, 8, None).await;
    let c = &f.client;
    let configuration = json!({"origin":ORIGIN,"url":f.server.websocket_url,"credential":c.credential,"identity_id":c.identity_id,"other_identity":f.peer.identity_id,"device":{"id":c.device_id,"kind":c.device_kind,"platform":c.device_platform},"private_pkcs8":URL_SAFE_NO_PAD.encode(std::fs::read(&c.signing_private_key).unwrap()),"keyid":c.signing_key_id,"algorithm":algorithm,"server_keyid":c.server_key_id,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let config = f.temp.path().join("sdk-wss.json");
    std::fs::write(&config, serde_json::to_vec(&configuration).unwrap()).unwrap();
    let certificate = base64::engine::general_purpose::STANDARD
        .encode(std::fs::read(&c.trust_certificate).unwrap());
    let pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        certificate
            .as_bytes()
            .chunks(64)
            .map(|x| std::str::from_utf8(x).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let trust = f.temp.path().join("sdk-wss-trust.pem");
    std::fs::write(&trust, pem).unwrap();
    let node = std::env::var_os("CONNECT_GATE6_NODE").unwrap_or_else(|| "node".into());
    let mut command = tokio::process::Command::new(node);
    command
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gate6-webtransport-browser/sdk-wss-node.mjs"),
        )
        .arg(config)
        .env("NODE_EXTRA_CA_CERTS", trust)
        .env_remove("NODE_TLS_REJECT_UNAUTHORIZED")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .expect("owned Node WSS deadline")
        .unwrap();
    assert!(output.stdout.len() < 65536 && output.stderr.len() < 65536);
    assert!(
        output.status.success(),
        "Production SDK WSS failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["passed"], true);
    println!("wss_sdk_evidence {value}");
    count(&f, 0).await;
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE operation_id='wss-sdk-original'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sdk_uses_actual_tls_wss_and_recovers_original_receipt_eddsa() {
    production_sdk("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sdk_uses_actual_tls_wss_and_recovers_original_receipt_es256() {
    production_sdk("ES256").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_host_shutdown_keeps_wss_and_pending_tls_owned() {
    let f = configured("EdDSA", 8, None).await;
    let mut ws = Peer::open(&f).await;
    ws.create(&f).await;
    let address = f.server.websocket_address.unwrap();
    let service = f.server.service.clone();
    let mut pending = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut shutdown = Box::pin(f.server.shutdown());
    assert!(matches!(
        futures_util::poll!(&mut shutdown),
        std::task::Poll::Pending
    ));
    drop(shutdown);
    ws.closed().await;
    use tokio::io::AsyncReadExt;
    let mut buffer = [0; 1];
    let result = tokio::time::timeout(Duration::from_secs(5), pending.read(&mut buffer))
        .await
        .unwrap();
    assert!(
        matches!(result, Ok(0) | Err(_)),
        "pending TLS socket outlived its owner"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::net::TcpListener::bind(address).await.is_ok()
                && service.canonical_runtime_snapshot().unwrap().physical_peers == 0
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[path = "canonical_websocket_control/webrtc.rs"]
mod webrtc;
