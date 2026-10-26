#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
//! A real signed native SDK bridge; setup never accesses the application Store.
#[path = "support/connector_stock_setup.rs"]
mod setup;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    application::ApplicationClient,
    canonical::{
        CanonicalClient, CanonicalClientConfig, CanonicalHttpClient, CommandScope, PcmuDevice,
        PublicNativeAudio, PublicPcmuBridge,
    },
    ClientError,
};
use connect_connector_api::{dispatch::Authority, DispatchPermit};
use connect_media_connector_template::{Adapter, Admission, Remote};
use connect_protocol::{
    credential_issuance::CredentialIssueRequest,
    provisioning::*,
    session_control::{Action, Request},
};
use connect_server::canonical::RunningCanonicalServer;
use rvoip_core::{
    adapter::{ConnectionAdapter, OriginateRequest},
    ids::{ParticipantId, SessionId, StreamId},
    stream::{MediaFrame, StreamKind},
    Direction, Transport,
};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex, Notify};

struct LocalCustody {
    current: AtomicBool,
    until: Instant,
}
impl Authority for LocalCustody {
    fn current(&self) -> bool {
        self.current.load(Ordering::SeqCst)
    }
    fn deadline(&self) -> Instant {
        self.until
    }
}
#[derive(Clone, Copy)]
enum DeviceMode {
    Normal,
    Short,
    Panic,
}
struct Device {
    adapter: Arc<Adapter>,
    custody: Arc<LocalCustody>,
    input: Mutex<mpsc::Receiver<MediaFrame>>,
    output: mpsc::Sender<MediaFrame>,
    stream: StreamId,
    clock: AtomicU32,
    cancelled: Notify,
    mode: DeviceMode,
    _events: Mutex<mpsc::Receiver<rvoip_core::adapter::OrchestratorAdapterEvent>>,
    drain_entered: Notify,
    drain_release: Notify,
    drains: AtomicUsize,
}
#[async_trait::async_trait]
impl PcmuDevice for Device {
    async fn receive(&self) -> connect_client::Result<Vec<u8>> {
        let mut frame = self
            .input
            .lock()
            .await
            .recv()
            .await
            .map(|v| v.payload.to_vec())
            .ok_or(ClientError::MediaEnded)?;
        match self.mode {
            DeviceMode::Normal => (),
            DeviceMode::Short => {
                frame.pop();
            }
            DeviceMode::Panic => panic!("deliberate local device receive failure"),
        }
        Ok(frame)
    }
    async fn send(&self, bytes: &[u8]) -> connect_client::Result<()> {
        self.output
            .send(MediaFrame {
                stream_id: self.stream.clone(),
                kind: StreamKind::Audio,
                payload: bytes.to_vec().into(),
                timestamp_rtp: self.clock.fetch_add(160, Ordering::SeqCst),
                source_clock: None,
                captured_at: chrono::Utc::now(),
                payload_type: Some(0),
            })
            .await
            .map_err(|_| ClientError::MediaEnded)
    }
    fn cancel(&self) {
        self.custody.current.store(false, Ordering::SeqCst);
        self.adapter.cancel();
        self.cancelled.notify_one();
    }
    async fn drain(&self) -> connect_client::Result<()> {
        self.drains.fetch_add(1, Ordering::SeqCst);
        self.drain_entered.notify_one();
        self.drain_release.notified().await;
        self.adapter
            .drain()
            .await
            .map_err(|_| ClientError::MediaEnded)
    }
}
async fn peer(
    f: &setup::Setup,
    server: &RunningCanonicalServer,
    admin: &mut ApplicationClient,
    identity: &str,
    name: &str,
    recipients: &[String],
) -> CanonicalClientConfig {
    let issued = admin
        .issue_scoped_credential(CredentialIssueRequest {
            operation_id: format!("{name}-credential"),
            identity_id: identity.into(),
            operations: [
                "send",
                "history",
                "conversation.get",
                "session.invite",
                "session.accept",
                "session.end",
                "session_get",
                "connection.offer",
                "connection.end",
                "stream.subscribe",
                "stream.unsubscribe",
                "operation.get",
            ]
            .map(str::to_owned)
            .to_vec(),
            recipients: recipients.to_vec(),
            expires_at: chrono::Utc::now().timestamp() + 600,
        })
        .await
        .unwrap();
    let device = format!("{name}-device");
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = admin
        .provision(ProvisioningCommand::Endpoint {
            operation_id: format!("{name}-endpoint"),
            identity_id: identity.into(),
            device: DeviceSpec {
                id: device.clone(),
                kind: "service".into(),
                platform: "stock-native-fixture".into(),
            },
            transport: EndpointTransport::Quic,
            address: format!("quic://localhost:{}/{name}", server.address.port()),
            capabilities: vec!["audio".into(), "data".into()],
            secret_ref: None,
        })
        .await
        .unwrap()
    else {
        panic!("endpoint")
    };
    admin
        .provision(ProvisioningCommand::VerifyEndpoint {
            operation_id: format!("{name}-verify"),
            endpoint_id,
            expected_revision: 0,
            evidence_ref: "isolated-signed-native-fixture".into(),
        })
        .await
        .unwrap();
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let private = f.temp.path().join(format!("{name}.der"));
    std::fs::write(&private, key.serialize_der()).unwrap();
    admin
        .provision(ProvisioningCommand::SigningKey {
            operation_id: format!("{name}-key"),
            identity_id: identity.into(),
            credential_id: issued.credential_id,
            device_id: device.clone(),
            key_id: name.into(),
            algorithm: SigningAlgorithm::EdDSA,
            public_key: URL_SAFE_NO_PAD.encode(key.public_key_raw()),
            not_before: chrono::Utc::now().timestamp() - 1,
            expires_at: chrono::Utc::now().timestamp() + 600,
            expected_revision: None,
        })
        .await
        .unwrap();
    let raw = std::fs::read(&f.config.signing_private_key).unwrap();
    let server_key = rcgen::KeyPair::try_from(raw.as_slice()).unwrap();
    let public = f.temp.path().join("native-server-public.key");
    std::fs::write(&public, server_key.public_key_raw()).unwrap();
    CanonicalClientConfig {
        network_profile: Default::default(),
        endpoint: server.address,
        server_name: "localhost".into(),
        trust_certificate: f.config.certificate.clone(),
        credential: issued.credential,
        identity_id: identity.into(),
        device_id: device,
        device_kind: "service".into(),
        device_platform: "stock-native-fixture".into(),
        signing_key_id: name.into(),
        signing_algorithm: "EdDSA".into(),
        signing_private_key: private,
        server_key_id: f.config.signing_key_id.clone(),
        server_algorithm: "EdDSA".into(),
        server_public_key: public,
        server_key_not_before: f.config.signing_key_not_before,
        server_key_expires_at: f.config.signing_key_expires_at,
        timeout_ms: 5000,
    }
}
async fn try_audio(
    config: CanonicalClientConfig,
    cid: &str,
    sid: &str,
    name: &str,
) -> connect_client::Result<PublicNativeAudio> {
    CanonicalClient::open(config).await?.prepare_public_native_audio(CommandScope {conversation_id:Some(cid.into()),session_id:Some(sid.into()),connection_id:Some(name.into()),operation_id:Some(format!("{name}-offer")),expected_revision:Some(0)},json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":format!("{name}-voice"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]})).await
}
async fn audio(config: CanonicalClientConfig, cid: &str, name: &str) -> PublicNativeAudio {
    try_audio(config, cid, "media-host-session", name)
        .await
        .unwrap()
}
async fn select(audio: &mut PublicNativeAudio, source: &str, operation: &str) {
    audio
        .subscribe_audio(
            operation,
            audio.stream_revision(),
            vec![
                serde_json::from_value(json!({"from_participant":source,"kinds":["audio"]}))
                    .unwrap(),
            ],
        )
        .await
        .unwrap();
}
fn decode(byte: u8) -> i32 {
    let v = !byte;
    let magnitude = (((i32::from(v & 15) << 3) + 132) << ((v >> 4) & 7)) - 132;
    if v & 128 == 0 {
        magnitude
    } else {
        -magnitude
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_native_host_exchanges_actual_pcmu_and_cancelled_drain_keeps_original_cleanup()
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_service=debug")
        .with_test_writer()
        .try_init();
    connect_media_connector_template::native_host_manifest()
        .validate()
        .unwrap();
    let (f, server, root, host) = scene().await;
    let mut root_audio = audio(root, &f.cid, "root-native").await;
    let mut host_audio = audio(host, &f.cid, "fixture-native").await;
    root_audio
        .subscribe_audio(
            "root-hear-fixture",
            root_audio.stream_revision(),
            vec![
                serde_json::from_value(json!({"from_participant":f.ai_pid,"kinds":["audio"]}))
                    .unwrap(),
            ],
        )
        .await
        .unwrap();
    host_audio
        .subscribe_audio(
            "fixture-hear-root",
            host_audio.stream_revision(),
            vec![
                serde_json::from_value(json!({"from_participant":f.root_pid,"kinds":["audio"]}))
                    .unwrap(),
            ],
        )
        .await
        .unwrap();
    let (device, connection, permit) = device("stock-media-original", DeviceMode::Normal).await;
    let adapter = device.adapter.clone();
    let bridge = PublicPcmuBridge::start(host_audio, device.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            root_audio.send_pcmu(&[0x1f; 160]).unwrap();
            let frame = root_audio.receive_pcmu().await.unwrap();
            if frame.iter().all(|b| (decode(*b) - 4096).abs() < 256)
                && (adapter.metrics(&connection).unwrap().last_sample + 8192).abs() < 256
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!((adapter.metrics(&connection).unwrap().last_sample + 8192).abs() < 256);
    assert_eq!(
        adapter
            .observe("stock-media-original", &permit)
            .unwrap()
            .effect_attempts,
        1
    );
    {
        let waiting = bridge.drain();
        tokio::pin!(waiting);
        tokio::select! {result=&mut waiting=>panic!("cleanup returned before device drain: {result:?}"),_=device.drain_entered.notified()=>{}}
    }
    device.drain_release.notify_one();
    let report = bridge.drain().await.unwrap();
    assert!(report.cleanup_confirmed(), "{report:?}");
    assert!(report.native_submissions > 0 && report.device_submissions > 0);
    assert_eq!(bridge.drain().await.unwrap(), report);
    assert_eq!(device.drains.load(Ordering::SeqCst), 1);
    assert!(adapter
        .originate(
            OriginateRequest::new(
                SessionId::from_string("local-fixture-session"),
                ParticipantId::from_string("local-fixture-speaker"),
                "fixture:duplex",
                Direction::Outbound,
                adapter.capabilities()
            )
            .with_transport(Transport::InProcessAi)
            .with_context(Admission {
                operation_id: "stock-media-original".into(),
                permit
            })
        )
        .await
        .is_err());
    root_audio.close();
    root_audio.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

async fn scene() -> (
    setup::Setup,
    RunningCanonicalServer,
    CanonicalClientConfig,
    CanonicalClientConfig,
) {
    let mut f = setup::setup("local-contributor", "isolated-fixture-key").await;
    f.config.provider_messages = None;
    let server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    let mut admin = ApplicationClient::open(setup::http(&server, &f.operator.credential))
        .await
        .unwrap();
    let ai = f.route.identity_id.clone();
    let recipients = vec![f.operator.identity_id.clone(), ai.clone()];
    let root = peer(
        &f,
        &server,
        &mut admin,
        &f.operator.identity_id,
        "native-root",
        &recipients,
    )
    .await;
    let host = peer(&f, &server, &mut admin, &ai, "native-host", &recipients).await;
    let mut caller = CanonicalHttpClient::open(setup::http(&server, &root.credential))
        .await
        .unwrap();
    caller
        .session_command(Request {
            operation_id: "media-host-invite".into(),
            conversation_id: f.cid.clone(),
            session_id: "media-host-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![f.ai_pid.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let mut accepting = CanonicalHttpClient::open(setup::http(&server, &host.credential))
        .await
        .unwrap();
    accepting
        .session_command(Request {
            operation_id: "media-host-accept".into(),
            conversation_id: f.cid.clone(),
            session_id: "media-host-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    (f, server, root, host)
}
async fn device(
    operation: &str,
    mode: DeviceMode,
) -> (Arc<Device>, rvoip_core::ids::ConnectionId, DispatchPermit) {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let events = adapter.subscribe_orchestrator_events();
    // Local synthetic-device custody does not grant any Conversation rights.
    let custody = Arc::new(LocalCustody {
        current: AtomicBool::new(true),
        until: Instant::now() + Duration::from_secs(10),
    });
    let permit = DispatchPermit::from_host(custody.clone());
    let request = OriginateRequest::new(
        SessionId::from_string("local-fixture-session"),
        ParticipantId::from_string("local-fixture-speaker"),
        "fixture:duplex",
        Direction::Outbound,
        adapter.capabilities(),
    )
    .with_transport(Transport::InProcessAi)
    .with_context(Admission {
        operation_id: operation.into(),
        permit: permit.clone(),
    });
    let handle = adapter.originate(request.clone()).await.unwrap();
    assert_eq!(
        adapter.originate(request).await.unwrap().connection.id,
        handle.connection.id
    );
    adapter
        .activate_outbound(handle.connection.id.clone())
        .await
        .unwrap();
    let stream = adapter
        .streams(handle.connection.id.clone())
        .await
        .unwrap()
        .remove(0);
    let device = Arc::new(Device {
        adapter,
        custody,
        input: Mutex::new(stream.try_frames_in().unwrap()),
        output: stream.try_frames_out().unwrap(),
        stream: stream.id(),
        clock: AtomicU32::new(0),
        cancelled: Notify::new(),
        mode,
        _events: Mutex::new(events),
        drain_entered: Notify::new(),
        drain_release: Notify::new(),
        drains: AtomicUsize::new(0),
    });
    (device, handle.connection.id, permit)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_host_refuses_malformed_device_frames_and_drains_the_real_peer() {
    let (f, server, _root, host) = scene().await;
    let audio = audio(host, &f.cid, "malformed-native").await;
    let (device, _, _) = device("malformed-device", DeviceMode::Short).await;
    device.drain_release.notify_one();
    let bridge = PublicPcmuBridge::start(audio, device.clone());
    tokio::time::timeout(Duration::from_secs(3), device.cancelled.notified())
        .await
        .unwrap();
    let report = bridge.drain().await.unwrap();
    assert_eq!(
        report.exit,
        connect_client::canonical::PcmuBridgeExit::InvalidFrame
    );
    assert_eq!(report.native_submissions, 0);
    assert_eq!(report.device_submissions, 0);
    assert!(report.cleanup_confirmed(), "{report:?}");
    assert_eq!(bridge.drain().await.unwrap(), report);
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_host_worker_failure_cancels_device_and_repeated_drain_never_repolls_failed_join() {
    let (f, server, _root, host) = scene().await;
    let audio = audio(host, &f.cid, "failed-worker-native").await;
    let (device, _, _) = device("failed-worker-device", DeviceMode::Panic).await;
    let bridge = PublicPcmuBridge::start(audio, device.clone());
    tokio::time::timeout(Duration::from_secs(3), device.cancelled.notified())
        .await
        .unwrap();
    assert!(matches!(bridge.drain().await, Err(ClientError::Transport)));
    assert!(matches!(bridge.drain().await, Err(ClientError::Transport)));
    device.drain_release.notify_one();
    device.drain().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_host_session_isolation_has_an_authorized_positive_control_and_excludes_fixture_audio(
) {
    let (f, server, root, host) = scene().await;
    let mut admin = ApplicationClient::open(setup::http(&server, &f.operator.credential))
        .await
        .unwrap();
    let other =
        connect_client::ClientConfig::load(&f.temp.path().join("state/agent.toml")).unwrap();
    let third = peer(
        &f,
        &server,
        &mut admin,
        &other.identity_id,
        "isolated-third",
        &[f.operator.identity_id.clone(), other.identity_id.clone()],
    )
    .await;
    assert!(try_audio(
        third.clone(),
        &f.cid,
        "media-host-session",
        "uninvited-third"
    )
    .await
    .is_err());
    let other_root = peer(
        &f,
        &server,
        &mut admin,
        &f.operator.identity_id,
        "isolation-root",
        &[f.operator.identity_id.clone(), other.identity_id.clone()],
    )
    .await;
    let mut other_control = CanonicalHttpClient::open(setup::http(&server, &third.credential))
        .await
        .unwrap();
    other_control
        .session_command(Request {
            operation_id: "isolation-invite".into(),
            conversation_id: f.cid.clone(),
            session_id: "isolation-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![f.root_pid.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let mut root_control = CanonicalHttpClient::open(setup::http(&server, &other_root.credential))
        .await
        .unwrap();
    root_control
        .session_command(Request {
            operation_id: "isolation-accept".into(),
            conversation_id: f.cid.clone(),
            session_id: "isolation-session".into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let mut root_audio = audio(root, &f.cid, "private-root").await;
    let mut host_audio = audio(host, &f.cid, "private-fixture").await;
    select(&mut root_audio, &f.ai_pid, "private-hear-fixture").await;
    select(&mut host_audio, &f.root_pid, "private-hear-root").await;
    let mut control_audio = try_audio(other_root, &f.cid, "isolation-session", "control-root")
        .await
        .unwrap();
    let mut third_audio = try_audio(third, &f.cid, "isolation-session", "control-third")
        .await
        .unwrap();
    select(&mut third_audio, &f.root_pid, "third-hear-authorized-root").await;
    select(
        &mut control_audio,
        &f.peer_pid,
        "root-hear-authorized-third",
    )
    .await;
    let (device, connection, _) = device("private-fixture-original", DeviceMode::Normal).await;
    let bridge = PublicPcmuBridge::start(host_audio, device.clone());
    let mut positive = 0;
    let mut private_positive = 0;
    let mut observed = 0;
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while observed < 40 || positive == 0 || private_positive == 0 {
            tokio::select! {
                _ = tick.tick() => {
                    root_audio.send_pcmu(&[0x1f; 160]).unwrap();
                    control_audio.send_pcmu(&[0x2f; 160]).unwrap();
                    third_audio.send_pcmu(&[0xff; 160]).unwrap();
                }
                frame = third_audio.receive_pcmu() => {
                    let frame = frame.unwrap();
                    assert!(
                        frame.iter().all(|b| decode(*b) < 1000),
                        "the excluded Session must not hear the positive fixture marker"
                    );
                    if frame.iter().all(|b| (decode(*b) + 4096).abs() < 256) {
                        positive += 1;
                    }
                    observed += 1;
                }
                heard = root_audio.receive_pcmu() => {
                    if heard.unwrap().iter().all(|b| (decode(*b) - 4096).abs() < 256) {
                        private_positive += 1;
                    }
                }
                control = control_audio.receive_pcmu() => { control.unwrap(); }
            }
        }
    })
    .await
    .unwrap();
    assert!(
        positive > 0,
        "the isolated peer must positively decode its own authorized Session"
    );
    assert!(
        private_positive > 0,
        "the original Session must still decode the fixture marker"
    );
    assert!(device.adapter.metrics(&connection).unwrap().decoded > 0);
    device.drain_release.notify_one();
    assert!(bridge.drain().await.unwrap().cleanup_confirmed());
    root_audio.close();
    root_audio.shutdown().await.unwrap();
    control_audio.close();
    control_audio.shutdown().await.unwrap();
    third_audio.close();
    third_audio.shutdown().await.unwrap();
    drop(admin);
    drop(other_control);
    drop(root_control);
    server.shutdown().await.unwrap();
}
