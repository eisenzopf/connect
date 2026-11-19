//! Genuine original Native QUIC OLD/source→WebRTC B, one signed public Root.
//! No matrix runner, injected Core F/Closed/warm/readiness, or second source
//! control peer. Source codec/epoch/PLI and OLD playout use original SDK owners.
#[path = "../../../public_video_artifacts.rs"]
mod public_video_artifacts;
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};
use futures_util::FutureExt;
const MIXED: &str = "native-rtc-public-b-original";
mod browser_fixture {
    //! Pinned private copy of existing cold normal-CA/browser fixture helpers.
    //! No public helper API or native/Core proof constructor is introduced.
    use super::*;
    use std::{
        path::PathBuf,
        process::{Child, Command},
    };
    pub(super) struct OwnedBrowser(pub(super) Child, bool);
    impl OwnedBrowser {
        pub(super) fn group_cleanup_parts(&mut self) -> (&mut Child, &mut bool) {
            (&mut self.0, &mut self.1)
        }
    }
    impl Drop for OwnedBrowser {
        fn drop(&mut self) {
            if self.1 {
                return;
            }
            // try_wait may itself reap an exited child. Never signal a PGID after
            // that point or when wait ownership is unknown (including startup panic).
            match self.0.try_wait() {
                Ok(None) => {} // STILL unreaped: original PID anchors the group.
                Ok(Some(_)) | Err(_) => {
                    self.1 = true;
                    return;
                }
            }
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
    pub(super) fn certificate(f: &mut Fixture) -> PathBuf {
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
        let mut leaf =
            CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
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
    pub(super) async fn enroll(f: &Fixture, c: &CanonicalClientConfig, index: usize) {
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
    fn signing_jwk(c: &CanonicalClientConfig) -> Value {
        use ed25519_dalek::pkcs8::DecodePrivateKey;
        use p256::elliptic_curve::sec1::ToEncodedPoint;

        // Current fixture keys can be RFC5958 PKCS#8 v2, which the pinned Node
        // OpenSSL parser rejects. Decode the same enrolled key with Rust's typed
        // decoder and send its JWK privately to the ordinary browser SDK instead.
        let encoded = std::fs::read(&c.signing_private_key).unwrap();
        match c.signing_algorithm.as_str() {
            "EdDSA" => {
                let key = ed25519_dalek::SigningKey::from_pkcs8_der(&encoded).unwrap();
                json!({"kty":"OKP","crv":"Ed25519",
                "d":URL_SAFE_NO_PAD.encode(key.to_bytes()),
                "x":URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())})
            }
            "ES256" => {
                let key = p256::SecretKey::from_pkcs8_der(&encoded).unwrap();
                let public = key.public_key().to_encoded_point(false);
                json!({"kty":"EC","crv":"P-256",
                "d":URL_SAFE_NO_PAD.encode(key.to_bytes()),
                "x":URL_SAFE_NO_PAD.encode(public.x().unwrap()),
                "y":URL_SAFE_NO_PAD.encode(public.y().unwrap())})
            }
            _ => panic!("Unknown fixture signing algorithm"),
        }
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
        let mut owner = OwnedBrowser(command.spawn().unwrap(), false);
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

    pub(super) fn target_actor(f: &Fixture, c: &CanonicalClientConfig) -> Value {
        json!({"url":format!("wss://localhost:{}/connect",f.server.websocket_address.unwrap().port()),
        "credential":c.credential,"identityId":c.identity_id,"device":{"id":c.device_id,"kind":c.device_kind,"platform":c.device_platform},
        "signing":{"algorithm":c.signing_algorithm,"keyId":c.signing_key_id,"jwk":signing_jwk(c)},
        "server":{"algorithm":c.server_algorithm,"keyId":c.server_key_id,"publicRaw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"notBefore":c.server_key_not_before.timestamp(),"expiresAt":c.server_key_expires_at.timestamp()}})
    }
}
// Cold PRIVATE mixed inputs only. The shared sparse fixture intentionally
// leaves chroma at zero; that clips RGB source identity in a browser. Preserve
// every authored Y marker/texture byte and all frame/sample clocks; write only
// the U/V planes of the two original sources this selector actually uses.
fn neutralize_selected_mixed_chroma(paths: &[std::path::PathBuf]) {
    use std::io::{Seek, SeekFrom, Write};
    const LUMA: u64 = 640 * 360;
    const FRAME: u64 = LUMA * 3 / 2;
    let neutral_chroma = vec![128u8; usize::try_from(LUMA / 2).unwrap()];
    assert_eq!(paths.len(), 6, "same original representative input set");
    for source in [0usize, 3] {
        let path = &paths[source];
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("representative-source-{source}.i420")
        );
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let expected = u64::from(FRAMES) * FRAME;
        assert_eq!(file.metadata().unwrap().len(), expected);
        for frame in 0..FRAMES {
            file.seek(SeekFrom::Start(u64::from(frame) * FRAME + LUMA))
                .unwrap();
            file.write_all(&neutral_chroma).unwrap();
        }
        file.sync_all().unwrap();
        assert_eq!(
            file.metadata().unwrap().len(),
            expected,
            "chroma preparation never changes original frame count/layout"
        );
    }
}
fn fresh_until(until: Instant) -> RequestOptions {
    programme::request(until)
}
// Cold public observations only; no grant, new request or allocation factory.
fn retain_original_allocation_status(attempt: u64, snapshot: &sdk::OperationSnapshot) {
    let diagnostic = sdk::helper::snapshot_json(snapshot);
    let text = diagnostic.to_string();
    if text.len() <= 16 * 1024 {
        eprintln!("NATIVE_ORIGINAL_PRE_ALLOCATION_STATUS attempt={attempt} {text}");
    } else {
        let token = |value: &Value| {
            value
                .as_str()
                .filter(|s| {
                    s.len() <= 96
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                })
                .map(str::to_owned)
        };
        let problem = &diagnostic["problem"];
        eprintln!(
            "NATIVE_ORIGINAL_PRE_ALLOCATION_STATUS attempt={attempt} {}",
            json!({"profile":"connect/native-original-status-diagnostic/1","state":snapshot.state,"revision":snapshot.revision,"recordId":snapshot.record_id,"problem":{"code":problem["code"].as_u64(),"category":token(&problem["category"]),"reason":token(&problem["reason"])},"full_snapshot_omitted_bytes":text.len()})
        );
    }
}
fn three_native_streams(index: usize) -> NonEmpty<sdk::OfferedStream> {
    NonEmpty {
        first: sdk::OfferedStream {
            id: format!("representative-audio-{index}").parse().unwrap(),
            kind: Medium::Audio,
            direction: Direction::Sendrecv,
            codecs: single("opus".into()),
        },
        rest: vec![
            sdk::OfferedStream {
                id: format!("representative-video-{index}").parse().unwrap(),
                kind: Medium::Video,
                direction: Direction::Sendonly,
                codecs: single("vp8".into()),
            },
            sdk::OfferedStream {
                id: format!("representative-incoming-{index}").parse().unwrap(),
                kind: Medium::Video,
                direction: Direction::Recvonly,
                codecs: single("vp8".into()),
            },
        ],
    }
}
async fn read_browser_file(
    directory: &Path,
    name: &str,
    runner: &mut tokio::process::Child,
    until: Instant,
) -> Value {
    timeout_at(until, async {
        loop {
            let path = directory.join(format!("{name}.json"));
            if path.is_file() {
                let bytes = std::fs::read(path).unwrap();
                assert!(bytes.len() < 65536);
                return serde_json::from_slice(&bytes).unwrap();
            }
            assert!(
                runner.try_wait().unwrap().is_none(),
                "same original browser exited before {name}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("same original browser IPC cutoff {name}"))
}
fn publish_file(directory: &Path, name: &str, value: &Value) {
    let pending = directory.join(format!("{name}.pending"));
    init::private_write(&pending, &serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::rename(pending, directory.join(format!("{name}.json"))).unwrap();
}
// Immutable public SDK current binding projection, never a Core/transport
// capability. The SAME Client workers above own both Session Connections.
async fn pooled_bindings(
    connections: &[sdk::Connection],
    session: &sdk::SessionId,
    until: Instant,
) -> Vec<Value> {
    let mut values = Vec::new();
    for connection in connections {
        let current = connection.refresh(fresh_until(until)).await.unwrap();
        assert_eq!(&current.session_id, session);
        assert_eq!(current.generation, 1);
        let native = connection
            .native_negotiation()
            .unwrap()
            .expect("SAME original native physical parameters");
        let streams=native.streams.iter().map(|stream|json!({"id":stream.id,"medium":format!("{:?}",stream.medium),"direction":format!("{:?}",stream.direction),"codec":stream.codec,"handle":stream.stream_local_id})).collect::<Vec<_>>();
        values.push(json!({"connection":current.id,"session":current.session_id,"endpoint":current.endpoint_id,"device":current.device_id,"generation":current.generation,"state":current.state,"revision":current.revision,"stream_revision":current.stream_revision,"native":{"source_clock":native.source_clock,"stream_revision":native.stream_revision,"feedback":native.video_feedback,"streams":streams}}));
    }
    values
}
fn monitor_pooled_originals(
    progress: Vec<SharedProgress>,
    epochs: Vec<sdk::VideoReceiveEpoch>,
    stop: CancellationToken,
    until: Instant,
) -> Work {
    tokio::spawn(async move {
        AssertUnwindSafe(async {
            assert_eq!(progress.len(),2);assert_eq!(epochs.len(),2);
            let mut tick=tokio::time::interval(Duration::from_millis(20));
            loop {tokio::select! {biased;_ = stop.cancelled()=>break,_=tick.tick()=>{}};
                let now=Instant::now();if now>=until {break;}
                for (value,epoch)in progress.iter().zip(&epochs) {let p=value.lock().unwrap();
                    assert_eq!(p.epoch.as_ref(),Some(epoch),"unrelated Session keeps original authenticated native source epoch");
                    assert!(now.saturating_duration_since(p.last_video.unwrap())<=Duration::from_secs(1),"unrelated actual decoded video cannot stall for more than original1s limit");
                    assert!(now.saturating_duration_since(p.last_audio.unwrap())<=Duration::from_secs(1),"unrelated actual decoded PCM cannot stall for more than original1s limit");
                }
            }
        }).catch_unwind().await.map_err(panic_text)
    })
}
async fn await_pooled_advancement(progress: &[SharedProgress], until: Instant) {
    assert_eq!(progress.len(), 2);
    let before = progress
        .iter()
        .map(|value| {
            let p = value.lock().unwrap();
            (
                p.frames,
                p.audio_samples,
                p.published_frames,
                p.published_audio_packets,
                p.timestamp,
                p.epoch.clone(),
                p.audio_run_nonincreases,
                p.encoded_frames,
            )
        })
        .collect::<Vec<_>>();
    timeout_at(until,async {loop {
        let advanced=progress.iter().zip(&before).enumerate().all(|(slot,(value,old))|{let p=value.lock().unwrap();
            assert_eq!(p.epoch,old.5,"same unrelated native source identity, no rebind");assert_eq!(p.audio_run_nonincreases,old.6,"unrelated contiguous PCM cannot reset during a different Session's move");
            // Both counterpart chains are original Native->Native. The real
            // encoder count increments BEFORE Sample broadcast. Reading N
            // proves Sample(index N) was not published to its publisher at
            // this checkpoint. Its original source timestamp is N*3000;
            // the native normalizer/fragmenter preserve those header bytes.
            // Requiring that decoded coordinate defeats pre-checkpoint buffer
            // progress without comparing any independent PC/clock timeline.
            let future_original=before[1-slot].7.checked_mul(3000).and_then(|value|u32::try_from(value).ok()).expect("bounded35s native source timestamp does not wrap");
            let decoded_after=p.timestamp.is_some_and(|timestamp|timestamp>=future_original);
            p.frames>=old.0+3&&p.audio_samples>=old.1+2880&&p.published_frames>old.2&&p.published_audio_packets>old.3&&p.timestamp!=old.4&&decoded_after
        });
        if advanced{break;}tokio::task::yield_now().await;
    }}).await.expect("actual unrelated Session decoders and original publishers must advance under SAME original move cutoff");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "explicit installed Chromium/Node/live codecs; pooled two-Session Native QUIC OLD and announced publisher to signed public WebRTC B"]
async fn actual_native_old_and_announced_publisher_to_webrtc_b_use_original_sdk_root_and_same_warmed_decoder(
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    assert!(matches!(std::env::consts::OS, "linux" | "macos"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    // Shared targets also have packet sites; admit only these exact cold
    // modules. Missing module metadata fails closed for this diagnostic.
    use tracing_subscriber::prelude::*;
    let control_trace_installed =
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(std::io::stderr)
                    .with_filter(tracing_subscriber::filter::filter_fn(|meta| {
                        matches!(
                        meta.module_path(),
                        Some("connect_service::canonical_replacement::preparation")
                            | Some("connect_service::canonical_replacement::public_video::pipeline")
                    ) && matches!(
                            meta.target(),
                            "connect_service::canonical_replacement"
                                | "connect_service::replacement_phase_trace"
                        ) && meta.level() == &tracing::Level::DEBUG
                    })),
            )
            .try_init()
            .is_ok();
    eprintln!(
        "native-replacement-control-diagnostic subscriber_installed={control_trace_installed}"
    );
    let mut f =
        Fixture::with_extra_scopes("EdDSA", 12, true, true, true, &["connection.replace"]).await;
    let mut retained_artifacts = public_video_artifacts::Capture::new(
        &root,
        f.temp.path().join("native-rtc-public-b"),
        f.config.database.clone(),
    );
    let ca = browser_fixture::certificate(&mut f);
    f.server.shutdown().await.unwrap();
    let assets = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let asset_port = assets.local_addr().unwrap().port();
    f.config.websocket = Some(
        connect_server::canonical_websocket::CanonicalWebSocketConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            mount_path: "/connect".into(),
            allowed_origins: vec![format!("http://127.0.0.1:{asset_port}")],
            advertised_url: None,
        },
    );
    f.config.webrtc = Some(connect_server::canonical_webrtc::CanonicalWebRtcConfig {
        profile: connect_server::canonical_webrtc::CanonicalWebRtcProfile::OpusVp8,
        max_peers: 2,
        addressing: None,
    });
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "connection.end".into(),
        "connection.replace".into(),
    ]);
    let configs = [&f.client.identity_id, &f.peer.identity_id]
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            super::super::super::super::super::canonical_native_connections::party_with_scope(
                &f,
                id,
                840 + i,
                vec!["audio".into(), "video".into()],
                scope.clone(),
            )
        })
        .collect::<Vec<_>>();
    let target = super::super::super::super::super::canonical_native_connections::party_with_scope(
        &f,
        &f.client.identity_id,
        842,
        vec!["audio".into(), "video".into()],
        scope.clone(),
    );
    browser_fixture::enroll(&f, &target, 843).await;
    let endpoint: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT id FROM endpoints WHERE device_id=?1 AND transport='webrtc'",
        [&target.device_id],
        |r| r.get(0),
    )
    .unwrap();
    let directory = f.temp.path().join("native-rtc-public-b");
    std::fs::create_dir(&directory).unwrap();
    let (binary, raw) = codec_inputs(&directory).await;
    let mut clients = Vec::new();
    let mut sessions = Vec::new();
    let mut encoders = Vec::new();
    let mut works = Vec::<Work>::new();
    let mut browser = None;
    let mut runner = None;
    let stop = CancellationToken::new();
    let mut progress = Vec::<SharedProgress>::new();
    let work=AssertUnwindSafe(async {
        neutralize_selected_mixed_chroma(&raw); // cold, before clients/encoder readiness/media clocks
        for config in &configs {clients.push(Client::connect_configured(configured(config.clone())).await.unwrap());}
        let conversation=clients[0].create_conversation(vec![configs[1].identity_id.parse().unwrap()],ConversationPolicy::Persistent,mutation("mixed-room")).await.unwrap().wait(options()).await.unwrap().value;
        let view=conversation.refresh(options()).await.unwrap();let remote=view.participants.iter().find(|p|p.identity_id.as_str()==configs[1].identity_id).unwrap().id.clone();
        let capabilities=json!({"audio":["opus"],"video":["vp8"]});
        let session=conversation.invite("mixed-session".parse().unwrap(),vec![remote.clone()],sdk::SessionMedium::VoiceVideo,"actual mixed native public B".into(),capabilities.clone(),mutation("mixed-invite")).await.unwrap().wait(options()).await.unwrap().value;
        sessions.push(session.clone());let other=clients[1].conversation_by_id(conversation.id().clone()).session_by_id(session.id().clone());
        let before=other.refresh(options()).await.unwrap();other.accept(capabilities.clone(),before.topology_revision,mutation("mixed-accept")).await.unwrap().wait(options()).await.unwrap();
        let mut connections=Vec::new();
        for (slot,index) in [0usize,3].into_iter().enumerate() {
            let s=clients[slot].conversation_by_id(conversation.id().clone()).session_by_id(session.id().clone());
            connections.push(s.offer_connection(format!("mixed-native-{index}").parse().unwrap(),three_native_streams(index),mutation(&format!("mixed-offer-{index}"))).await.unwrap().wait(options()).await.unwrap().value);
        }
        for(slot,source)in[3usize,0].into_iter().enumerate(){let c=&connections[slot];let revision=c.refresh(options()).await.unwrap().stream_revision.unwrap();
            c.subscribe(NonEmpty {first:StreamSelector::Stream(format!("representative-audio-{source}").parse().unwrap()),rest:vec![StreamSelector::Stream(format!("representative-video-{source}").parse().unwrap())]},revision,mutation(&format!("mixed-select-{slot}"))).await.unwrap().wait(options()).await.unwrap();}
        {
            // SAME original Clients/Shared workers/accepted peers: no new
            // connect/open/claim_native IO is performed for this second Session.
            let second=conversation.invite("mixed-pooled-session".parse().unwrap(),vec![remote],sdk::SessionMedium::VoiceVideo,"unrelated Session stays decoded during strict Native-to-RTC F".into(),capabilities.clone(),mutation("mixed-pooled-invite")).await.unwrap().wait(options()).await.unwrap().value;
            let remote=clients[1].conversation_by_id(conversation.id().clone()).session_by_id(second.id().clone());
            let view=remote.refresh(options()).await.unwrap();remote.accept(capabilities,view.topology_revision,mutation("mixed-pooled-accept")).await.unwrap().wait(options()).await.unwrap();
            for(slot,index)in[1usize,2].into_iter().enumerate(){
                let same_peer=clients[slot].conversation_by_id(conversation.id().clone()).session_by_id(second.id().clone());
                connections.push(same_peer.offer_connection(format!("mixed-pooled-native-{index}").parse().unwrap(),three_native_streams(index),mutation(&format!("mixed-pooled-offer-{index}"))).await.unwrap().wait(options()).await.unwrap().value);
            }
            for(slot,source)in[2usize,1].into_iter().enumerate(){let c=&connections[2+slot];let revision=c.refresh(options()).await.unwrap().stream_revision.unwrap();
                c.subscribe(NonEmpty {first:StreamSelector::Stream(format!("representative-audio-{source}").parse().unwrap()),rest:vec![StreamSelector::Stream(format!("representative-video-{source}").parse().unwrap())]},revision,mutation(&format!("mixed-pooled-select-{slot}"))).await.unwrap().wait(options()).await.unwrap();}
            sessions.push(second);
        }
        // All six are started by the exact existing readiness helper. The
        // four selected originals move immediately to registered source owners;
        // the two unused original encoders are joined as one owned group.
        encoder_ready::prepare(&binary,&raw,&mut encoders,Instant::now()+Duration::from_secs(8)).await.unwrap();
        let(chromium,debug_port)=browser_fixture::browser(&root,&directory,&ca).await;browser=Some(chromium);
        retained_artifacts.browser_started(&mut browser.as_mut().unwrap().0).await.expect("actual original Chromium PID/PGID captured while live");
        let spec=json!({"format":"connect/native-rtc-public-b-target/1","root":root,"directory":directory,"assetPort":asset_port,"debugPort":debug_port,"operationId":MIXED,"expectedAudioHz":330+3*110,"expectedVideoLuma":marker(3),"actor":browser_fixture::target_actor(&f,&target)});
        let input=directory.join("input.json");init::private_write(&input,&serde_json::to_vec(&spec).unwrap()).unwrap();drop(assets);
        let child=tokio::process::Command::new(std::env::var_os("CONNECT_GATE6_NODE").expect("designated Node")).arg(root.join("scripts/video/browser-native-public-b-target.mjs")).stdin(std::fs::File::open(input).unwrap()).stdout(std::fs::File::create(directory.join("browser-result.json")).unwrap()).stderr(std::fs::File::create(directory.join("browser.log")).unwrap()).kill_on_drop(true).spawn().unwrap();runner=Some(child);
        read_browser_file(&directory,"browser-ready",runner.as_mut().unwrap(),Instant::now()+Duration::from_secs(12)).await;
        let began=Instant::now()+Duration::from_millis(100);let until=began+Duration::from_secs(35);
        let retirements=connections.iter().map(|c|Arc::new(RetirementWitness {database:f.config.database.clone(),source:c.id().as_str().into(),original:Mutex::new(None)})).collect::<Vec<_>>();
        let sources=vec![(0usize,3usize),(3,0),(1,2),(2,1)];
        for _ in &sources {let observed=Arc::new(Mutex::new(Progress::default()));{let mut p=observed.lock().unwrap();p.public=true;p.public_since=Some(began);}progress.push(observed);}
        // Remove in descending original-index order, only immediately before
        // registering its original source/codec owner. Every unused encoder
        // stays in the outer cleanup Vec if a later decoder/setup fails.
        for index in (0..raw.len()).rev() {if let Some(slot)=sources.iter().position(|(source,_)|*source==index){
            let encoder=encoders.remove(index);let observed=progress[slot].clone();
            let(master,owners)=programme::source(index,encoder,observed.clone(),stop.clone(),began,until);works.extend(owners);
            works.extend(spawn_publisher(&connections[slot],index,&master,observed.clone(),stop.clone(),retirements[slot].clone(),began,until));
            works.push(spawn_original_receiver(&connections[slot],index,sources[slot].1,&binary,observed,stop.clone(),retirements[slot].clone(),began,until));
        }}
        let unused=std::mem::take(&mut encoders);let unused=encoder_ready::close_all(unused).await;assert!(unused.into_iter().all(|v|matches!(v,Ok(true))));
        timeout_at(began+Duration::from_secs(8),async {loop {if progress.iter().all(|p|{let p=p.lock().unwrap();p.frames>=5&&p.audio_samples>=4800}){break;}tokio::task::yield_now().await;}}).await.unwrap();
        let old_before=progress[0].lock().unwrap().frames;let retained_before=progress[1].lock().unwrap().frames;
        let feedback_before=progress[1].lock().unwrap().publisher_feedback_used;
        let move_started=Instant::now();let cutoff=move_started+Duration::from_secs(8);let topology=session.refresh(fresh_until(cutoff)).await.unwrap().topology_revision;
        let pooled_original={
            assert_eq!(connections.len(),4);assert_eq!(sessions.len(),2);assert_eq!(clients.len(),2);
            let original=pooled_bindings(&connections[2..],sessions[1].id(),cutoff).await;
            let epochs=progress[2..].iter().map(|p|p.lock().unwrap().epoch.clone().expect("actual unrelated native source decoded before freeze")).collect();
            works.push(monitor_pooled_originals(progress[2..].to_vec(),epochs,stop.clone(),cutoff));original
        };
        let operation=connections[0].replace_device(sdk::MoveInput {retain:connections[1].id().clone(),target:sdk::MoveTarget::Endpoint(endpoint.parse().unwrap()),expected_topology_revision:topology,
            video:Some(ReplacementVideo {publications:vec![ReplacementVideoStream {previous:"representative-video-0".into(),replacement:"mixed-target-video".into()}],receivers:vec![ReplacementVideoStream {previous:"representative-video-3".into(),replacement:"mixed-target-incoming".into()}]})},programme::mutation_at(MIXED,cutoff)).await.unwrap();
        // Status observes the accepted original command; its record may precede
        // the asynchronous worker's private allocation transaction. Observe only
        // this SAME record under the SAME original8s command cutoff.
        let (record,reservation,candidate,runtime,generation)=timeout_at(cutoff,async {
            let mut original_record=None;let mut attempt=0u64;
            loop {
                attempt+=1;
                let observed=operation.status(fresh_until(cutoff)).await;
                if let Err(error)=&observed {eprintln!("NATIVE_ORIGINAL_ALLOCATION_STATUS_FAILURE attempt={attempt} {}",sdk::helper::error_json(error));}
                let snapshot=observed.unwrap();retain_original_allocation_status(attempt,&snapshot);
                let record=snapshot.record_id.clone().expect("same original operation record");
                if let Some(original)=&original_record {assert_eq!(&record,original,"same original allocation record");} else {original_record=Some(record.clone());}
                match snapshot.state {
                    OperationState::Failed|OperationState::Cancelled|OperationState::Completed=>panic!("original replacement cannot allocate before target preparation: state={} revision={} (see same original snapshot)",snapshot.state.as_str(),snapshot.revision),
                    OperationState::Unknown|OperationState::Interrupted=>{}, // never an allocation/readiness success
                    OperationState::Accepted|OperationState::Preparing|OperationState::Ready|OperationState::Committing=>{
                        let allocation:rusqlite::Result<(String,String,String,u64)>={
                            let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
                            db.query_row("SELECT reservation_id,candidate_connection_id,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",[record.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                        }; // No SQL handle/statement/Reader remains across the pause.
                        match allocation {
                            Ok((reservation,candidate,runtime,generation))=>break (record,reservation,candidate,runtime,generation),
                            Err(rusqlite::Error::QueryReturnedNoRows)=>eprintln!("NATIVE_ORIGINAL_PRIVATE_ALLOCATION_PENDING attempt={attempt} record={} error=QueryReturnedNoRows",record.as_str()),
                            Err(error)=>{eprintln!("NATIVE_ORIGINAL_PRIVATE_ALLOCATION_QUERY_FAILURE attempt={attempt} record={} error={error:?}",record.as_str());panic!("same original allocation query failed: {error:?}");}
                        }
                    }
                }
                tokio::time::sleep_until((Instant::now()+Duration::from_millis(20)).min(cutoff)).await;
            }
        }).await.expect("same original8s native allocation observation cutoff");
        assert_eq!(generation,2);
        *retirements[0].original.lock().unwrap()=Some(RetiredMove {record:record.as_str().into(),reservation,candidate: candidate.clone(),runtime,generation,observed_commit:Arc::new(AtomicBool::new(false))});
        let remaining=cutoff.checked_duration_since(Instant::now()).unwrap();let expires_at_ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()+remaining.as_millis();
        publish_file(&directory,"original-root",&json!({"operation_id":MIXED,"source":connections[0].id(),"conversation_id":conversation.id(),"session_id":session.id(),"reference":operation.reference_json(),"expires_at_ms":u64::try_from(expires_at_ms).unwrap()}));
        let before_ready=read_browser_file(&directory,"before-public-ready",runner.as_mut().unwrap(),cutoff).await;
        assert_eq!(before_ready["target"],candidate);assert_eq!(before_ready["record_id"],record.as_str());
        let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let phase:String=db.query_row("SELECT phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='cut-bound-post-f'",[record.as_str()],|r|r.get(0)).unwrap();assert_eq!(serde_json::from_str::<Value>(&phase).unwrap(),before_ready["phase"]);
        let commits:u64=db.query_row("SELECT (SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1)+(SELECT count(*) FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase IN ('decoded-ready','activated'))",[record.as_str()],|r|r.get(0)).unwrap();assert_eq!(commits,0);drop(db);
        assert!(progress[1].lock().unwrap().publisher_feedback_used>feedback_before,"actual retained Native announced-epoch feedback reached original live encoder");
        assert!(progress[1].lock().unwrap().frames>retained_before,"actual Native OLD camera and retained native decoder advance through F/warm");
        assert!(progress[0].lock().unwrap().frames>=old_before);assert!(progress[0].lock().unwrap().audio_samples>=4800);
        {
            // The real production phase exists and signed B Ready remains
            // unsent. Fresh unrelated Native IO must not invalidate SAME F.
            await_pooled_advancement(&progress[2..],cutoff).await;
            assert_eq!(pooled_bindings(&connections[2..],sessions[1].id(),cutoff).await,pooled_original);
        }
        publish_file(&directory,"before-public-ready.ack",&json!({"operation_id":MIXED}));
        let completed=operation.wait(fresh_until(cutoff)).await.unwrap();assert_eq!(completed.value.connection.id().as_str(),candidate);assert!(retirements[0].committed());
        let retained_at_commit={let p=progress[1].lock().unwrap();(p.frames,p.audio_samples)};
        let rendered=read_browser_file(&directory,"public-rendered",runner.as_mut().unwrap(),cutoff).await;
        assert_eq!(rendered["target"],candidate);assert_eq!(rendered["same_warmed_element"],true);assert!(rendered["public_draws"].as_u64().unwrap()>=5&&rendered["video_frames"].as_u64().unwrap()>=5&&rendered["audio_samples"].as_u64().unwrap()>=4800);
        assert!(rendered["changing_video_frames"].as_u64().unwrap()>=3,"actual original Native source texture must change on adopted B");
        assert_eq!(rendered["audio_identity_hz"],330+3*110);
        assert_eq!(rendered["video_identity_luma"],marker(3));
        timeout_at(cutoff,async {loop {let advanced={let p=progress[1].lock().unwrap();p.frames>=retained_at_commit.0+3&&p.audio_samples>=retained_at_commit.1+2880};if advanced{break;}tokio::task::yield_now().await;}}).await.expect("same retained Native decoder receives actual newly published RTC camera/audio");
        assert_eq!(clients[0].conversation_by_id(conversation.id().clone()).refresh(fresh_until(cutoff)).await.unwrap().id,conversation.id().clone());
        let auth=f.server.store.authenticate(&configs[0].credential).unwrap().credential_id;
        let actual=f.server.store.execute_operation_action(&auth,OperationAction::Get{operation_id:MIXED.into()}).unwrap();assert_eq!(actual.state,OperationState::Completed);
        assert_eq!(actual.record_id,record.as_str());let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let activated:String=db.query_row("SELECT phase_json FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase='activated'",[record.as_str()],|r|r.get(0)).unwrap();assert_eq!(serde_json::from_str::<Value>(&activated).unwrap(),rendered["activated"]);drop(db);
        {
            await_pooled_advancement(&progress[2..],cutoff).await;
            assert_eq!(pooled_bindings(&connections[2..],sessions[1].id(),cutoff).await,pooled_original);
            assert_eq!(clients[0].conversation_by_id(conversation.id().clone()).session_by_id(sessions[1].id().clone()).refresh(fresh_until(cutoff)).await.unwrap().id,sessions[1].id().clone());
        }
        publish_file(&directory,"public-rendered.ack",&json!({"operation_id":MIXED}));
    }).catch_unwind().await;
    // Every original owner is attempted/joined before any final oracle escapes.
    stop.cancel();
    let mut cleanup = Vec::new();
    for mut owner in works {
        let result = timeout_at(Instant::now() + Duration::from_secs(5), &mut owner).await;
        match result {
            Ok(Ok(Ok(()))) => {}
            Ok(other) => cleanup.push(format!("original native work: {other:?}")),
            Err(error) => {
                cleanup.push(format!("original native work timeout: {error:?}"));
                owner.abort();
                let _ = owner.await;
            }
        }
    }
    for result in encoder_ready::close_all(encoders).await {
        if !matches!(result, Ok(true)) {
            cleanup.push(format!("original encoder: {result:?}"));
        }
    }
    for (index, session) in sessions.iter().enumerate() {
        let ended = AssertUnwindSafe(async {
            let snapshot = session.refresh(options()).await?;
            session
                .end(
                    0,
                    "mixed original cleanup".into(),
                    snapshot.topology_revision,
                    mutation(&format!("mixed-end-{index}")),
                )
                .await?
                .wait(options())
                .await?;
            Ok::<(), sdk::Error>(())
        })
        .catch_unwind()
        .await;
        if !matches!(ended, Ok(Ok(()))) {
            cleanup.push(format!("session cleanup: {ended:?}"));
        }
    }
    for client in &clients {
        if let Err(e) = client.close(options()).await {
            cleanup.push(format!("original client: {e:?}"));
        }
    }
    if let Some(child) = runner.as_mut() {
        match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
            Ok(Ok(status)) if status.success() => {}
            other => {
                cleanup.push(format!("original target runner: {other:?}"));
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
    }
    if let Some(child) = browser.as_mut() {
        let (original_child, disarmed) = child.group_cleanup_parts();
        let joined = retained_artifacts
            .join_browser(original_child, disarmed)
            .await;
        if let Err(error) = joined {
            cleanup.push(format!("original browser group: {error}"));
        }
    }
    let joined = f.server.shutdown().await;
    let artifact_result = retained_artifacts.save("after-original-cleanup-attempts");
    if joined.is_err() {
        cleanup.push(format!("original server: {joined:?}"));
    }
    if let Err(panic) = work {
        std::panic::resume_unwind(panic);
    }
    artifact_result.expect("actual private Native/B proof files retained");
    assert!(cleanup.is_empty(), "{cleanup:?}");
}
