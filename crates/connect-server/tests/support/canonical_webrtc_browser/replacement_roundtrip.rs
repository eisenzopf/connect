//! Genuine public SDK move-and-return through the same original Devices.
use super::replacement_terminal::{browser, target};
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
use futures_util::FutureExt;
use rvoip_core::adapter::ConnectionAdapter;
use std::panic::AssertUnwindSafe;
const ORIGINAL: &str = "browser-wire-roundtrip";

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "explicit installed Chromium and Node, normal private CA, genuine two-Session AV moves and original-Device returns"]
async fn canonical_chromium_two_session_av_moves_and_returns_reuse_original_device_at_generation_three(
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    assert!(matches!(std::env::consts::OS, "linux" | "macos"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut f =
        Fixture::with_extra_scopes("EdDSA", 12, true, true, true, &["connection.replace"]).await;
    let ca = certificate(&mut f);
    f.server.shutdown().await.unwrap();
    let asset = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let asset_port = asset.local_addr().unwrap().port();
    f.config.websocket = Some(
        connect_server::canonical_websocket::CanonicalWebSocketConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            mount_path: "/connect".into(),
            allowed_origins: vec![format!("http://127.0.0.1:{asset_port}")],
            advertised_url: None,
        },
    );
    f.config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec![format!("http://127.0.0.1:{asset_port}")],
            advertised_url: None,
        },
    );
    f.config.webrtc = Some(connect_server::canonical_webrtc::CanonicalWebRtcConfig {
        profile: connect_server::canonical_webrtc::CanonicalWebRtcProfile::OpusVp8,
        max_peers: 6,
        addressing: None,
    });
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let host = f.server.test_webrtc_host().unwrap();
    let other = canonical_webtransport_media::other_caps_with_end(
        &f,
        vec!["audio".into(), "video".into()],
        true,
    );
    let target = target(&f).await;
    for (index, config) in [&f.client, &other].into_iter().enumerate() {
        enroll(&f, config, 710 + index).await;
    }
    let target_endpoint: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT id FROM endpoints WHERE device_id=?1 AND transport='webrtc'",
        [&target.device_id],
        |row| row.get(0),
    )
    .unwrap();
    let original_endpoint: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT id FROM endpoints WHERE device_id=?1 AND transport='webrtc'",
        [&f.client.device_id],
        |row| row.get(0),
    )
    .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut create =
        connect_client::conversations::CreateRequest::persistent(vec![other.identity_id.clone()]);
    create.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("browser-wire-room", create)
        .await
        .unwrap();
    let theirs = room
        .participants
        .iter()
        .find(|p| p.identity_id == other.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let mine = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let mut http = canonical_http_config(&f);
    http.credential = other.credential.clone();
    let mut other_http = CanonicalHttpClient::open(http).await.unwrap();
    let sessions = ["browser-wire-session-0", "browser-wire-session-1"];
    for (index, sid) in sessions.iter().enumerate() {
        let caps = json!({"audio":["opus"],"video":["vp8"]});
        native
            .session_command(Request {
                operation_id: format!("browser-wire-invite-{index}"),
                conversation_id: room.id.clone(),
                session_id: (*sid).into(),
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
        other_http
            .session_command(Request {
                operation_id: format!("browser-wire-accept-{index}"),
                conversation_id: room.id.clone(),
                session_id: (*sid).into(),
                by_participant: None,
                expected_revision: Some(0),
                action: Action::Accept { capabilities: caps },
            })
            .await
            .unwrap();
    }
    native.shutdown().await.unwrap();
    let directory = f.temp.path().join("chromium-original-roundtrip");
    std::fs::create_dir(&directory).unwrap();
    let (mut chromium, debug_port) = browser(&root, &directory, &ca).await;
    let spec = json!({"format":"connect/chromium-original-roundtrip-integration/1","root":root,
        "privateDirectory":directory,"assetPort":asset_port,"debugPort":debug_port,
        "operationId":ORIGINAL,"conversationId":room.id,"sessions":sessions,"targetEndpoint":target_endpoint,"originalEndpoint":original_endpoint,
        "actors":[participant(&f,&f.client,&room.id,&mine,&theirs,0),
            participant(&f,&other,&room.id,&theirs,&mine,1),participant(&f,&target,&room.id,&mine,&theirs,0)]});
    let input = directory.join("private-input.json");
    init::private_write(&input, &serde_json::to_vec(&spec).unwrap()).unwrap();
    drop(asset);
    let mut command = tokio::process::Command::new(
        std::env::var_os("CONNECT_GATE6_NODE").expect("designated installed Node"),
    );
    command
        .arg(root.join("scripts/video/browser-replacement-roundtrip.mjs"))
        .stdin(std::fs::File::open(input).unwrap())
        .stdout(std::fs::File::create(directory.join("browser-result.json")).unwrap())
        .stderr(std::fs::File::create(directory.join("runner.log")).unwrap())
        .kill_on_drop(true);
    let mut runner = command.spawn().unwrap();

    let observed=AssertUnwindSafe(async {
        let complete=tokio::time::timeout(Duration::from_secs(100),async {
            loop {
                let path=directory.join("roundtrip-complete.json");
                if path.is_file() {
                    let bytes=std::fs::read(path).unwrap();assert!(bytes.len()<65536);
                    let report:Value=serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(report["operation_id"],ORIGINAL);break report;
                }
                assert!(runner.try_wait().unwrap().is_none(),"original positive browser runner exited before proof");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("bounded original roundtrip browser report");
        let moves=complete["moves"].as_array().unwrap();assert_eq!(moves.len(),4);
        let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let source_credentials=[f.server.store.authenticate(&f.client.credential).unwrap().credential_id,
            f.server.store.authenticate(&target.credential).unwrap().credential_id];
        for phase in 0..2 {for sid in 0..2 {
            let operation_id=format!("{ORIGINAL}-{phase}-{sid}");
            let proof=moves.iter().find(|row|row["operation_id"]==operation_id).unwrap();
            let root=f.server.store.execute_operation_action(&source_credentials[phase],OperationAction::Get{operation_id:operation_id.clone()}).unwrap();
            assert_eq!(root.state,OperationState::Completed);assert_eq!(root.record_id,proof["record_id"]);
            assert!(root.effects.iter().any(|effect|effect.provider=="native-replacement-commit"&&effect.state==connect_protocol::transitions::EffectState::Confirmed));
            let (source,candidate,generation):(String,String,u64)=db.query_row(
                "SELECT o.connection_id,n.candidate_connection_id,n.generation FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1",[&root.record_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
            assert_eq!(source,proof["source"]);assert_eq!(candidate,proof["target"]);assert_eq!(generation,phase as u64+2);
            let (source_state,source_generation):(String,u64)=db.query_row("SELECT state,generation FROM connections WHERE id=?1",[&source],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(source_state,"retired");assert_eq!(source_generation,phase as u64+2);
            let (candidate_state,actual_generation,endpoint,device):(String,u64,String,String)=db.query_row(
                "SELECT c.state,c.generation,c.endpoint_id,e.device_id FROM connections c JOIN endpoints e ON e.id=c.endpoint_id WHERE c.id=?1",[&candidate],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
            // The outbound candidate is itself retired by the completed return;
            // its immutable first reservation stays2 but current row becomes3.
            assert_eq!(actual_generation,3);
            assert_eq!(candidate_state,if phase==0 {"retired"}else{"ready"});
            assert_eq!(endpoint,if phase==0 {target_endpoint.clone()}else{original_endpoint.clone()});
            assert_eq!(device,if phase==0 {target.device_id.clone()}else{f.client.device_id.clone()});
            assert_eq!(proof["generation"],phase as u64+2);assert_eq!(proof["readiness"]["video"]["decoder_startup"],"fresh-peer-first-decoded-frame");
            assert!(proof["readiness"]["audio"]["decoded_samples"].as_u64().unwrap()>=960);
            assert!(proof["readiness"]["video"]["decoded_keyframes"].as_u64().unwrap()>0);
        }}
        for index in [1,3] {let (state,generation):(String,u64)=db.query_row("SELECT state,generation FROM connections WHERE id=?1",[format!("browser-wire-connection-{index}")],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();assert_eq!((state.as_str(),generation),("ready",1));}
        for sid in &sessions {let state:String=db.query_row("SELECT state FROM sessions WHERE id=?1",[sid],|row|row.get(0)).unwrap();assert_eq!(state,"accepted");}
        assert_eq!(host.adapter().route_count(),4);assert_eq!(host.available_slots(),2);
        drop(db);
        let pending=directory.join("roundtrip-complete.ack.pending");
        init::private_write(&pending,&serde_json::to_vec(&json!({"stage":"roundtrip-complete","operation_id":ORIGINAL})).unwrap()).unwrap();
        std::fs::rename(pending,directory.join("roundtrip-complete.ack.json")).unwrap();
    }).catch_unwind().await;
    let result = tokio::time::timeout(Duration::from_secs(20), runner.wait()).await;
    let mut runner_forced = false;
    let status = match result {
        Ok(value) => value.unwrap(),
        Err(_) => {
            runner_forced = true;
            runner.kill().await.unwrap();
            runner.wait().await.unwrap()
        }
    };
    let browser_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = chromium.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let browser_forced = browser_status.is_err();
    if browser_forced {
        let _ = chromium.0.kill();
        let _ = chromium.0.wait();
    }
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if host.adapter().route_count() == 0 && host.available_slots() == 6 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let shutdown = f.server.shutdown().await;
    if let Err(panic) = observed {
        std::panic::resume_unwind(panic);
    }
    assert!(
        !runner_forced && status.success(),
        "original positive Node runner must join naturally"
    );
    assert!(
        !browser_forced && browser_status.unwrap().success(),
        "original positive Chromium must join naturally"
    );
    assert!(drained.is_ok(), "all original RTC routes and slots drain");
    shutdown.unwrap();
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("browser-result.json")).unwrap())
            .unwrap();
    assert_eq!(report["passed"], true);
    eprintln!("chromium-roundtrip verified original_shared_controls=true sessions=2 original_peers=4 moves=4 target_generations=2,3 original_device_reused=true real_private_decoded_audio_keyframes=true final_public_decoders=4 forced=false");
}
