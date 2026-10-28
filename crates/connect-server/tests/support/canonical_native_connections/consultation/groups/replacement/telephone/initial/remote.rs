//! Owned Linux SIP/RTP peer, explicit Remote host addressing and public SDK audio.
use super::*;
use std::{
    net::{IpAddr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Child-only synthetic distinct-network SIP fixture; use the owned runner"]
async fn distinct_network_telephone_server_child() {
    let dir = PathBuf::from(std::env::var_os("CONNECT_G6_REMOTE_PHONE_DIR").unwrap())
        .canonicalize()
        .unwrap();
    assert!(dir.starts_with(
        std::env::current_dir()
            .unwrap()
            .join(".local/gate-6/telephone-remote")
            .canonicalize()
            .unwrap()
    ));
    let spec: Value =
        serde_json::from_slice(&std::fs::read(dir.join("server-input.json")).unwrap()).unwrap();
    let algorithm = spec["algorithm"].as_str().unwrap();
    assert!(matches!(algorithm, "EdDSA" | "ES256"));
    let media_ip: IpAddr = spec["host_media_ip"].as_str().unwrap().parse().unwrap();
    assert!(
        media_ip.is_ipv4()
            && !media_ip.is_loopback()
            && !media_ip.is_unspecified()
            && !media_ip.is_multicast()
    );
    let sip: SocketAddr = spec["server_sip"].as_str().unwrap().parse().unwrap();
    let peer_sip: SocketAddr = spec["peer_sip"].as_str().unwrap().parse().unwrap();
    assert_eq!(sip.ip(), media_ip);
    assert_eq!(peer_sip.ip(), media_ip);
    let start = u16::try_from(spec["media_port_start"].as_u64().unwrap()).unwrap();
    assert!((20000..=59984).contains(&start));
    let (f, _, _) = configured_initial_at(
        algorithm,
        20,
        EndpointTransport::Telephone,
        false,
        false,
        None,
        peer_sip,
    )
    .await;
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
    let phone = config.telephone.as_mut().unwrap();
    phone.bind = sip;
    phone.advertised = Some(sip);
    phone.media_bind_ip = Some(media_ip);
    phone.media_public_ip = Some(media_ip);
    phone.media_port_start = start;
    phone.media_port_end = start + 15;
    let path = temp.path().join("remote-telephone.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    config = CanonicalServerConfig::load(&path).unwrap();
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    client.network_profile = connect_client::NativeNetworkProfile::Remote;
    let f = Fixture {
        temp,
        config,
        server,
        client,
        peer,
        admin,
        key_revision,
    };
    let ready = dir.join("ready-writing.json");
    std::fs::write(&ready, serde_json::to_vec(&json!({"database":f.config.database,"algorithm":algorithm,"server_sip":sip,"host_media_ip":media_ip})).unwrap()).unwrap();
    std::fs::set_permissions(&ready, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(ready, dir.join("ready.json")).unwrap();
    let mut control = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = control
        .create_conversation("remote-phone-room", input)
        .await
        .unwrap();
    let target = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let request = connect_protocol::session_control::Request {
        operation_id: "remote-phone-invite".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![target],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    let receipt = control.session_command(request.clone()).await.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let state: Option<String> = db.query_row("SELECT o.state FROM operations o JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id WHERE json_extract(i.source_json,'$.parent.request.operation_id')='remote-phone-invite'", [], |r|r.get(0)).optional().unwrap();
            if state.as_deref() == Some("completed") { break; }
            assert!(!matches!(state.as_deref(), Some("failed" | "unknown" | "cancelled")), "telephone admission: {state:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(control.session_command(request).await.unwrap(), receipt);
    let audio_config = crate::canonical_native_connections::party_with_capabilities(
        &f,
        &f.client.identity_id,
        908,
        true,
        vec!["audio".into()],
    );
    let mut audio = CanonicalClient::open(audio_config.clone()).await.unwrap().prepare_public_native_audio(CommandScope {
        conversation_id: Some(room.id.clone()), session_id: Some("native-session".into()), connection_id: Some("remote-phone-native".into()), operation_id: Some("remote-phone-native-offer".into()), expected_revision: Some(0),
    }, json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"remote-phone-host-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})).await.unwrap();
    wait_native_operation(
        &f,
        &audio_config.credential,
        &room.id,
        "remote-phone-native-offer",
    )
    .await;
    assert_eq!(
        audio
            .subscribe_audio(
                "remote-phone-select",
                1,
                vec![serde_json::from_value(json!({"strm_id":"telephone-audio"})).unwrap()]
            )
            .await
            .unwrap(),
        2
    );
    let wave = std::fs::read(dir.join("host-send.pcmu")).unwrap();
    assert_eq!(wave.len(), 200 * 160);
    std::fs::write(dir.join("media-go"), b"go\n").unwrap();
    let mut received = Vec::new();
    let mut cadence = tokio::time::interval(Duration::from_millis(20));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    for frame in wave.chunks_exact(160) {
        cadence.tick().await;
        audio.send_pcmu(frame).unwrap();
        if let Ok(result) =
            tokio::time::timeout(Duration::from_millis(10), audio.receive_pcmu()).await
        {
            received.extend(result.unwrap());
        }
    }
    std::fs::write(dir.join("host-received.pcmu"), &received).unwrap();
    assert!(received.len() >= 8000, "less than one second decoded input");
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let session = f
        .server
        .store
        .session_view(&actor.credential_id, &room.id, "native-session")
        .unwrap();
    control
        .session_command(connect_protocol::session_control::Request {
            operation_id: "remote-phone-end".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(session.topology_revision),
            action: connect_protocol::session_control::Action::End {
                reason_code: 1000,
                reason: "synthetic rehearsal complete".into(),
            },
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if audio.receive_signaling().await.unwrap().kind() == "session.ended" {
                break;
            }
        }
    })
    .await
    .expect("paired native Session end");
    audio.shutdown().await.unwrap();
    control.shutdown().await.unwrap();
    until(|| {
        db.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE retired_at IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 1
    })
    .await;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
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
    f.server.shutdown().await.unwrap();
    let released = std::net::UdpSocket::bind(sip).expect("original SIP listener not released");
    drop(released);
    eprintln!(
        "remote-telephone-server-witness {}",
        json!({"algorithm":algorithm,"network_profile":"remote","durable_initial_calls":1,"durable_native_offers":1,"original_invite_replayed":true,"paired_session_end":true,"physical_peers_drained":true,"sip_listener_released":true,"server_shutdown_joined":true,"received_bytes":received.len()})
    );
}
