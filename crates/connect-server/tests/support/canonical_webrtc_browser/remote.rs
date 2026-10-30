//! Child-only Remote host for actual browsers in the owned local Linux VM.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Child-only Linux browser to Mac fixture; normal CA and synthetic AV"]
async fn distinct_network_browser_server_child() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let _audio_trace = audio_trace::install(&root);
    let output = PathBuf::from(std::env::var_os("CONNECT_G6_REMOTE_BROWSER_DIR").unwrap())
        .canonicalize()
        .unwrap();
    assert!(output.starts_with(
        root.join(".local/gate-6/browser-remote")
            .canonicalize()
            .unwrap()
    ));
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
    let algorithm = std::env::var("CONNECT_G6_REMOTE_BROWSER_ALGORITHM").unwrap();
    assert!(matches!(algorithm.as_str(), "EdDSA" | "ES256"));
    let port: u16 = std::env::var("CONNECT_G6_REMOTE_BROWSER_MEDIA_PORT")
        .unwrap()
        .parse()
        .unwrap();
    assert!((20000..=60000).contains(&port));
    let bridge = std::env::var("CONNECT_G6_OWNED_BROWSER_BRIDGE_IP")
        .ok()
        .map(|value| {
            assert!(cfg!(all(target_os = "linux", target_arch = "x86_64")));
            assert_eq!(
                std::env::var("CONNECT_G6_OWNED_BROWSER_BRIDGE").as_deref(),
                Ok("1")
            );
            let ip: std::net::Ipv4Addr = value.parse().unwrap();
            assert!(ip.is_private());
            ip
        });
    let hostname = if bridge.is_some() {
        "connect.fixture.internal"
    } else {
        "host.lima.internal"
    };
    let bind_ip = bridge.unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
    let public_ip = bridge.unwrap_or(std::net::Ipv4Addr::new(192, 168, 5, 2));
    let relay = std::env::var_os("CONNECT_G6_REMOTE_BROWSER_RELAY_CONFIG").map(|path| {
        assert!(
            bridge.is_some(),
            "Relay extension requires the owned internal bridge"
        );
        let path = PathBuf::from(path).canonicalize().unwrap();
        assert_eq!(path, output.join("turn/fixture.json"));
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let ip: std::net::Ipv4Addr = value["ip"].as_str().unwrap().parse().unwrap();
        assert!(ip.is_private() && Some(ip) != bridge);
        let url = value["url"].as_str().unwrap();
        assert!(
            url == format!("turn:{ip}:3478?transport=udp")
                || url == format!("turn:{ip}:3478?transport=tcp")
        );
        assert_eq!(value["relay_port_start"], 32000);
        assert_eq!(value["relay_port_end"], 32031);
        value
    });
    let mut f = Fixture::with_native_end_scope(&algorithm, 8).await;
    f.server.shutdown().await.unwrap();
    let now = std::time::SystemTime::now();
    let mut ca = rcgen::CertificateParams::default();
    ca.not_before = (now - Duration::from_secs(60)).into();
    ca.not_after = (now + Duration::from_secs(3600)).into();
    ca.distinguished_name.push(
        rcgen::DnType::CommonName,
        "Connect owned Linux browser root",
    );
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let issuer =
        rcgen::CertifiedIssuer::self_signed(ca, rcgen::KeyPair::generate().unwrap()).unwrap();
    let mut leaf =
        rcgen::CertificateParams::new(vec!["localhost".into(), hostname.into()]).unwrap();
    leaf.not_before = (now - Duration::from_secs(60)).into();
    leaf.not_after = (now + Duration::from_secs(3600)).into();
    leaf.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = leaf.signed_by(&key, &issuer).unwrap();
    let ca_path = output.join("root.der");
    std::fs::write(&ca_path, issuer.der()).unwrap();
    let mut assets = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    assets.not_before = (now - Duration::from_secs(60)).into();
    assets.not_after = (now + Duration::from_secs(3600)).into();
    assets.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    assets.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let assets_key = rcgen::KeyPair::generate().unwrap();
    let assets_certificate = assets.signed_by(&assets_key, &issuer).unwrap();
    std::fs::write(output.join("asset-cert.pem"), assets_certificate.pem()).unwrap();
    std::fs::write(output.join("asset-key.pem"), assets_key.serialize_pem()).unwrap();
    std::fs::set_permissions(
        output.join("asset-key.pem"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    f.config.certificate = f.temp.path().join("remote-browser-leaf.der");
    f.config.private_key = f.temp.path().join("remote-browser-key.der");
    std::fs::write(&f.config.certificate, certificate.der()).unwrap();
    std::fs::write(&f.config.private_key, key.serialize_der()).unwrap();
    f.client.trust_certificate = ca_path;
    f.config.network_profile = connect_client::NativeNetworkProfile::Remote;
    let native = std::net::UdpSocket::bind((bind_ip, 0)).unwrap();
    f.config.bind = native.local_addr().unwrap();
    // The shared enrolled-peer fixture also declares a WebTransport Device.
    // Give that optional listener its own exact Remote contact; this browser
    // rehearsal still selects signed WSS and separate RTC media.
    f.config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec!["https://localhost:30379".into()],
            advertised_url: Some(format!(
                "https://{hostname}:{}/connect",
                f.config.bind.port()
            )),
        },
    );
    let reserved = std::net::TcpListener::bind((bind_ip, 0)).unwrap();
    let signaling_port = reserved.local_addr().unwrap().port();
    let url = format!("wss://{hostname}:{signaling_port}/connect");
    f.config.websocket = Some(
        connect_server::canonical_websocket::CanonicalWebSocketConfig {
            bind: (bind_ip, signaling_port).into(),
            mount_path: "/connect".into(),
            allowed_origins: vec!["https://localhost:30379".into()],
            advertised_url: Some(url.clone()),
        },
    );
    f.config.webrtc = Some(connect_server::canonical_webrtc::CanonicalWebRtcConfig {
        profile: connect_server::canonical_webrtc::CanonicalWebRtcProfile::OpusVp8,
        max_peers: 2,
        addressing: Some(
            connect_server::canonical_webrtc::CanonicalWebRtcAddressing {
                bind_ip: bind_ip.into(),
                port_start: port,
                port_end: port + 15,
                public_ip: Some(public_ip.into()),
                ice_policy: if relay.is_some() {
                    connect_server::canonical_webrtc::CanonicalIcePolicy::Relay
                } else {
                    connect_server::canonical_webrtc::CanonicalIcePolicy::All
                },
                ice_services: relay
                    .as_ref()
                    .map(|turn| {
                        vec![connect_server::canonical_webrtc::CanonicalIceService {
                            urls: vec![turn["url"].as_str().unwrap().to_owned()],
                            username_ref: Some("CONNECT_GATE6_TURN_USER".into()),
                            credential_ref: Some("CONNECT_GATE6_TURN_PASSWORD".into()),
                        }]
                    })
                    .unwrap_or_default(),
            },
        ),
    });
    drop(reserved);
    drop(native);
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = (
        bridge.unwrap_or(std::net::Ipv4Addr::LOCALHOST),
        f.server.address.port(),
    )
        .into();
    f.client.network_profile = connect_client::NativeNetworkProfile::Remote;
    let host = f.server.test_webrtc_host().unwrap();
    let other = canonical_webtransport_media::other_caps_with_end(
        &f,
        vec!["audio".into(), "video".into()],
        true,
    );
    enroll(&f, &f.client, 0).await;
    enroll(&f, &other, 1).await;
    let mut control = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut create =
        connect_client::conversations::CreateRequest::persistent(vec![other.identity_id.clone()]);
    create.tenant_id = Some("tnt_default".into());
    let room = control
        .create_conversation("remote-browser-room", create)
        .await
        .unwrap();
    let mine = &room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id;
    let theirs = &room
        .participants
        .iter()
        .find(|p| p.identity_id == other.identity_id)
        .unwrap()
        .participant_id;
    let caps = json!({"audio":["opus"],"video":["vp8"]});
    control
        .session_command(Request {
            operation_id: "browser-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "browser-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![theirs.clone()],
                medium: "voice+video".into(),
                intent: "conference".into(),
                capabilities: caps.clone(),
            },
        })
        .await
        .unwrap();
    let mut http = canonical_http_config(&f);
    http.credential = other.credential.clone();
    let accepted = CanonicalHttpClient::open(http)
        .await
        .unwrap()
        .session_command(Request {
            operation_id: "browser-accept".into(),
            conversation_id: room.id.clone(),
            session_id: "browser-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept { capabilities: caps },
        })
        .await
        .unwrap();
    let connect_protocol::Outcome::SessionControl(accepted) = accepted else {
        panic!("Session acceptance required")
    };
    control.shutdown().await.unwrap();
    let mut participants = vec![
        participant(&f, &f.client, &room.id, mine, theirs, 0),
        participant(&f, &other, &room.id, theirs, mine, 1),
    ];
    for participant in &mut participants {
        participant["url"] = json!(url);
        if let Some(turn) = &relay {
            participant["connection"]["iceTransportPolicy"] = json!("relay");
            participant["connection"]["iceServers"] = json!([{
                "urls": turn["url"], "username": turn["username"], "credential": turn["credential"],
            }]);
        }
    }
    let bridge_path = bridge.map(|ip| match &relay {
        Some(turn) => {
            json!({"ip":turn["ip"],"port_start":32000,"port_end":32031,"require_relay":true})
        }
        None => json!({"ip":ip.to_string(),"port_start":port,"port_end":port+15}),
    });
    let spec = json!({"browser":"chrome","root":root,"assetPort":30379,"assetTls":{"certFile":"/fixture/asset-cert.pem","keyFile":"/fixture/asset-key.pem"},"seconds":60,"profileMode":"clean",
        "designatedRemoteHost":hostname,"ownedContainerBridge":bridge_path,"requireRelay":relay.is_some(),"iceRestart":relay.is_some(),"sourceIds":[0,1],"sourceToneHz":[440,660],"expectedToneHz":[660,440],
        "participants":participants,"terminal":{"operationId":"browser-session-end","expectedRevision":accepted.session.topology_revision}});
    let input = output.join("input.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(output.join("ready-writing.json"),serde_json::to_vec(&json!({"algorithm":algorithm,"url":url,"media_public_ip":public_ip.to_string(),"media_port_start":port,"media_port_end":port+15})).unwrap()).unwrap();
    std::fs::rename(output.join("ready-writing.json"), output.join("ready.json")).unwrap();
    tokio::time::timeout(Duration::from_secs(180), async {
        while !output.join("done").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.available_slots() != 2 || host.adapter().route_count() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("original RTC routes/capacity did not drain");
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        2
    );
    drop(db);
    let media_diagnostics = f.server.service.readiness.snapshot().media;
    f.server.shutdown().await.unwrap();
    eprintln!(
        "remote-browser-server-witness {}",
        json!({"algorithm":algorithm,"durable_offers":2,"physical_routes_drained":true,"capacity_reclaimed":2,"server_shutdown_joined":true,"media_diagnostics":media_diagnostics})
    );
}
