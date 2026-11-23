//! Genuine signed public B SDK move/return; no opaque Core/native proof is fabricated.
#[path = "../public_video_artifacts.rs"]
mod public_video_artifacts;
use super::replacement_terminal::{browser, target};
use super::*;
use futures_util::FutureExt;
use rusqlite::OptionalExtension;
use rvoip_core::adapter::ConnectionAdapter;
use std::panic::AssertUnwindSafe;
const ORIGINAL: &str = "browser-wire-public-b-abort";

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "explicit installed Chromium and Node, normal private CA, actual post-Cut candidate close/production Discarded and fresh original decoded AV restore"]
async fn canonical_chromium_public_b_actual_post_cut_native_close_discards_then_restores_original_decoded_av(
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    assert!(matches!(std::env::consts::OS, "linux" | "macos"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    // This exact ignored test owns its process in the media diagnostic lane.
    // Limit global cross-task capture to cold OLD restore events, never media packets.
    let restore_trace_installed = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "off,connect_service::canonical_replacement::old_video_restore=debug,connect_service::original_replacement_terminal=debug",
        ))
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init()
        .is_ok();
    eprintln!("old-video-restore-diagnostic subscriber_installed={restore_trace_installed}");
    let mut f =
        Fixture::with_extra_scopes("EdDSA", 12, true, true, true, &["connection.replace"]).await;
    let mut retained_artifacts = public_video_artifacts::Capture::new(
        &root,
        f.temp.path().join("chromium-original-public-b-abort"),
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
    let directory = f.temp.path().join("chromium-original-public-b-abort");
    std::fs::create_dir(&directory).unwrap();
    let (mut chromium, debug_port) = browser(&root, &directory, &ca).await;
    retained_artifacts
        .browser_started(&mut chromium.0)
        .await
        .expect("actual original Chromium PID/PGID captured while live");
    let spec = json!({"format":"connect/chromium-original-public-b-abort-integration/1","root":root,
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
        .arg(root.join("scripts/video/browser-public-video-abort.mjs"))
        .stdin(std::fs::File::open(input).unwrap())
        .stdout(std::fs::File::create(directory.join("browser-result.json")).unwrap())
        .stderr(std::fs::File::create(directory.join("runner.log")).unwrap())
        .kill_on_drop(true);
    let mut runner = command.spawn().unwrap();

    let observed=AssertUnwindSafe(async {
        let operation_id=format!("{ORIGINAL}-0-0");let mut cut_checked=false;let mut discarded_checked=false;let mut saved_cut:Option<Value>=None;
        let report=tokio::time::timeout(Duration::from_secs(45),async {
            loop {
                let stage="before-public-ready-0-0";let event=directory.join(format!("{stage}.json"));
                if !cut_checked && event.is_file() {
                    let bytes=std::fs::read(event).unwrap();assert!(bytes.len()<65536);let proof:Value=serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(proof["operation_id"],operation_id);
                    let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                    let(record,candidate,state):(String,String,String)=db.query_row("SELECT n.operation_record_id,n.candidate_connection_id,o.state FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.operation_id=?1",[&operation_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
                    assert_eq!(record,proof["record_id"]);assert_eq!(candidate,proof["connection_id"]);assert_eq!(state,"preparing");
                    let raw:String=db.query_row("SELECT phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='cut-bound-post-f'",[&record],|row|row.get(0)).unwrap();let cut:Value=serde_json::from_str(&raw).unwrap();assert_eq!(cut,proof["phase"]);
                    assert!(proof["native"]["decoded_callbacks"].as_u64().unwrap()>0);
                    let started:u64=db.query_row("SELECT (SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')+(SELECT count(*) FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase IN ('decoded-ready','activated'))",[&record],|row|row.get(0)).unwrap();assert_eq!(started,0);saved_cut=Some(cut);
                    drop(db);let pending=directory.join(format!("{stage}.ack.pending"));init::private_write(&pending,&serde_json::to_vec(&json!({"stage":stage,"operation_id":operation_id})).unwrap()).unwrap();std::fs::rename(pending,directory.join(format!("{stage}.ack.json"))).unwrap();cut_checked=true;
                }
                let stage="abort-candidate-closed";let event=directory.join(format!("{stage}.json"));
                if !discarded_checked && event.is_file() {
                    assert!(cut_checked);let bytes=std::fs::read(event).unwrap();assert!(bytes.len()<65536);let proof:Value=serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(proof["operation_id"],operation_id);assert_eq!(proof["close"]["only_native_peer_closed"],true);assert_eq!(proof["close"]["connection_state"],"connected");
                    let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                    let(record,candidate):(String,String)=db.query_row("SELECT n.operation_record_id,n.candidate_connection_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.operation_id=?1",[&operation_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();assert_eq!(record,proof["record_id"]);assert_eq!(candidate,proof["connection_id"]);
                    let no_commit:u64=db.query_row("SELECT (SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')+(SELECT count(*) FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase IN ('decoded-ready','activated'))",[&record],|row|row.get(0)).unwrap();assert_eq!(no_commit,0);
                    let discarded:Option<String>=db.query_row("SELECT phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='discarded'",[&record],|row|row.get(0)).optional().unwrap();
                    if let Some(raw)=discarded {let value:Value=serde_json::from_str(&raw).unwrap();assert_eq!(value["phase"],"discarded");assert_eq!(value["source_cut_id"],saved_cut.as_ref().unwrap()["source_cut_id"]);assert_eq!(value["binding"],saved_cut.as_ref().unwrap()["binding"]);drop(db);let pending=directory.join(format!("{stage}.ack.pending"));init::private_write(&pending,&serde_json::to_vec(&json!({"stage":stage,"operation_id":operation_id})).unwrap()).unwrap();std::fs::rename(pending,directory.join(format!("{stage}.ack.json"))).unwrap();discarded_checked=true;}
                }
                let final_event=directory.join("abort-complete.json");
                if final_event.is_file() {assert!(cut_checked&&discarded_checked);let bytes=std::fs::read(final_event).unwrap();assert!(bytes.len()<65536);let report:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(report["operation_id"],operation_id);assert_eq!(report["public_ready_sent"],false);assert_eq!(report["local_wait_cancel_after_durable_discarded"],true);break report;}
                assert!(runner.try_wait().unwrap().is_none(),"original abort browser runner exited before actual proof");tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("bounded real Cut/native-close/Discarded/fresh OLD output");
        let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        for index in 0..4 {let(state,generation):(String,u64)=db.query_row("SELECT state,generation FROM connections WHERE id=?1",[format!("browser-wire-connection-{index}")],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();assert_eq!((state.as_str(),generation),("ready",1));}
        let phases:Vec<String>={let mut statement=db.prepare("SELECT p.phase FROM native_replacement_public_video_phases p JOIN operations o ON o.record_id=p.operation_record_id WHERE o.operation_id=?1 ORDER BY p.ordinal").unwrap();let rows=statement.query_map([&operation_id],|row|row.get(0)).unwrap();rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()};
        assert_eq!(phases,vec!["negotiated-never-fed","cut-bound-post-f","discarded"]);
        let commits:u64=db.query_row("SELECT count(*) FROM native_replacement_steps s JOIN operations o ON o.record_id=s.operation_record_id WHERE o.operation_id=?1 AND s.phase='commit'",[&operation_id],|row|row.get(0)).unwrap();assert_eq!(commits,0);
        let ended:u64=db.query_row("SELECT count(*) FROM operations WHERE command_type='connection.end'",[],|row|row.get(0)).unwrap();assert_eq!(ended,0,"no application end substitutes for actual native close");
        // Fresh reservation metadata owns this candidate ID; a never-published
        // replacement has no logical Connection to retire. Bind absence to the
        // SAME retained operation/reservation, never an arbitrary missing ID.
        let (record,candidate):(String,String)=db.query_row("SELECT n.operation_record_id,n.candidate_connection_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.operation_id=?1 AND o.command_type='connection.replace'",[&operation_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(report["record_id"].as_str(),Some(record.as_str()));
        assert_eq!(report["candidate"].as_str(),Some(candidate.as_str()));
        assert_eq!(report["close"]["record_id"].as_str(),Some(record.as_str()));
        assert_eq!(report["close"]["connection_id"].as_str(),Some(candidate.as_str()));
        let unpublished:[u64;8]=db.query_row("SELECT (SELECT count(*) FROM connections WHERE id=?2),(SELECT count(*) FROM connection_offers WHERE connection_id=?2),(SELECT count(*) FROM native_connection_lifetimes WHERE connection_id=?2 OR operation_record_id=?1),(SELECT count(*) FROM native_replacement_lifetimes WHERE operation_record_id=?1),(SELECT count(*) FROM native_replacement_candidates WHERE operation_record_id=?1 OR candidate_connection_id=?2),(SELECT count(*) FROM native_replacement_audiences WHERE operation_record_id=?1 AND phase='commit'),(SELECT count(*) FROM operation_effects WHERE operation_record_id=?1 AND provider='native-replacement-commit'),(SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind IN ('connection_ready','connection_retired') AND json_extract(data_json,'$.connection_id')=?2)",rusqlite::params![record,candidate],|row|Ok([row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?])).unwrap();
        assert_eq!(unpublished,[0;8],"fresh discarded candidate must remain unmaterialized without offer, lifetime, claimed-slot, commit or lifecycle publication");
        for sid in &sessions {let state:String=db.query_row("SELECT state FROM sessions WHERE id=?1",[sid],|row|row.get(0)).unwrap();assert_eq!(state,"accepted");}
        assert_eq!(host.adapter().route_count(),4);assert_eq!(host.available_slots(),2);drop(db);
        let stage="abort-complete";let pending=directory.join(format!("{stage}.ack.pending"));init::private_write(&pending,&serde_json::to_vec(&json!({"stage":stage,"operation_id":operation_id})).unwrap()).unwrap();std::fs::rename(pending,directory.join(format!("{stage}.ack.json"))).unwrap();
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
    eprintln!("chromium-public-b-abort verified actual_post_cut_native_close=true production_discarded=true commit_and_activated=0 fresh_old_decoded_av=true original_camera_audio_live=true original_owners_joined=true");
}
