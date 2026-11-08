//! A genuine Chromium encrypted terminal alert across the full Service path.
//! No adapter.end, injected state, Store mutation or WSS close creates the event.
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
use futures_util::FutureExt;
use rvoip_core::adapter::ConnectionAdapter;
use std::panic::AssertUnwindSafe;
const ORIGINAL: &str = "browser-wire-original-move";

pub(super) async fn target(f: &Fixture) -> CanonicalClientConfig {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "connection.replace".into(),
        "connection.end".into(),
        "stream.subscribe".into(),
    ]);
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut target = f.client.clone();
    target.credential = key.token;
    target.device_id = "browser-wire-target-device".into();
    target.signing_key_id = "browser-wire-target-key".into();
    target.signing_private_key = f.temp.path().join("browser-wire-target.der");
    let public = signing(&target.signing_private_key, &target.signing_algorithm);
    enroll(f, &target, 712).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    f.server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::SigningKey {
                operation_id: "browser-wire-target-signing".into(),
                identity_id: target.identity_id.clone(),
                credential_id: key.id,
                device_id: target.device_id.clone(),
                key_id: target.signing_key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    target
}
fn acknowledge(directory: &Path, stage: &str) {
    let pending = directory.join(format!("{stage}.ack.pending"));
    init::private_write(
        &pending,
        &serde_json::to_vec(&json!({"stage":stage,"operation_id":ORIGINAL})).unwrap(),
    )
    .unwrap();
    std::fs::rename(pending, directory.join(format!("{stage}.ack.json"))).unwrap();
}

async fn checkpoint(
    directory: &Path,
    stage: &str,
    runner: &mut tokio::process::Child,
    cutoff: tokio::time::Instant,
) -> Value {
    tokio::time::timeout_at(cutoff, async {
        loop {
            let path = directory.join(format!("{stage}.json"));
            if path.is_file() {
                let bytes = std::fs::read(path).unwrap();
                assert!(bytes.len() < 16384);
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["operation_id"], ORIGINAL);
                return value;
            }
            assert!(
                runner.try_wait().unwrap().is_none(),
                "original browser runner exited before {stage}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("original browser checkpoint cutoff phase={stage}"))
}
pub(super) async fn browser(root: &Path, directory: &Path, ca: &Path) -> (OwnedBrowser, u16) {
    let binary = PathBuf::from(
        std::env::var_os("CONNECT_GATE6_CHROME").expect("designated installed Chromium"),
    );
    let profile = directory.join("normal-trust-profile");
    let imported = Command::new("python3")
        .arg(root.join("scripts/video/browser_profile.py"))
        .arg("chrome")
        .arg(&binary)
        .arg(&profile)
        .arg(ca)
        .output()
        .unwrap();
    assert!(imported.status.success(), "normal private CA import failed");
    init::private_write(&directory.join("trust-import.json"), &imported.stdout).unwrap();
    let log = directory.join("chrome.log");
    let mut command = Command::new(binary);
    command.args([
        "--headless=new",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-sync",
        "--disable-breakpad",
        "--disable-crash-reporter",
        "--autoplay-policy=no-user-gesture-required",
        "--disable-background-timer-throttling",
        "--disable-renderer-backgrounding",
    ]);
    if cfg!(target_os = "linux")
        && std::env::var("CONNECT_GATE6_OWNED_BROWSER_CONTAINER").as_deref() == Ok("1")
    {
        command.args(["--no-sandbox", "--allow-loopback-in-peer-connection"]);
    }
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut owner = OwnedBrowser(command.spawn().unwrap());
    let port = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
                if let Some(port) = text.lines().next().and_then(|s| s.parse::<u16>().ok()) {
                    return port;
                }
            }
            assert!(
                owner.0.try_wait().unwrap().is_none(),
                "original Chromium startup owner exited"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("original installed Chromium startup cutoff");
    (owner, port)
}
fn original_root(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    let credential = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    f.server
        .store
        .execute_operation_action(
            &credential,
            OperationAction::Get {
                operation_id: ORIGINAL.into(),
            },
        )
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "explicit installed Chromium and Node, private normal CA, synthetic AV, actual encrypted close with original WSS retained"]
async fn canonical_chromium_connected_candidate_close_notifies_exact_pending_root_with_control_and_other_session_live(
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
        max_peers: 5,
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
    let directory = f.temp.path().join("chromium-original-terminal");
    std::fs::create_dir(&directory).unwrap();
    let (mut chromium, debug_port) = browser(&root, &directory, &ca).await;
    let spec = json!({"format":"connect/chromium-original-terminal-integration/1","root":root,
        "privateDirectory":directory,"assetPort":asset_port,"debugPort":debug_port,
        "operationId":ORIGINAL,"conversationId":room.id,"sessions":sessions,"targetEndpoint":target_endpoint,
        "actors":[participant(&f,&f.client,&room.id,&mine,&theirs,0),
            participant(&f,&other,&room.id,&theirs,&mine,1),participant(&f,&target,&room.id,&mine,&theirs,0)]});
    let input = directory.join("private-input.json");
    init::private_write(&input, &serde_json::to_vec(&spec).unwrap()).unwrap();
    drop(asset);
    let mut command = tokio::process::Command::new(
        std::env::var_os("CONNECT_GATE6_NODE").expect("designated installed Node"),
    );
    command
        .arg(root.join("scripts/video/browser-replacement-terminal.mjs"))
        .stdin(std::fs::File::open(input).unwrap())
        .stdout(std::fs::File::create(directory.join("browser-result.json")).unwrap())
        .stderr(std::fs::File::create(directory.join("runner.log")).unwrap())
        .kill_on_drop(true);
    let mut runner = command.spawn().unwrap();
    let observed=AssertUnwindSafe(async {
        let waiting=checkpoint(&directory,"candidate-answer",&mut runner,tokio::time::Instant::now()+Duration::from_secs(30)).await;
        assert_eq!(waiting["close_already_attempted"],false);
        let credential=f.server.store.authenticate(&f.client.credential).unwrap().credential_id;
        let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (record,runtime,candidate,generation):(String,String,String,u64)=db.query_row(
            "SELECT n.operation_record_id,n.candidate_runtime_id,n.candidate_connection_id,n.generation FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.operation_id=?1 AND n.credential_id=?2",
            rusqlite::params![ORIGINAL,credential],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(waiting["connection_id"],candidate);assert_eq!(generation,2);
        let id=rvoip_core::ids::ConnectionId::from_string(runtime);
        // Clone the exact original peer, then drop the map guard before await.
        let peer=host.adapter().original_peer_for_test(&id,tokio::time::Instant::now()+Duration::from_secs(1)).await.expect("exact original allocated target peer");
        let before=original_root(&f);assert_eq!(before.record_id,record);assert_eq!(before.state,OperationState::Preparing);
        assert!(!before.effects.iter().any(|effect|["native-replacement-readiness","native-replacement-commit"].contains(&effect.provider.as_str())&&effect.state==connect_protocol::transitions::EffectState::Confirmed));
        drop(db);acknowledge(&directory,"candidate-answer");
        let closed=checkpoint(&directory,"terminal-observed",&mut runner,tokio::time::Instant::now()+Duration::from_secs(10)).await;
        assert_eq!(closed["record_id"],record);assert_eq!(closed["connection_id"],candidate);assert_eq!(closed["close"]["connection_state"],"connected");
        assert_eq!(closed["close"]["only_native_peer_closed"],true);assert_eq!(closed["wss_epochs_unchanged"],true);
        // The retained Core remains queryable after normal adapter shutdown.
        // Only the typed alert path sets this original transport statistic to
        // Failed; normal local close does not invent that state.
        let stats=peer.peer_connection().get_stats(std::time::Instant::now(),rtc::statistics::StatsSelector::None).await;
        let dtls=stats.transport().expect("original backend transport statistic");
        assert_eq!(dtls.dtls_state,rtc::peer_connection::transport::RTCDtlsTransportState::Failed,
            "actual original server did not observe the encrypted terminal DTLS alert");
        assert!(!host.adapter().is_connection_live(&id));assert_eq!(host.adapter().route_count(),4);assert_eq!(host.available_slots(),1);
        let ended=original_root(&f);assert_eq!(ended.record_id,record);assert!(matches!(ended.state,OperationState::Failed|OperationState::Cancelled));
        assert!(!ended.effects.iter().any(|effect|["native-replacement-readiness","native-replacement-commit"].contains(&effect.provider.as_str())&&effect.state==connect_protocol::transitions::EffectState::Confirmed));
        let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let started_terminal_phases:u64=db.query_row("SELECT (SELECT count(*) FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='readiness' AND started=1)+(SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1)",[&record],|r|r.get(0)).unwrap();assert_eq!(started_terminal_phases,0,"wire close must precede any readiness or commit attempt");
        let end_operations:u64=db.query_row("SELECT count(*) FROM operations WHERE command_type='connection.end'",[],|r|r.get(0)).unwrap();assert_eq!(end_operations,0,"no application end can stand in for a wire alert");
        for index in 0..4 {let (state,generation):(String,u64)=db.query_row("SELECT state,generation FROM connections WHERE id=?1",[format!("browser-wire-connection-{index}")],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();assert_eq!((state.as_str(),generation),("ready",1));}
        for sid in &sessions {let state:String=db.query_row("SELECT state FROM sessions WHERE id=?1",[sid],|r|r.get(0)).unwrap();assert_eq!(state,"accepted");}
        drop(db);drop(peer);acknowledge(&directory,"terminal-observed");
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
    // CDP browser.close must join the original OS owner; only failed cleanup
    // uses its existing guarded kill/wait and cannot qualify this test.
    let mut browser_forced = false;
    let browser_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = chromium.0.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if browser_status.is_err() {
        browser_forced = true;
        let _ = chromium.0.kill();
        let _ = chromium.0.wait();
    }
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if host.adapter().route_count() == 0 && host.available_slots() == 5 {
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
        "original Node runner did not naturally join"
    );
    assert!(
        !browser_forced && browser_status.unwrap().success(),
        "original Chromium did not naturally join"
    );
    assert!(
        drained.is_ok(),
        "original RTC routes and slots did not drain"
    );
    shutdown.unwrap();
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("browser-result.json")).unwrap())
            .unwrap();
    assert_eq!(report["passed"], true);
    eprintln!("chromium-original-terminal verified actual_dtls_failed=true original_root_terminal=true ready_and_commit_confirmed=0 application_connection_end=0 original_wss_retained=true original_old_decoders=4 unrelated_session_live=true runner_forced=false browser_forced=false");
}
