//! Explicit installed-browser lane: synthetic sources, normal private CA trust.
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

struct OwnedChrome(Child);
impl Drop for OwnedChrome {
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
    let mut leaf = CertificateParams::new(vec!["localhost".into()]).unwrap();
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
async fn run(algorithm: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(
        std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS")
            .expect("explicit private output directory"),
    );
    assert!(output.is_absolute());
    std::fs::create_dir_all(&output).unwrap();
    let mut f = Fixture::with_native_end_scope(algorithm, 8).await;
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
    native.shutdown().await.unwrap();
    let chrome =
        PathBuf::from(std::env::var_os("CONNECT_GATE6_CHROME").expect("explicit installed Chrome"));
    let profile = f.temp.path().join("normal-trust-profile");
    let imported = Command::new("python3")
        .arg(root.join("scripts/video/browser_profile.py"))
        .arg("chrome")
        .arg(&chrome)
        .arg(&profile)
        .arg(&ca)
        .output()
        .unwrap();
    std::fs::write(output.join("trust-import.log"), &imported.stderr).unwrap();
    assert!(imported.status.success());
    std::fs::write(output.join("trust-import.json"), &imported.stdout).unwrap();
    let mut command = Command::new(chrome);
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
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(output.join("chrome.log")).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut browser = OwnedChrome(command.spawn().unwrap());
    let until = tokio::time::Instant::now() + Duration::from_secs(12);
    let debug_port = loop {
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
    let spec = json!({"root":root,"assetPort":asset_port,"debugPort":debug_port,"seconds":60,"profileMode":"clean","sourceIds":[0,1],"sourceToneHz":[440,660],"expectedToneHz":[660,440],"participants":[participant(&f,&f.client,&room.id,mine,theirs,0),participant(&f,&other,&room.id,theirs,mine,1)],"terminal":{"operationId":"browser-session-end","expectedRevision":accepted.session.topology_revision}});
    let input = f.temp.path().join("private-browser-input.json");
    std::fs::write(&input, serde_json::to_vec(&spec).unwrap()).unwrap();
    drop(asset);
    let mut node = tokio::process::Command::new(
        std::env::var_os("CONNECT_GATE6_NODE").expect("explicit Node runtime"),
    );
    node.arg(root.join("scripts/video/browser-qualify.mjs"))
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
    std::fs::write(output.join("ownership.json"), serde_json::to_vec(&json!({"owned_chrome_joined":true,"owned_node_joined":true,"runner_timed_out":runner_timed_out,"physical_routes_and_slots_retired":cleanup.is_ok(),"server_shutdown_joined":true,"original_listener_addresses_reclaimed":3})).unwrap()).unwrap();
    assert!(cleanup.is_ok(), "physical host routes/slots did not retire");
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
    Box::pin(run("EdDSA")).await;
}
