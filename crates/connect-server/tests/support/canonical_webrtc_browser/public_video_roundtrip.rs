//! Genuine signed public B SDK move/return; no opaque Core/native proof is fabricated.
#[path = "../public_video_artifacts.rs"]
mod public_video_artifacts;
use super::replacement_terminal::{browser, target};
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
use futures_util::FutureExt;
use rvoip_core::adapter::ConnectionAdapter;
use std::panic::AssertUnwindSafe;
const ORIGINAL: &str = "browser-wire-public-b-roundtrip";

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "explicit installed Chromium and Node, normal private CA, genuine two-Session AV moves and original-Device returns"]
async fn canonical_chromium_public_b_two_session_moves_and_returns_adopt_same_warmed_receiver_and_audio(
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    assert!(matches!(std::env::consts::OS, "linux" | "macos"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut f =
        Fixture::with_extra_scopes("EdDSA", 12, true, true, true, &["connection.replace"]).await;
    let mut retained_artifacts = public_video_artifacts::Capture::new(
        &root,
        f.temp.path().join("chromium-original-public-b-roundtrip"),
        f.config.database.clone(),
    );
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
    let directory = f.temp.path().join("chromium-original-public-b-roundtrip");
    std::fs::create_dir(&directory).unwrap();
    let (mut chromium, debug_port) = browser(&root, &directory, &ca).await;
    retained_artifacts
        .browser_started(&mut chromium.0)
        .await
        .expect("actual original Chromium PID/PGID captured while live");
    let spec = json!({"format":"connect/chromium-original-public-b-roundtrip-integration/1","root":root,
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
        .arg(root.join("scripts/video/browser-public-video-roundtrip.mjs"))
        .stdin(std::fs::File::open(input).unwrap())
        .stdout(std::fs::File::create(directory.join("browser-result.json")).unwrap())
        .stderr(std::fs::File::create(directory.join("runner.log")).unwrap())
        .kill_on_drop(true);
    let mut runner = command.spawn().unwrap();

    // These original owners live OUTSIDE the caught future: every failure
    // releases and joins actual SQL exclusion before native/server teardown.
    let mut sql_release = None;
    let mut sql_owner = None;
    let mut decoded_probe = None;
    let observed=AssertUnwindSafe(async {
        let mut checked = std::collections::BTreeSet::new();
        let complete=tokio::time::timeout(Duration::from_secs(100),async {
            loop {
                // Fixture checkpoints only delay the SAME actual original
                // signed public Ready send; every command keeps its own8s cutoff.
                for phase in 0..2 {for sid in 0..2 {
                    let stage=format!("before-public-ready-{phase}-{sid}");
                    let checkpoint=directory.join(format!("{stage}.json"));
                    if checked.contains(&stage) || !checkpoint.is_file() {continue;}
                    let bytes=std::fs::read(checkpoint).unwrap();assert!(bytes.len()<65536);
                    let proof:Value=serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(proof["operation_id"],format!("{ORIGINAL}-{phase}-{sid}"));
                    let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                    let(record,candidate,state):(String,String,String)=db.query_row(
                        "SELECT n.operation_record_id,n.candidate_connection_id,o.state FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.operation_id=?1",[format!("{ORIGINAL}-{phase}-{sid}")],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
                    assert_eq!(record,proof["record_id"]);assert_eq!(candidate,proof["connection_id"]);assert_eq!(state,"preparing");
                    let cut:String=db.query_row("SELECT phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='cut-bound-post-f'",[&record],|row|row.get(0)).unwrap();
                    let cut:Value=serde_json::from_str(&cut).unwrap();assert_eq!(cut,proof["phase"]);
                    let started:u64=db.query_row("SELECT (SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')+(SELECT count(*) FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase IN ('decoded-ready','activated'))",[&record],|row|row.get(0)).unwrap();
                    assert_eq!(started,0,"real B callback/marker must precede any Commit/DecodedReady/Activated");
                    assert_eq!(proof["native"]["decoded_callbacks"].as_u64().unwrap()>0,true);
                    assert_eq!(proof["native"]["mid"],cut["binding"]["mid"]);
                    drop(db);
                    if phase==0 && sid==0 {
                        decoded_probe=Some(f.server.service.arm_canonical_public_video_reader_release_probe_for_test(&format!("{ORIGINAL}-{phase}-{sid}")).unwrap());
                    }
                    let ack=directory.join(format!("{stage}.ack.pending"));
                    init::private_write(&ack,&serde_json::to_vec(&json!({"stage":stage,"operation_id":proof["operation_id"]})).unwrap()).unwrap();
                    std::fs::rename(ack,directory.join(format!("{stage}.ack.json"))).unwrap();
                    if phase==0 && sid==0 {
                        let probe=decoded_probe.as_ref().unwrap();
                        let coordination=tokio::time::Instant::now()+Duration::from_millis(750);
                        tokio::time::timeout_at(coordination,probe.capture_entered()).await.expect("actual first signed Ready capture retains originating Readers");
                        let original=probe.captured_ready().expect("actual SAME immutable signed B Ready");
                        assert!(probe.ready_present());assert_eq!(probe.readers_released(),Some(false));
                        assert_eq!(probe.observed(),(0,0,0,0),"Ready value alone must not enter phase metadata while originating Readers remain");
                        let capture_until=coordination.min(probe.capture_deadline().unwrap()).min(probe.original_deadline().unwrap());
                        tokio::time::timeout_at(capture_until,tokio::time::sleep(Duration::from_millis(15))).await.expect("bounded scheduling opportunity remains within SAME original capture deadline");
                        assert_eq!(probe.readers_released(),Some(false));assert_eq!(probe.observed(),(0,0,0,0));
                        assert!(probe.captured_ready().as_ref()==Some(&original));
                        probe.release_capture();tokio::time::timeout_at(capture_until,probe.capture_left()).await.unwrap();
                        tokio::time::timeout_at(coordination,probe.entered()).await.expect("actual originating signed Readers released before metadata gate");
                        assert_eq!(probe.readers_released(),Some(true),"actual originating signed Readers released, not merely notified");
                        assert!(probe.ready_present());assert_eq!(probe.operation_id(),Some(format!("{ORIGINAL}-{phase}-{sid}").as_str()));assert_eq!(probe.operation_record_id(),Some(record.as_str()));
                        assert_eq!(serde_json::to_value(&original).unwrap()["public_video_readiness"]["binding"],cut["binding"]);
                        assert_eq!(probe.observed(),(1,0,0,0));
                        let until=coordination.min(probe.original_deadline().unwrap());
                        let store=f.server.store.clone();let(entered_tx,entered_rx)=tokio::sync::oneshot::channel::<()>();let(release_tx,release_rx)=tokio::sync::oneshot::channel::<()>();
                        sql_release=Some(release_tx);
                        sql_owner=Some(tokio::task::spawn_blocking(move ||store.with_primary_sql_held_for_test(|| {let _=entered_tx.send(());let _=release_rx.blocking_recv();})));
                        tokio::time::timeout_at(until,entered_rx).await.unwrap().unwrap();
                        probe.release();tokio::time::timeout_at(until,probe.left()).await.unwrap();
                        tokio::time::timeout_at(until,async {loop {let counts=probe.observed();assert_eq!(counts.0,1);assert_eq!(counts.3,0);assert!(probe.captured_ready().as_ref()==Some(&original));if counts.2>0 {assert!(counts.1>=counts.2);break;}tokio::time::sleep(Duration::from_millis(1)).await;}}).await.expect("actual producer authority-busy while SAME Ready remains captured under real SQL mutex");
                        sql_release.take().unwrap().send(()).unwrap();sql_owner.take().unwrap().await.unwrap().unwrap();
                        assert!(probe.captured_ready().as_ref()==Some(&original));
                    }
                    checked.insert(stage);
                }}
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
        assert_eq!(checked.len(),4,"every real public Ready checkpoint reached before publication");
        let moves=complete["moves"].as_array().unwrap();assert_eq!(moves.len(),4);
        let probe=decoded_probe.as_ref().unwrap();let counts=probe.observed();
        assert_eq!(counts.0,1);assert!(counts.1>=2 && counts.2>=1);assert_eq!(counts.3,1,"exact same Ready's Decoded phase has one durable success");
        let guarded=serde_json::to_value(probe.captured_ready().unwrap()).unwrap();
        let guarded_move=moves.iter().find(|row|row["operation_id"]==format!("{ORIGINAL}-0-0")).unwrap();assert_eq!(guarded_move["public_ready"],guarded);
        assert_eq!(moves.iter().filter(|row|row["post_take_native_fault_same_lease_retry"]==true).count(),1,"one actual post-take native API failure must return the SAME B lease for successful public retry");
        let mut reservations=std::collections::BTreeSet::new();
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
            assert!(proof["readiness"]["video"]["decoded_keyframes"].is_null() || proof["readiness"]["video"]["decoded_keyframes"].as_u64().unwrap()>0);
            let mut statement=db.prepare("SELECT phase,phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 ORDER BY ordinal").unwrap();
            let phases:Vec<(String,String)>=statement.query_map([&root.record_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().map(Result::unwrap).collect();
            assert_eq!(phases.iter().map(|row|row.0.as_str()).collect::<Vec<_>>(),vec!["negotiated-never-fed","cut-bound-post-f","decoded-ready","activated"]);
            let activated:Value=serde_json::from_str(&phases[3].1).unwrap();
            assert_eq!(activated,proof["activated"]);assert_eq!(activated["target_generation"],phase as u64+2);
            assert_eq!(activated["binding"],proof["public_ready"]["public_video_readiness"]["binding"]);
            assert_eq!(activated["source_cut_id"],proof["public_ready"]["public_video_readiness"]["source_cut_id"]);
            assert!(reservations.insert(activated["reservation_id"].as_str().unwrap().to_owned()),"new original Root cannot borrow another Root's B reservation");
            assert_eq!(proof["same_warmed_element"],true);assert_eq!(proof["same_native_receiver"],true);
        }}
        for index in [1,3] {let (state,generation):(String,u64)=db.query_row("SELECT state,generation FROM connections WHERE id=?1",[format!("browser-wire-connection-{index}")],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();assert_eq!((state.as_str(),generation),("ready",1));}
        for sid in &sessions {let state:String=db.query_row("SELECT state FROM sessions WHERE id=?1",[sid],|row|row.get(0)).unwrap();assert_eq!(state,"accepted");}
        assert_eq!(host.adapter().route_count(),4);assert_eq!(host.available_slots(),2);
        drop(db);
        let pending=directory.join("roundtrip-complete.ack.pending");
        init::private_write(&pending,&serde_json::to_vec(&json!({"stage":"roundtrip-complete","operation_id":ORIGINAL})).unwrap()).unwrap();
        std::fs::rename(pending,directory.join("roundtrip-complete.ack.json")).unwrap();
    }).catch_unwind().await;
    if let Some(release) = sql_release.take() {
        let _ = release.send(());
    }
    if let Some(probe) = decoded_probe.as_ref() {
        probe.release_capture();
        probe.release();
    }
    let sql_cleanup = if let Some(owner) = sql_owner.take() {
        matches!(owner.await, Ok(Ok(())))
    } else {
        true
    };
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
    // SAME original five-second cleanup interval; it is not renewed for group checks.
    let (original_child, disarmed) = chromium.group_cleanup_parts();
    let browser_join = retained_artifacts
        .join_browser(original_child, disarmed)
        .await;
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
    let artifact_result = retained_artifacts.save("after-original-cleanup-attempts");
    assert!(
        sql_cleanup,
        "original real SQL exclusion worker must join before verdict escapes"
    );
    if let Err(panic) = observed {
        std::panic::resume_unwind(panic);
    }
    assert!(
        !runner_forced && status.success(),
        "original positive Node runner must join naturally"
    );
    assert!(
        browser_join.is_ok(),
        "original positive Chromium must join naturally"
    );
    assert!(drained.is_ok(), "all original RTC routes and slots drain");
    shutdown.unwrap();
    artifact_result.expect("actual private B proof files retained");
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("browser-result.json")).unwrap())
            .unwrap();
    assert_eq!(report["passed"], true);
    eprintln!("chromium-public-b-roundtrip verified original_shared_controls=true sessions=2 original_peers=4 moves=4 target_generations=2,3 original_device_reused=true real_private_A_audio=true same_warmed_public_B=true actual_prepublication_B_callbacks=true final_public_decoders=4 forced=false");
}
