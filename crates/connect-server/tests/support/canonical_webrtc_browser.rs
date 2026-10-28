//! Explicit installed-browser lane: synthetic sources, normal private CA trust.
#[path = "canonical_webrtc_browser/audio_trace.rs"]
mod audio_trace;
#[path = "canonical_webrtc_browser/recovery.rs"]
mod recovery;
#[path = "canonical_webrtc_browser/transitions.rs"]
mod transitions;
use super::*;
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};
use connect_service::canonical_webrtc::Host;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};

struct OwnedBrowser(Child);
impl Drop for OwnedBrowser {
    fn drop(&mut self) {
        let id = self.0.id();
        let _ = self.0.kill();
        #[cfg(unix)]
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{id}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.0.wait();
    }
}
fn certificate(f: &mut Fixture) -> PathBuf {
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose,
        IsCa, KeyPair, KeyUsagePurpose,
    };
    let mut ca = CertificateParams::default();
    let now = std::time::SystemTime::now();
    ca.not_before = (now - Duration::from_secs(60)).into();
    ca.not_after = (now + Duration::from_secs(3600)).into();
    ca.distinguished_name
        .push(DnType::CommonName, "Connect isolated browser root");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let issuer = CertifiedIssuer::self_signed(ca, KeyPair::generate().unwrap()).unwrap();
    let mut leaf = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    leaf.not_before = (now - Duration::from_secs(60)).into();
    leaf.not_after = (now + Duration::from_secs(3600)).into();
    leaf.distinguished_name
        .push(DnType::CommonName, "Connect isolated browser listener");
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().unwrap();
    let cert = leaf.signed_by(&key, &issuer).unwrap();
    let root = f.temp.path().join("browser-root.der");
    std::fs::write(&root, issuer.der()).unwrap();
    f.config.certificate = f.temp.path().join("browser-leaf.der");
    f.config.private_key = f.temp.path().join("browser-leaf-key.der");
    std::fs::write(&f.config.certificate, cert.der()).unwrap();
    std::fs::write(&f.config.private_key, key.serialize_der()).unwrap();
    f.client.trust_certificate = root.clone();
    root
}
async fn enroll(f: &Fixture, c: &CanonicalClientConfig, index: usize) {
    let actor = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            &actor,
            ProvisioningCommand::Endpoint {
                operation_id: format!("browser-enroll-{index}"),
                identity_id: c.identity_id.clone(),
                device: DeviceSpec {
                    id: c.device_id.clone(),
                    kind: c.device_kind.clone(),
                    platform: c.device_platform.clone(),
                },
                transport: EndpointTransport::Webrtc,
                address: format!("https://localhost/browser-device-{index}"),
                capabilities: vec!["audio".into(), "video".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            &actor,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("browser-verify-{index}"),
                endpoint_id,
                expected_revision: 0,
                evidence_ref: "designated-isolated-browser".into(),
            },
        )
        .unwrap();
}
fn participant(
    f: &Fixture,
    c: &CanonicalClientConfig,
    cid: &str,
    party: &str,
    remote: &str,
    index: usize,
) -> Value {
    json!({"url":format!("wss://localhost:{}/connect",f.server.websocket_address.unwrap().port()),
        "credential":c.credential,"identityId":c.identity_id,"device":{"id":c.device_id,"kind":c.device_kind,"platform":c.device_platform},
        "signing":{"algorithm":c.signing_algorithm,"keyId":c.signing_key_id,"privatePkcs8":URL_SAFE_NO_PAD.encode(std::fs::read(&c.signing_private_key).unwrap())},
        "server":{"algorithm":c.server_algorithm,"keyId":c.server_key_id,"publicRaw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"notBefore":c.server_key_not_before.timestamp(),"expiresAt":c.server_key_expires_at.timestamp()},
        "connection":{"conversationId":cid,"sessionId":"browser-session","connectionId":format!("browser-connection-{index}"),"operationId":format!("browser-offer-{index}"),"expectedRevision":0,"participantId":party,"audioStreamId":format!("browser-audio-{index}"),"videoStreamId":format!("browser-video-{index}")},
        "subscribe":{"operationId":format!("browser-select-{index}"),"expectedRevision":1,"subscriptions":[{"from_participant":remote,"kinds":["audio"]},{"strm_id":format!("browser-video-{}",1-index)}]},
        "end":{"operationId":format!("browser-end-{index}"),"expectedRevision":2}})
}

struct OwnedUdpFixture {
    address: std::net::SocketAddr,
    packets: Arc<std::sync::atomic::AtomicU64>,
    headers: Arc<std::sync::Mutex<Vec<Value>>>,
    stop: tokio_util::sync::CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl OwnedUdpFixture {
    async fn open() -> Self {
        Self::bind(false).await
    }
    async fn stun() -> Self {
        Self::bind(true).await
    }
    async fn bind(stun: bool) -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let packets = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count = packets.clone();
        let headers = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_headers = headers.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let cancelled = stop.clone();
        let task = tokio::spawn(async move {
            let mut buffer = [0u8; 2048];
            loop {
                tokio::select! {biased;
                    _=cancelled.cancelled()=>break,
                    received=socket.recv_from(&mut buffer)=>{
                        let (length,source)=received.unwrap();
                        // The listener itself is loopback-only. Firefox can bind a
                        // local interface address before sending to that listener.
                        // Preserve that native socket's actual reflected address.
                        if !stun && !source.ip().is_loopback() { continue; }
                        if stun {
                            {
                                let mut headers = observed_headers.lock().unwrap();
                                if headers.len() < 8 {
                                    headers.push(json!({"bytes":length,"source_is_loopback":source.ip().is_loopback(),
                                        "message_type":(length>=2).then(||u16::from_be_bytes([buffer[0],buffer[1]])),
                                        "body_bytes":(length>=4).then(||u16::from_be_bytes([buffer[2],buffer[3]])),
                                        "cookie":(length>=8).then(||u32::from_be_bytes(buffer[4..8].try_into().unwrap()))}));
                                }
                            }
                            // RFC5389 loopback binding only. No proxy, auth or
                            // provider traffic; the original transaction is echoed.
                            if length < 20 || length > 2048 || buffer[..2] != [0,1]
                                || buffer[4..8] != [0x21,0x12,0xa4,0x42]
                                || usize::from(u16::from_be_bytes([buffer[2],buffer[3]])) != length-20
                                || (length-20)%4 != 0 { continue; }
                            let std::net::IpAddr::V4(ip) = source.ip() else { continue; };
                            let mut response=[0u8;32];
                            response[..4].copy_from_slice(&[1,1,0,12]);
                            response[4..20].copy_from_slice(&buffer[4..20]);
                            response[20..26].copy_from_slice(&[0,0x20,0,8,0,1]);
                            response[26..28].copy_from_slice(&(source.port()^0x2112).to_be_bytes());
                            for (index,byte) in ip.octets().iter().enumerate() {
                                response[28+index]=byte^buffer[4+index];
                            }
                            socket.send_to(&response,source).await.unwrap();
                            count.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                        } else if length>=1200 {
                            count.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            }
        });
        Self {
            address,
            packets,
            headers,
            stop,
            task: Some(task),
        }
    }
    async fn close(mut self) -> u64 {
        self.stop.cancel();
        self.task.take().unwrap().await.unwrap();
        self.packets.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Drop for OwnedUdpFixture {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[tokio::test]
async fn canonical_loopback_stun_fixture_preserves_transaction_and_joins() {
    let fixture = OwnedUdpFixture::stun().await;
    let address = fixture.address;
    let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut request = [7u8; 20];
    request[..8].copy_from_slice(&[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    client.send_to(&request, address).await.unwrap();
    let mut response = [0u8; 64];
    let (length, source) =
        tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut response))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(source, address);
    assert_eq!(length, 32);
    assert_eq!(&response[..4], &[1, 1, 0, 12]);
    assert_eq!(&response[4..20], &request[4..20]);
    assert_eq!(&response[20..26], &[0, 0x20, 0, 8, 0, 1]);
    assert_eq!(
        u16::from_be_bytes(response[26..28].try_into().unwrap()) ^ 0x2112,
        client.local_addr().unwrap().port()
    );
    assert_eq!(&response[28..32], &[127 ^ 0x21, 0x12, 0xa4, 1 ^ 0x42]);
    request[4] ^= 1;
    client.send_to(&request, address).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), client.recv_from(&mut response))
            .await
            .is_err()
    );
    assert_eq!(fixture.close().await, 1);
    let reclaimed = std::net::UdpSocket::bind(address).unwrap();
    drop(reclaimed);
}

fn original_message_counts(f: &Fixture) -> (i64, i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        f.server.store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let messages = db
        .query_row(
            "SELECT count(*) FROM messages WHERE id='browser-original-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let journal = db
        .query_row(
            "SELECT count(*) FROM journal WHERE message_id='browser-original-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let operations=db.query_row("SELECT count(*) FROM operations WHERE operation_id='browser-original-message-op' AND state='completed'",[],|r|r.get(0)).unwrap();
    (messages, journal, operations)
}

async fn run(algorithm: &str, browser_kind: &str, forced_fallback: bool) {
    Box::pin(run_profile(
        algorithm,
        browser_kind,
        forced_fallback,
        false,
        false,
        false,
    ))
    .await;
}
async fn run_profile(
    algorithm: &str,
    browser_kind: &str,
    forced_fallback: bool,
    transition_control: bool,
    transport_recovery: bool,
    transport_faults: bool,
) {
    assert!(!transport_faults || transport_recovery);
    assert!(!transport_recovery || transition_control);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let _audio_trace = audio_trace::install(&root);
    if _audio_trace.is_none() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn".into()),
            )
            .try_init();
    }
    let output = PathBuf::from(
        std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS")
            .expect("explicit private output directory"),
    );
    assert!(output.is_absolute());
    std::fs::create_dir_all(&output).unwrap();
    let mut f = if transition_control {
        Fixture::with_extra_scopes(
            algorithm,
            8,
            true,
            true,
            true,
            &[
                "session.consult",
                "session.handoff",
                "connection.replace",
                "operation.cancel",
            ],
        )
        .await
    } else {
        Fixture::with_native_end_scope(algorithm, 8).await
    };
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
    // The common enrolled-peer fixture includes a logical WT Device endpoint.
    f.config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec![format!("http://127.0.0.1:{asset_port}")],
            advertised_url: None,
        },
    );
    f.config.webrtc = Some(connect_server::canonical_webrtc::CanonicalWebRtcConfig {
        profile: connect_server::canonical_webrtc::CanonicalWebRtcProfile::OpusVp8,
        max_peers: 2,
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
    enroll(&f, &f.client, 0).await;
    enroll(&f, &other, 1).await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut create =
        connect_client::conversations::CreateRequest::persistent(vec![other.identity_id.clone()]);
    create.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("browser-room", create)
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
    native
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
        panic!("expected actual Session accept result")
    };
    let original_message = if forced_fallback {
        let mut draft = connect_client::MessageDraft::text_all(
            "one original synthetic message before transport switch",
        );
        draft.msg_id = Some("browser-original-message".into());
        native
            .send_message(
                "browser-original-message-op",
                &room.id,
                "browser-original-message",
                draft.clone(),
            )
            .await
            .unwrap();
        Some(
            json!({"type":"message.send","payload":draft,"scope":{"cid":room.id,"operation_id":"browser-original-message-op"}}),
        )
    } else {
        None
    };
    let original_counts = forced_fallback.then(|| original_message_counts(&f));
    if let Some((messages, _, operations)) = original_counts {
        assert_eq!(messages, 1);
        assert_eq!(operations, 1);
    }
    let transition_probes = if transition_control {
        transitions::seed(&f, &other)
    } else {
        Vec::new()
    };
    native.shutdown().await.unwrap();
    assert!(["chrome", "firefox"].contains(&browser_kind));
    let binary = PathBuf::from(
        std::env::var_os(if browser_kind == "chrome" {
            "CONNECT_GATE6_CHROME"
        } else {
            "CONNECT_GATE6_FIREFOX"
        })
        .expect("explicit installed browser"),
    );
    if browser_kind == "firefox" {
        let version = Command::new(&binary).arg("--version").output().unwrap();
        assert!(version.status.success());
        assert_eq!(
            String::from_utf8(version.stdout).unwrap().trim(),
            "Mozilla Firefox 158.0.1"
        );
    }
    let profile = f.temp.path().join("normal-trust-profile");
    let imported = Command::new("python3")
        .arg(root.join("scripts/video/browser_profile.py"))
        .arg(browser_kind)
        .arg(&binary)
        .arg(&profile)
        .arg(&ca)
        .output()
        .unwrap();
    std::fs::write(output.join("trust-import.log"), &imported.stderr).unwrap();
    assert!(imported.status.success());
    std::fs::write(output.join("trust-import.json"), &imported.stdout).unwrap();
    if browser_kind == "firefox" {
        // Firefox excludes loopback ICE candidates in its normal profile.
        // This owned profile qualifies only this isolated loopback fixture.
        std::fs::write(
            profile.join("user.js"),
            "user_pref(\"media.peerconnection.ice.loopback\", true);\n",
        )
        .unwrap();
    }
    std::fs::write(
        output.join("browser-profile.json"),
        serde_json::to_vec(&json!({
            "normal_certificate_verification":true,
            "isolated_firefox_loopback_ice":browser_kind == "firefox"
        }))
        .unwrap(),
    )
    .unwrap();
    let browser_log = output.join(format!("{browser_kind}.log"));
    let mut command = Command::new(binary);
    if browser_kind == "chrome" {
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
        command.arg(format!("--user-data-dir={}", profile.display()));
    } else {
        command
            .args([
                "--headless",
                "--no-remote",
                "--remote-debugging-port",
                "0",
                "--profile",
            ])
            .arg(&profile);
    }
    command
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&browser_log).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut browser = OwnedBrowser(command.spawn().unwrap());
    let until = tokio::time::Instant::now() + Duration::from_secs(12);
    let debug_port = loop {
        if browser_kind == "firefox" {
            if let Ok(text) = std::fs::read_to_string(&browser_log) {
                if let Some(port) = text
                    .lines()
                    .filter_map(|line| line.split("WebDriver BiDi listening on ").nth(1))
                    .filter_map(|url| url::Url::parse(url).ok())
                    .filter(|url| url.scheme() == "ws" && url.host_str() == Some("127.0.0.1"))
                    .find_map(|url| url.port())
                {
                    break port;
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            if let Some(port) = text
                .lines()
                .next()
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|p| *p > 0)
            {
                break port;
            }
        }
        assert!(browser.0.try_wait().unwrap().is_none());
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let mut blackholes = Vec::new();
    if forced_fallback {
        for _ in 0..2 {
            blackholes.push(OwnedUdpFixture::open().await);
        }
    }
    let stun = if browser_kind == "firefox" {
        Some(OwnedUdpFixture::stun().await)
    } else {
        None
    };
    let mut participants = vec![
        participant(&f, &f.client, &room.id, mine, theirs, 0),
        participant(&f, &other, &room.id, theirs, mine, 1),
    ];
    if let Some(stun) = &stun {
        for participant in &mut participants {
            participant["connection"]["iceServers"] =
                json!([{ "urls":format!("stun:127.0.0.1:{}",stun.address.port()) }]);
        }
    }
    if forced_fallback {
        participants[0]["fallbackOriginals"] = json!([original_message.unwrap(),
            {"type":"session.invite","payload":{"to":[theirs],"medium":"voice+video","intent":"conference","capabilities_offer":{"audio":["opus"],"video":["vp8"]}},
             "scope":{"cid":room.id,"sid":"browser-session","operation_id":"browser-invite"}}]);
        participants[1]["fallbackOriginals"] = json!([
            {"type":"session.accept","payload":{"capabilities_answer":{"audio":["opus"],"video":["vp8"]}},
             "scope":{"cid":room.id,"sid":"browser-session","operation_id":"browser-accept","expected_revision":0}}]);
        for (index, hole) in blackholes.iter().enumerate() {
            participants[index]["fallbackProbe"] = json!({"url":format!("https://127.0.0.1:{}/connect",hole.address.port()),"timeoutMs":1000});
        }
    }
    let mut spec = json!({"browser":browser_kind,"root":root,"assetPort":asset_port,"debugPort":debug_port,"seconds":60,"profileMode":"clean","sourceIds":[0,1],"sourceToneHz":[440,660],"expectedToneHz":[660,440],"participants":participants,"terminal":{"operationId":"browser-session-end","expectedRevision":accepted.session.topology_revision}});
    if transition_control {
        spec["controlOnly"] = json!(true);
        spec["transportRecovery"] = json!(transport_recovery);
        spec["controlDatabase"] = json!(f.server.store.path());
        spec["controlTransitions"] = json!(transition_probes);
    }
    if transport_faults {
        spec["transportFaults"] = recovery::fault_spec(&mut f, &ca);
    }
    let input = f.temp.path().join("private-browser-input.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    drop(asset);
    let runner_script = if transition_control {
        root.join("scripts/video/browser-control-transitions.mjs")
    } else if let Some(path) = std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_RUNNER") {
        let path = PathBuf::from(path).canonicalize().unwrap();
        assert!(
            path.starts_with(root.join(".local")) && path.is_file(),
            "diagnostic runner must be an explicit private local file"
        );
        path
    } else {
        root.join("scripts/video/browser-qualify.mjs")
    };
    let mut node = tokio::process::Command::new(
        std::env::var_os("CONNECT_GATE6_NODE").expect("explicit Node runtime"),
    );
    node.arg(runner_script)
        .stdin(std::fs::File::open(input).unwrap())
        .stdout(std::fs::File::create(output.join("browser-result.jsonl")).unwrap())
        .stderr(std::fs::File::create(output.join("runner.log")).unwrap())
        .kill_on_drop(true);
    let mut runner = node.spawn().unwrap();
    // Keep the original child across timeout so its actual kill and wait join
    // precede browser/host cleanup and the failing test verdict.
    let result = tokio::time::timeout(Duration::from_secs(110), runner.wait()).await;
    let runner_timed_out = result.is_err();
    let status = match result {
        Ok(status) => status.unwrap(),
        Err(_) => {
            runner.kill().await.unwrap();
            runner.wait().await.unwrap()
        }
    };
    drop(browser);
    let mut blackhole_packets = Vec::new();
    for hole in blackholes {
        blackhole_packets.push(hole.close().await);
    }
    let (stun_bindings, stun_headers) = match stun {
        Some(stun) => {
            let headers = stun.headers.clone();
            let count = stun.close().await;
            let values = headers.lock().unwrap().clone();
            (count, values)
        }
        None => (0, Vec::new()),
    };
    std::fs::write(
        output.join("stun.json"),
        serde_json::to_vec(
            &json!({"owned_loopback_only":true,"successful_binding_responses":stun_bindings,"bounded_numeric_headers":stun_headers}),
        )
        .unwrap(),
    )
    .unwrap();
    let message_preserved =
        original_counts.is_none_or(|before| original_message_counts(&f) == before);
    std::fs::write(output.join("transport-switch.json"),serde_json::to_vec(&json!({"forced_fallback":forced_fallback,"actual_loopback_quic_initial_datagrams":blackhole_packets,"original_message_counts_preserved":message_preserved,"original_message_counts":original_counts})).unwrap()).unwrap();

    if transition_control && !runner_timed_out && status.success() {
        let report: Value =
            serde_json::from_slice(&std::fs::read(output.join("browser-result.jsonl")).unwrap())
                .unwrap();
        if transport_recovery {
            assert_eq!(report["transport_recovery"]["passed"], true);
            assert_eq!(
                report["transport_recovery"]["steps"]
                    .as_array()
                    .unwrap()
                    .len(),
                3
            );
            assert_eq!(
                report["transport_recovery"]["rejected_current_credential"],
                true
            );
        }
        if transport_faults {
            let audit = recovery::audit(&f, &other, &report);
            std::fs::write(
                output.join("transport-fault-oracle.json"),
                serde_json::to_vec(&audit).unwrap(),
            )
            .unwrap();
        }
        let audit = transitions::audit(&f, &transition_probes, &report).await;
        std::fs::write(
            output.join("transition-parity.json"),
            serde_json::to_vec(&audit).unwrap(),
        )
        .unwrap();
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(5), async {
        while host.available_slots() != 2 || host.adapter().route_count() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let addresses = [
        f.server.address,
        f.server.websocket_address.unwrap(),
        f.server.http_address.unwrap(),
    ];
    f.server.shutdown().await.unwrap();
    let udp = std::net::UdpSocket::bind(addresses[0]).unwrap();
    let tcp = std::net::TcpListener::bind(addresses[1]).unwrap();
    let http = std::net::TcpListener::bind(addresses[2]).unwrap();
    drop((udp, tcp, http));
    std::fs::write(output.join("ownership.json"), serde_json::to_vec(&json!({"browser_kind":browser_kind,"owned_browser_joined":true,"owned_node_joined":true,"runner_timed_out":runner_timed_out,"physical_routes_and_slots_retired":cleanup.is_ok(),"server_shutdown_joined":true,"original_listener_addresses_reclaimed":3})).unwrap()).unwrap();
    assert!(cleanup.is_ok(), "physical host routes/slots did not retire");
    assert!(
        message_preserved,
        "transport switch repeated the original accepted message/journal effect"
    );
    assert!(
        !forced_fallback
            || (blackhole_packets.len() == 2 && blackhole_packets.iter().all(|count| *count > 0)),
        "actual WebTransport UDP traffic was not observed at both owned blackholes"
    );

    assert!(
        !runner_timed_out && status.success(),
        "browser qualification failed; inspect private report"
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(output.join("browser-result.jsonl")).unwrap())
            .unwrap();
    assert_eq!(report["status"], "passed");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed Chrome, private normal CA trust, synthetic AV; no device/provider"]
async fn canonical_chrome_signed_av_normal_trust_clean_eddsa() {
    Box::pin(run("EdDSA", "chrome", false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed Firefox, private normal CA trust, synthetic AV; no device/provider"]
async fn canonical_firefox_signed_av_normal_trust_clean_eddsa() {
    Box::pin(run("EdDSA", "firefox", false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed chrome, owned UDP blackhole, original native message/Session replay and synthetic AV"]
async fn canonical_chrome_actual_udp_failure_original_wss_intents_eddsa() {
    Box::pin(run("EdDSA", "chrome", true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed chrome, owned UDP blackhole, original native message/Session replay and synthetic AV"]
async fn canonical_chrome_actual_udp_failure_original_wss_intents_es256() {
    Box::pin(run("ES256", "chrome", true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed firefox, owned UDP blackhole, original native message/Session replay and synthetic AV"]
async fn canonical_firefox_actual_udp_failure_original_wss_intents_eddsa() {
    Box::pin(run("EdDSA", "firefox", true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit installed firefox, owned UDP blackhole, original native message/Session replay and synthetic AV"]
async fn canonical_firefox_actual_udp_failure_original_wss_intents_es256() {
    Box::pin(run("ES256", "firefox", true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Chrome; actual UDP failure, normal private CA trust and signed transition control; no live movement/provider"]
async fn canonical_chrome_actual_udp_failure_transition_control_eddsa() {
    Box::pin(run_profile("EdDSA", "chrome", true, true, false, false)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Chrome; actual UDP failure, normal private CA trust and signed transition control; no live movement/provider"]
async fn canonical_chrome_actual_udp_failure_transition_control_es256() {
    Box::pin(run_profile("ES256", "chrome", true, true, false, false)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Firefox; actual UDP failure, normal private CA trust and signed transition control; no live movement/provider"]
async fn canonical_firefox_actual_udp_failure_transition_control_eddsa() {
    Box::pin(run_profile("EdDSA", "firefox", true, true, false, false)).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed Firefox; actual UDP failure, normal private CA trust and signed transition control; no live movement/provider"]
async fn canonical_firefox_actual_udp_failure_transition_control_es256() {
    Box::pin(run_profile("ES256", "firefox", true, true, false, false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed chrome; explicit transport owner reconnect and current authorization on normal private TLS; control only"]
async fn canonical_chrome_transport_owner_reconnect_eddsa() {
    Box::pin(run_profile("EdDSA", "chrome", true, true, true, false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed chrome; explicit transport owner reconnect and current authorization on normal private TLS; control only"]
async fn canonical_chrome_transport_owner_reconnect_es256() {
    Box::pin(run_profile("ES256", "chrome", true, true, true, false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed firefox; explicit transport owner reconnect and current authorization on normal private TLS; control only"]
async fn canonical_firefox_transport_owner_reconnect_eddsa() {
    Box::pin(run_profile("EdDSA", "firefox", true, true, true, false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed firefox; explicit transport owner reconnect and current authorization on normal private TLS; control only"]
async fn canonical_firefox_transport_owner_reconnect_es256() {
    Box::pin(run_profile("ES256", "firefox", true, true, true, false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed chrome; bounded loopback TLS/QUIC faults and real scoped credential expiry/revocation; control only"]
async fn canonical_chrome_transport_owner_faults_eddsa() {
    Box::pin(run_profile("EdDSA", "chrome", true, true, true, true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed chrome; bounded loopback TLS/QUIC faults and real scoped credential expiry/revocation; control only"]
async fn canonical_chrome_transport_owner_faults_es256() {
    Box::pin(run_profile("ES256", "chrome", true, true, true, true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed firefox; bounded loopback TLS/QUIC faults and real scoped credential expiry/revocation; control only"]
async fn canonical_firefox_transport_owner_faults_eddsa() {
    Box::pin(run_profile("EdDSA", "firefox", true, true, true, true)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installed firefox; bounded loopback TLS/QUIC faults and real scoped credential expiry/revocation; control only"]
async fn canonical_firefox_transport_owner_faults_es256() {
    Box::pin(run_profile("ES256", "firefox", true, true, true, true)).await;
}
