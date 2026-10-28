//! Child-only Mac server and local peer for the owned Linux guest media fixture.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Child-only isolated guest fixture; invoked by the distinct-media runner"]
async fn distinct_network_audio_server_child() {
    let dir = std::path::PathBuf::from(std::env::var_os("CONNECT_G6_DISTINCT_NATIVE_DIR").unwrap())
        .canonicalize()
        .unwrap();
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
    let unreachable = match std::env::var("CONNECT_G6_NATIVE_UNREACHABLE") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value == "1" => true,
        _ => panic!("invalid unreachable-port fixture mode"),
    };
    let mut f = configured_at(&algorithm, true, "0.0.0.0:0".parse().unwrap()).await;
    // A wildcard is a listener address, never a client destination. The guest
    // independently resolves the Mac's designated non-loopback host address.
    f.client.endpoint =
        std::net::SocketAddr::new("127.0.0.1".parse().unwrap(), f.server.address.port());
    ready_endpoint_with_capabilities(&f, vec!["audio".into()]).await;
    let local = party_with_capabilities(&f, &f.peer.identity_id, 906, true, vec!["audio".into()]);
    let mut control = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut control).await;
    control.shutdown().await.unwrap();
    let mut peer = CanonicalClient::open(local.clone()).await.unwrap()
        .prepare_public_native_audio(CommandScope {
            conversation_id: Some(cid.clone()), session_id: Some("native-session".into()),
            connection_id: Some("distinct-media-host".into()),
            operation_id: Some("distinct-media-host-offer".into()), expected_revision: Some(0),
        }, json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"distinct-media-host-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})).await.unwrap();
    wait_native_operation(&f, &local.credential, &cid, "distinct-media-host-offer").await;
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
    client.endpoint =
        std::net::SocketAddr::new("127.0.0.1".parse().unwrap(), f.server.address.port());
    std::fs::write(dir.join("client.toml"), toml::to_string(&client).unwrap()).unwrap();
    std::fs::set_permissions(
        dir.join("client.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::write(
        dir.join("ready-writing.json"),
        serde_json::to_vec(
            &json!({"algorithm":algorithm,"port":f.server.address.port(),"conversation_id":cid}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::rename(dir.join("ready-writing.json"), dir.join("ready.json")).unwrap();
    tokio::time::timeout(Duration::from_secs(90), async {
        while !dir
            .join(if unreachable {
                "guest-refused"
            } else {
                "guest-ready"
            })
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    if unreachable {
        // The unreachable guest submitted no offer. A fresh authorized local
        // control peer must still authenticate and receive its correlated ack.
        let mut health = CanonicalClient::open(f.client.clone()).await.unwrap();
        let id = health.keepalive().await.unwrap();
        reply(&mut health, &id, "ack").await;
        health.shutdown().await.unwrap();
        peer.shutdown().await.unwrap();
        finish(f, &algorithm, 0, 1, true).await;
        return;
    }
    wait_native_operation(&f, &f.client.credential, &cid, "distinct-media-guest-offer").await;
    assert_eq!(
        peer.subscribe_audio(
            "distinct-media-host-select",
            1,
            vec![serde_json::from_value(json!({"strm_id":"distinct-media-guest-voice"})).unwrap()]
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
        peer.send_pcmu(frame).unwrap();
        if let Ok(result) =
            tokio::time::timeout(Duration::from_millis(10), peer.receive_pcmu()).await
        {
            received.extend(result.unwrap());
        }
    }
    std::fs::write(dir.join("host-received.pcmu"), &received).unwrap();
    peer.shutdown().await.unwrap();
    assert!(received.len() >= 8000, "less than one second received");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !dir.join("done").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    finish(f, &algorithm, received.len(), 2, false).await;
}

async fn finish(
    f: Fixture,
    algorithm: &str,
    received_bytes: usize,
    offers: u64,
    unreachable: bool,
) {
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
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        offers
    );
    drop(db);
    eprintln!(
        "distinct-media-server-witness {}",
        json!({"algorithm":algorithm,"sent_frames":if unreachable {0}else{200},"received_bytes":received_bytes,"durable_native_offers":offers,"physical_peers_drained":true,"guest_open_refused":unreachable,"post_timeout_signed_heartbeat":unreachable})
    );
    f.server.shutdown().await.unwrap();
}
