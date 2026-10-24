//! Real production canonical Core + provisioned telephone route + actual UDP peer.
#[path = "telephone/configured.rs"]
mod configured;
#[path = "telephone/dtmf.rs"]
mod dtmf;
#[path = "telephone/initial.rs"]
mod initial;
#[path = "telephone/private_primary.rs"]
mod private_primary;
#[path = "telephone/received.rs"]
mod received;
#[path = "telephone/restart.rs"]
mod restart;
#[path = "telephone/retained.rs"]
mod retained;
#[path = "telephone/return_native.rs"]
mod return_native;
#[path = "telephone/roles.rs"]
mod roles;
#[path = "telephone/streams.rs"]
mod streams;
use super::*;
use connect_service::TelephoneReplacementRoute;
use connect_store::TelephoneReplacementPolicy;
use rvoip_core::ConnectionAdapter;
use rvoip_sip::{Config as SipConfig, SipAdapter, SipOriginateContext, UnifiedCoordinator};
use rvoip_sip_core::{
    parser::parse_message,
    prelude::{HeaderName, Message, Method, StatusCode},
    types::headers::HeaderAccess,
};
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;
#[derive(Clone)]
struct Dialog {
    remote: std::net::SocketAddr,
    from: String,
    local_tag: String,
    offer: String,
    to: String,
    call_id: String,
    initial_uri: String,
}
struct PeerSip {
    socket: Arc<UdpSocket>,
    dialog: Arc<Mutex<Option<Dialog>>>,
    info_sequence: std::sync::atomic::AtomicU32,
    rtp_sequence: std::sync::atomic::AtomicU16,
    rtp_clock: tokio::time::Instant,
    address: std::net::SocketAddr,
    calls: Arc<Mutex<Vec<String>>>,
    methods: Arc<Mutex<Vec<Method>>>,
    info: Arc<Mutex<Vec<Vec<u8>>>>,
    info_responses: Arc<Mutex<Vec<(u32, String)>>>,
    reject_info: Arc<std::sync::atomic::AtomicBool>,
    reject_refresh: Arc<std::sync::atomic::AtomicBool>,
    ledger: Arc<Mutex<Option<std::path::PathBuf>>>,
    media: UdpSocket,
    media_target: Arc<Mutex<Option<std::net::SocketAddr>>>,
    task: tokio::task::JoinHandle<()>,
}
impl PeerSip {
    async fn new() -> Self {
        Self::with_audio(None).await
    }
    async fn with_audio(payload: Option<u8>) -> Self {
        Self::with_audio_for_user(payload, "+14155550123").await
    }
    async fn with_audio_for_user(payload: Option<u8>, user: &str) -> Self {
        let expected_user = user.to_owned();
        let media = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let media_port = media.local_addr().unwrap().port();
        let media_target = Arc::new(Mutex::new(None));
        let capture_target = media_target.clone();
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let wire = socket.clone();
        let dialog = Arc::new(Mutex::new(None::<Dialog>));
        let seen_dialog = dialog.clone();
        let address = socket.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let info = Arc::new(Mutex::new(Vec::new()));
        let info_seen = info.clone();
        let info_responses = Arc::new(Mutex::new(Vec::new()));
        let responses_seen = info_responses.clone();
        let reject_info = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let info_rejected = reject_info.clone();
        let ledger = Arc::new(Mutex::new(None::<std::path::PathBuf>));
        let recorded = ledger.clone();
        let captured = calls.clone();
        let seen = methods.clone();
        let reject_refresh = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reject = reject_refresh.clone();
        let task = tokio::spawn(async move {
            let mut data = [0; 16384];
            let mut original = None;
            loop {
                let (n, from) = wire.recv_from(&mut data).await.unwrap();
                let Ok(message) = parse_message(&data[..n]) else {
                    continue;
                };
                let request = match message {
                    Message::Request(request) => request,
                    Message::Response(response) => {
                        if let Some(value) = response.raw_header_value(&HeaderName::CSeq) {
                            let mut fields = value.split_whitespace();
                            if let (Some(sequence), Some("INFO")) = (fields.next(), fields.next()) {
                                let mut responses = responses_seen.lock().unwrap();
                                assert!(responses.len() < 256);
                                responses.push((
                                    sequence.parse::<u32>().unwrap(),
                                    format!("{:?}", response.status),
                                ));
                            }
                        }
                        continue;
                    }
                };
                seen.lock().unwrap().push(request.method());
                let mut response = match request.method() {
                    Method::Invite => {
                        let path = recorded
                            .lock()
                            .unwrap()
                            .clone()
                            .expect("ledger installed before callback");
                        let db = rusqlite::Connection::open_with_flags(
                            path,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                        )
                        .unwrap();
                        let durable: bool = db.query_row(
                            "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.phase='start' AND s.started=1 AND e.state='submitting' AND e.attempt=1 AND e.correlation_key=?1 AND EXISTS(SELECT 1 FROM native_replacement_steps a JOIN operation_effects ae ON ae.id=a.effect_id WHERE a.operation_record_id=s.operation_record_id AND a.phase='allocate' AND ae.state='confirmed' AND json_extract(a.observation_json,'$.call_reference.value')=?1)) OR EXISTS(SELECT 1 FROM initial_telephone_operations i JOIN operation_effects e ON e.id=i.effect_id WHERE i.signaling_started=1 AND i.runtime_connection_id IS NOT NULL AND i.sip_call_id=?1 AND e.state='submitting' AND e.attempt=1 AND e.provider_ref=?1 AND e.correlation_key=i.operation_record_id)",
                            [request.call_id().unwrap().value()], |r|r.get(0),
                        ).unwrap();
                        if request.to().and_then(|to| to.tag()).is_some() {
                            let established = seen_dialog
                                .lock()
                                .unwrap()
                                .clone()
                                .expect("refresh needs an established dialog");
                            assert_eq!(request.call_id().unwrap().value(), established.call_id);
                            assert_eq!(
                                request.from().and_then(|from| from.tag()).unwrap(),
                                established.local_tag
                            );
                            assert_eq!(
                                request.to().and_then(|to| to.tag()),
                                Some("canonical-telephone-peer")
                            );
                            assert!(!durable, "refresh reused the original submission interval");
                            assert_eq!(request.uri.user.as_deref(), Some("peer"));
                        } else {
                            assert!(
                                durable,
                                "SIP IO preceded durable exact allocation/submission correlation"
                            );
                            assert_eq!(request.uri.user.as_deref(), Some(expected_user.as_str()));
                            original = Some(request.clone());
                        }
                        assert_eq!(request.uri.host_port(), address.to_string());
                        captured
                            .lock()
                            .unwrap()
                            .push(request.call_id().unwrap().value());
                        rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                            &request,
                            if request.to().and_then(|to| to.tag()).is_some()
                                && reject.load(std::sync::atomic::Ordering::Acquire)
                            {
                                StatusCode::NotAcceptableHere
                            } else if payload.is_some() {
                                StatusCode::Ok
                            } else {
                                StatusCode::Ringing
                            },
                        )
                    }
                    Method::Info => {
                        let established = seen_dialog.lock().unwrap().clone().unwrap();
                        assert_eq!(request.call_id().unwrap().value(), established.call_id);
                        assert_eq!(request.uri.user.as_deref(), Some("peer"));
                        assert_eq!(request.uri.host_port(), address.to_string());
                        let path = recorded.lock().unwrap().clone().unwrap();
                        let db = rusqlite::Connection::open_with_flags(
                            path,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                        )
                        .unwrap();
                        let admitted: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM native_dtmf_deliveries g JOIN operations o ON o.record_id=g.operation_record_id WHERE g.target_kind IN ('sip-initial','sip-established') AND json_extract(g.target_json,'$.sip_call_id')=?1 AND g.attempted=1 AND g.frame_id IS NOT NULL AND g.disposition IS NULL AND o.state='preparing' AND o.command_type='dtmf.send' AND json_extract(o.intent_json,'$.payload.method')='info')",[&established.call_id],|r|r.get(0)).unwrap();
                        assert!(admitted, "SIP INFO preceded exact durable signal admission");
                        info_seen.lock().unwrap().push(data[..n].to_vec());
                        rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                            &request,
                            if info_rejected.load(std::sync::atomic::Ordering::Acquire) {
                                StatusCode::Forbidden
                            } else {
                                StatusCode::Ok
                            },
                        )
                    }
                    Method::Cancel | Method::Bye => {
                        rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                            &request,
                            StatusCode::Ok,
                        )
                    }
                    Method::Ack => continue,
                    _ => panic!("unexpected fixture request"),
                };
                if request.method() == Method::Invite
                    && payload.is_some()
                    && response.status == StatusCode::Ok
                {
                    let pt = payload.unwrap();
                    let codec = if pt == 0 { "PCMU" } else { "PCMA" };
                    let port = std::str::from_utf8(&request.body)
                        .unwrap()
                        .lines()
                        .find_map(|l| l.strip_prefix("m=audio "))
                        .unwrap()
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse::<u16>()
                        .unwrap();
                    *capture_target.lock().unwrap() =
                        Some(std::net::SocketAddr::new(from.ip(), port));
                    if let Some(rvoip_sip_core::TypedHeader::To(to)) = response
                        .headers
                        .iter_mut()
                        .find(|h| matches!(h, rvoip_sip_core::TypedHeader::To(_)))
                    {
                        to.set_tag("canonical-telephone-peer");
                    }
                    let mut observed_dialog = seen_dialog.lock().unwrap();
                    let initial_uri = observed_dialog
                        .as_ref()
                        .map_or_else(|| request.uri.to_string(), |old| old.initial_uri.clone());
                    *observed_dialog = Some(Dialog {
                        remote: from,
                        from: request.raw_header_value(&HeaderName::From).unwrap(),
                        local_tag: request.from().and_then(|from| from.tag()).unwrap().into(),
                        offer: std::str::from_utf8(&request.body).unwrap().into(),
                        to: response.raw_header_value(&HeaderName::To).unwrap(),
                        call_id: request.call_id().unwrap().value(),
                        initial_uri,
                    });
                    drop(observed_dialog);
                    response.headers.push(rvoip_sip_core::TypedHeader::Contact(
                        format!("<sip:peer@{address}>").parse().unwrap(),
                    ));
                    response
                        .headers
                        .push(rvoip_sip_core::TypedHeader::ContentType(
                            rvoip_sip_core::types::ContentType::sdp(),
                        ));
                    response.body=format!("v=0\r\no=canonical-telephone 1 1 IN IP4 127.0.0.1\r\ns=canonical-telephone\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP {pt} 101\r\na=rtpmap:{pt} {codec}/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n").into();
                    response
                        .headers
                        .retain(|h| !matches!(h, rvoip_sip_core::TypedHeader::ContentLength(_)));
                    response
                        .headers
                        .push(rvoip_sip_core::TypedHeader::ContentLength(
                            rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                        ));
                }
                wire.send_to(&Message::Response(response).to_bytes(), from)
                    .await
                    .unwrap();
                if request.method() == Method::Cancel {
                    let response =
                        rvoip_sip_dialog::transaction::utils::response_builders::create_response(
                            original.as_ref().unwrap(),
                            StatusCode::RequestTerminated,
                        );
                    wire.send_to(&Message::Response(response).to_bytes(), from)
                        .await
                        .unwrap();
                }
            }
        });
        Self {
            socket,
            dialog,
            info_sequence: std::sync::atomic::AtomicU32::new(2),
            rtp_sequence: std::sync::atomic::AtomicU16::new(1),
            rtp_clock: tokio::time::Instant::now(),
            address,
            calls,
            methods,
            info,
            info_responses,
            reject_info,
            reject_refresh,
            ledger,
            media,
            media_target,
            task,
        }
    }
    async fn digit(&self, digit: &str) -> Vec<u8> {
        let (remote, from, to, call_id) = {
            let guard = self.dialog.lock().unwrap();
            let d = guard.as_ref().unwrap();
            (d.remote, d.to.clone(), d.from.clone(), d.call_id.clone())
        };
        let body = format!("Signal={digit}\r\nDuration=160\r\n");
        let sequence = self
            .info_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let message=format!("INFO sip:replacement@{remote} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bK-connect-{};rport\r\nMax-Forwards: 70\r\nFrom: {from}\r\nTo: {to}\r\nCall-ID: {call_id}\r\nCSeq: {sequence} INFO\r\nContent-Type: application/dtmf-relay\r\nContent-Length: {}\r\n\r\n{body}",self.address,uuid::Uuid::new_v4().simple(),body.len());
        self.socket
            .send_to(message.as_bytes(), remote)
            .await
            .unwrap();
        message.into_bytes()
    }
    async fn rtp(&self, payload: u8) {
        self.rtp_pattern(payload, 16).await;
    }
    async fn rtp_pattern(&self, payload: u8, period: usize) {
        assert!(period > 1 && 160 % period == 0);
        let target = self.media_target.lock().unwrap().unwrap();
        for _ in 0..30u16 {
            let sequence = self
                .rtp_sequence
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut packet = vec![0x80, payload];
            packet.extend_from_slice(&sequence.to_be_bytes());
            packet.extend_from_slice(
                &((self.rtp_clock.elapsed().as_micros() * 8 / 1000) as u32).to_be_bytes(),
            );
            packet.extend_from_slice(&0x19374621u32.to_be_bytes());
            packet.extend((0..160).map(|i| if i % period < period / 2 { 0x80 } else { 0x00 }));
            self.media.send_to(&packet, target).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    fn unique(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }
}
impl Drop for PeerSip {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn sip_audio(peer: &PeerSip, payload: u8, duration: Duration) -> usize {
    let until = tokio::time::Instant::now() + duration;
    let mut bytes = [0; 4096];
    let mut marked = 0;
    while let Ok(Ok((n, _))) =
        tokio::time::timeout_at(until, peer.media.recv_from(&mut bytes)).await
    {
        if n < 12 || bytes[0] >> 6 != 2 || (200..=211).contains(&bytes[1]) {
            continue;
        }
        assert_eq!(bytes[1] & 0x7f, payload);
        let mut header = 12 + usize::from(bytes[0] & 15) * 4;
        if bytes[0] & 0x10 != 0 {
            header +=
                4 + 4 * usize::from(u16::from_be_bytes([bytes[header + 2], bytes[header + 3]]));
        }
        let end = if bytes[0] & 0x20 != 0 {
            n - usize::from(bytes[n - 1])
        } else {
            n
        };
        assert!(header <= end);
        if bytes[header..end].iter().any(|b| {
            if payload == 0 {
                !matches!(*b, 0xff | 0x7f)
            } else {
                !matches!(*b, 0xd5 | 0x55)
            }
        }) {
            marked += 1;
        }
    }
    marked
}
async fn sip_native_duplex(peer: &PeerSip, peers: &mut [Peer], payload: u8, name: &str) {
    let receive = async {
        let until = tokio::time::Instant::now() + Duration::from_millis(900);
        let mut marked = 0;
        while let Ok(Ok(bytes)) =
            tokio::time::timeout_at(until, peers[0].client.receive_native_rtp_datagram()).await
        {
            let packet = rvoip_uctp::substrate::unpack_rtp_datagram(&bytes).unwrap();
            assert_eq!(packet.stream_local_id, peers[0].handle);
            if packet
                .rtp
                .payload
                .iter()
                .any(|b| !matches!(*b, 0xff | 0x7f))
            {
                marked += 1;
            }
        }
        marked
    };
    let (_, marked) = tokio::join!(peer.rtp(payload), receive);
    eprintln!("SIP/native audio {name}: direction=sip-to-native marked={marked}");
    assert!(
        marked > 10,
        "{name}: SIP input did not reach retained native peer: {marked}"
    );
    let (_, marked) = tokio::join!(
        probe(peers, 0, &[], name),
        sip_audio(peer, payload, Duration::from_millis(2600))
    );
    eprintln!("SIP/native audio {name}: direction=native-to-sip marked={marked}");
    assert!(
        marked > 10,
        "{name}: retained native input did not reach SIP target: {marked}"
    );
}
async fn private_input_isolation(peers: &mut [Peer]) {
    let until = tokio::time::Instant::now() + Duration::from_millis(900);
    loop {
        let receives = peers
            .iter_mut()
            .enumerate()
            .filter(|(_, p)| p.live && p.handle != 0)
            .map(|(i, p)| {
                Box::pin(async move { (i, p.client.receive_native_rtp_datagram().await) })
            })
            .collect::<Vec<_>>();
        tokio::select! {
            _=tokio::time::sleep_until(until)=>break,
            ((i,bytes),_,_)=select_all(receives)=>{
                let packet=rvoip_uctp::substrate::unpack_rtp_datagram(&bytes.unwrap()).unwrap();
                assert!(packet.rtp.payload.iter().all(|b| matches!(*b,0xff|0x7f)),"private SIP input leaked to native receiver {i}");
            }
        }
    }
}
async fn until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_replacement_uses_same_core_records_call_before_io_and_releases_ring_readers_on_revocation(
) {
    ringing_case(Stop::RouteCredential).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_replacement_public_cancel_stops_one_call_preserves_original_and_replays_without_redial(
) {
    ringing_case(Stop::PublicCancel).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_replacement_no_answer_deadline_stops_one_call_without_changing_original_route(
) {
    ringing_case(Stop::Deadline).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_replacement_endpoint_revocation_stops_one_call_and_preserves_original_duplex(
) {
    ringing_case(Stop::Endpoint).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_movement_actual_pcmu_commits_once_survives_old_authority_and_retires_on_route_loss(
) {
    ringing_case(Stop::AnsweredPcmu).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_movement_actual_pcma_preserves_negotiated_codec_and_independent_lifetime(
) {
    ringing_case(Stop::AnsweredPcma).await;
}
#[derive(Clone, Copy)]
enum Stop {
    RouteCredential,
    PublicCancel,
    Deadline,
    Endpoint,
    AnsweredPcmu,
    AnsweredPcma,
    Shutdown,
    Voicemail,
    RejectedConfirmation,
    PrivateCancel,
    PrivateRestart,
    CommittedRestart,
    PrivateDrop,
    CommittedDrop,
    LostReply,
    StreamControl,
    Dtmf,
    DtmfReceive,
    DtmfRefresh,
    DtmfPrivacy,
    DtmfRetained,
    DtmfPrimary,
    ReturnNative,
    ReturnObserver,
    Refresh,
}
async fn ringing_case(stop: Stop) {
    telephone_case(stop, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_movement_loads_toml_and_drains_owned_sip_listener() {
    telephone_case(Stop::AnsweredPcmu, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_cancel_preserves_original_duplex_and_drains_owned_sip_listener(
) {
    telephone_case(Stop::PublicCancel, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_shutdown_cancels_the_actual_ringing_callback() {
    telephone_case(Stop::Shutdown, true).await;
}
async fn telephone_case(stop: Stop, configured: bool) {
    telephone_case_profile(stop, configured, "EdDSA").await;
}
async fn telephone_case_profile(stop: Stop, configured: bool, algorithm: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "connect_service=debug".into()),
        )
        .with_test_writer()
        .try_init();
    let answered = match stop {
        Stop::AnsweredPcmu
        | Stop::Voicemail
        | Stop::RejectedConfirmation
        | Stop::PrivateCancel
        | Stop::PrivateRestart
        | Stop::CommittedRestart
        | Stop::PrivateDrop
        | Stop::CommittedDrop
        | Stop::StreamControl
        | Stop::Dtmf
        | Stop::DtmfReceive
        | Stop::DtmfRefresh
        | Stop::DtmfPrivacy
        | Stop::DtmfRetained
        | Stop::DtmfPrimary
        | Stop::ReturnNative
        | Stop::ReturnObserver
        | Stop::Refresh => Some(0),
        Stop::AnsweredPcma => Some(8),
        _ => None,
    };
    let peer = if answered.is_some() {
        PeerSip::with_audio(answered).await
    } else {
        PeerSip::new().await
    };
    let sip_bind = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let coordinator = if configured {
        None
    } else {
        Some(
            UnifiedCoordinator::new(SipConfig::local("connect-callback-fixture", 0))
                .await
                .unwrap(),
        )
    };
    let adapter = match &coordinator {
        Some(c) => Some(SipAdapter::new(c.clone()).await.unwrap()),
        None => None,
    };
    let configure = |f: &Fixture| {
        let store = &f.server.store;
        let admin = store.authenticate(&f.admin).unwrap();
        let ProvisioningOutcome::Identity {
            identity_id: owner, ..
        } = store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::Identity {
                    operation_id: "telephone-route-owner".into(),
                    alias: "telephone-route-owner".into(),
                    kind: IdentityKind::System,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        let route_key = store
            .issue(
                &owner,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
            .execute_provisioning(
                &route_key.id,
                ProvisioningCommand::Endpoint {
                    operation_id: "telephone-endpoint".into(),
                    identity_id: f.client.identity_id.clone(),
                    device: DeviceSpec {
                        id: "callback-device".into(),
                        kind: "telephone".into(),
                        platform: "isolated-production-fixture".into(),
                    },
                    transport: EndpointTransport::Telephone,
                    address: "+14155550123".into(),
                    capabilities: vec!["audio".into()],
                    secret_ref: Some("config_test_callback".into()),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .execute_provisioning(
                &route_key.id,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: "verify-telephone".into(),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: 0,
                    evidence_ref: "isolated-fixture".into(),
                },
            )
            .unwrap();
        let route = TelephoneReplacementRoute {
            credential_id: route_key.id.clone(),
            policy: TelephoneReplacementPolicy {
                route_reference: "config_test_callback".into(),
                sip_authority: peer.address.to_string(),
                confirmation_digits: "55#".into(),
                timeout_seconds: if matches!(
                    stop,
                    Stop::Deadline
                        | Stop::LostReply
                        | Stop::AnsweredPcmu
                        | Stop::AnsweredPcma
                        | Stop::Voicemail
                        | Stop::RejectedConfirmation
                        | Stop::PrivateCancel
                        | Stop::PrivateRestart
                        | Stop::CommittedRestart
                        | Stop::PrivateDrop
                        | Stop::CommittedDrop
                        | Stop::StreamControl
                        | Stop::Dtmf
                        | Stop::DtmfReceive
                        | Stop::DtmfRefresh
                        | Stop::DtmfPrivacy
                        | Stop::DtmfRetained
                        | Stop::DtmfPrimary
                        | Stop::ReturnNative
                        | Stop::ReturnObserver
                        | Stop::Refresh
                ) {
                    10
                } else {
                    30
                },
            },
            originate: SipOriginateContext::default(),
        };
        if let Some(adapter) = &adapter {
            f.server
                .service
                .install_telephone_replacement_adapter(adapter.clone(), vec![route.clone()])
                .unwrap();
        }
        assert_ne!(owner, f.client.identity_id);
        (
            (endpoint_id, route_key, owner),
            connect_server::canonical_telephone::CanonicalTelephoneConfig {
                bind: sip_bind,
                media_port_start: 20000,
                media_port_end: 20063,
                outbound: None,
                initial: vec![],
                routes: vec![
                    connect_server::canonical_telephone::CanonicalTelephoneRoute {
                        credential_id: route.credential_id,
                        policy: route.policy,
                    },
                ],
            },
        )
    };
    let (
        codecs::Running {
            f,
            mut clients,
            cid,
            sh,
            th,
        },
        (endpoint, route_key, route_owner),
    ) = if configured {
        if matches!(
            stop,
            Stop::Dtmf
                | Stop::DtmfReceive
                | Stop::DtmfRefresh
                | Stop::DtmfPrivacy
                | Stop::DtmfRetained
                | Stop::DtmfPrimary
        ) {
            codecs::setup_with_telephone_dtmf(algorithm, configure).await
        } else {
            codecs::setup_with_telephone(configure).await
        }
    } else {
        codecs::setup_with_host(|f| configure(f).0).await
    };
    *peer.ledger.lock().unwrap() = Some(f.config.database.clone());
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let member = |identity: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == identity)
            .unwrap()
            .participant_id
            .clone()
    };
    let owner = member(&f.client.identity_id);
    let remote = member(&f.peer.identity_id);
    let primary = clients.pop().unwrap();
    let other = clients.pop().unwrap();
    let make = |client, identity, participant, id: &str, handle| Peer {
        client,
        config: None,
        identity,
        participant,
        id: id.into(),
        handle,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    };
    let mut peers = vec![
        make(
            other,
            f.peer.identity_id.clone(),
            remote,
            "codec-source",
            sh,
        ),
        make(
            primary,
            f.client.identity_id.clone(),
            owner,
            "codec-target",
            th,
        ),
    ];
    if matches!(stop, Stop::ReturnObserver) {
        roles::sibling(&f, &cid, &mut peers).await;
    }
    probe(&mut peers, 0, &[1], "telephone-before-callback").await;
    if matches!(stop, Stop::StreamControl) {
        let participant = peers[1].participant.clone();
        let expected = stream_revision(&f, "codec-source");
        stream_control(
            &mut peers[0].client,
            &cid,
            "codec-source",
            "telephone-retain-member-before-move",
            expected,
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":participant,"kinds":["audio"]}]}),
        )
        .await;
    }
    let controller_key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(&f);
    cfg.credential = controller_key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let command = TransitionCommand {
        operation_id: "production-telephone-replacement".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some("codec-target".into()),
        expected_revision: revision(&f),
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint.clone()),
            candidate_connid: None,
            retained_connid: "codec-source".into(),
        },
        payload_extensions: Default::default(),
    };
    // Keep a real request socket unread until the actual callback proves
    // admission; then discard that physical reply and recover the logical ID.
    let unread_reply = if matches!(stop, Stop::LostReply) {
        Some(restart::submit_without_read(&f, &controller_key.token, &command).await)
    } else {
        controller.transition(command.clone()).await.unwrap();
        None
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while peer.unique() == 0 {
        if unread_reply.is_some() {
            // The intentionally unread request races the first status poll.
            // Wait for admission rather than treating a private not-found
            // refusal as a failed callback.
            let exists: bool = rusqlite::Connection::open_with_flags(
                &f.config.database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE operation_id=?1 AND owner_id=?2)",
                [&command.operation_id, &f.client.identity_id],
                |r| r.get(0),
            )
            .unwrap();
            if !exists {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "unread HTTP replacement was not admitted"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        }
        let state = status(&f, &controller_key.id, &command.operation_id).await;
        assert!(
            !matches!(
                state.state,
                OperationState::Failed | OperationState::Unknown | OperationState::Cancelled
            ),
            "callback refused before IO: {state:?}"
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "callback did not reach wire: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if let Some(socket) = unread_reply {
        drop(socket); // No response header/body or acceptance ACK was read.
        let raw = peer.calls.lock().unwrap().len();
        let replay = controller.transition(command.clone()).await.unwrap();
        assert_eq!(
            replay.state,
            rvoip_uctp::v2::payloads::OperationState::Preparing
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            peer.calls.lock().unwrap().len(),
            raw,
            "lost command reply caused another INVITE"
        );
    }
    if matches!(stop, Stop::Shutdown) {
        let during = status(&f, &controller_key.id, &command.operation_id).await;
        assert_eq!(during.state, OperationState::Preparing);
        assert_eq!(peer.unique(), 1);
        drop(peers);
        restart::without_redial(
            f,
            command,
            &controller_key.token,
            &peer,
            sip_bind,
            during,
            None,
            false,
        )
        .await;
        return;
    }
    if let Some(payload) = answered {
        let resources_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = status(&f, &controller_key.id, &command.operation_id).await;
            assert_eq!(
                state.state,
                OperationState::Preparing,
                "answered resources stopped: {state:?}"
            );
            if state.effects.iter().any(|e| {
                e.provider == "telephone-replacement-resources" && e.state == EffectState::Confirmed
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < resources_deadline,
                "answered resources not captured: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let db = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let raw:String=db.query_row("SELECT observation_json FROM telephone_replacement_resources WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id=?1)",[&command.operation_id],|r|r.get(0)).unwrap();
        let observation: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            observation["codec"],
            if payload == 0 { "pcmu" } else { "pcma" }
        );
        assert_eq!(observation["payload_type"], payload);
        assert_eq!(observation["clock_rate_hz"], 8000);
        assert_eq!(observation["stream_revision"], 2);
        assert_eq!(observation["prepared_muted"], true);
        assert_eq!(observation["media_ready"], false);
        controller.transition(command.clone()).await.unwrap();
        let raw_after:String=db.query_row("SELECT observation_json FROM telephone_replacement_resources WHERE operation_record_id=(SELECT record_id FROM operations WHERE operation_id=?1)",[&command.operation_id],|r|r.get(0)).unwrap();
        assert_eq!(raw_after, raw);
        // Actual private activation must finish before any local preparation RTP.
        let phase_until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = status(&f, &controller_key.id, &command.operation_id).await;
            assert_eq!(
                state.state,
                OperationState::Preparing,
                "private phase stopped: {state:?}"
            );
            if state.effects.iter().any(|e| {
                e.provider == "telephone-replacement-activate" && e.state == EffectState::Confirmed
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < phase_until,
                "private activation missing: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut packet = [0; 4096];
        let output_until = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let (n, _) = tokio::time::timeout_at(output_until, peer.media.recv_from(&mut packet))
                .await
                .unwrap()
                .unwrap();
            if n >= 12 && packet[0] >> 6 == 2 && packet[1] & 0x7f == payload {
                assert!(
                    packet[12..n]
                        .iter()
                        .all(|b| *b == if payload == 0 { 0xff } else { 0xd5 }),
                    "private preparation was not encoded silence"
                );
                break;
            }
        }
        let readiness = || {
            db.query_row(
                "SELECT count(*) FROM telephone_replacement_phases WHERE phase='readiness'",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
        };
        if matches!(
            stop,
            Stop::Voicemail
                | Stop::RejectedConfirmation
                | Stop::PrivateCancel
                | Stop::PrivateRestart
                | Stop::PrivateDrop
        ) {
            // An answered audio-producing endpoint still needs the configured
            // attributed confirmation; audio is not a human/voicemail classifier.
            tokio::join!(peer.rtp(payload), private_input_isolation(&mut peers));
            assert_eq!(readiness(), 0);
            if matches!(stop, Stop::RejectedConfirmation) {
                for digit in ["9", "9", "#"] {
                    peer.digit(digit).await;
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
            }
            let pending = status(&f, &controller_key.id, &command.operation_id).await;
            assert_eq!(pending.state, OperationState::Preparing);
            assert!(pending.result.is_none());
            assert_eq!(readiness(), 0);
            assert!(!pending
                .effects
                .iter()
                .any(|e| e.provider == "native-replacement-commit"));
            if matches!(stop, Stop::PrivateRestart | Stop::PrivateDrop) {
                drop(peers);
                restart::without_redial(
                    f,
                    command,
                    &controller_key.token,
                    &peer,
                    sip_bind,
                    pending,
                    Some(payload),
                    matches!(stop, Stop::PrivateDrop),
                )
                .await;
                return;
            }
            if matches!(stop, Stop::PrivateCancel) {
                controller
                    .request_value(
                        "operation.cancel",
                        json!({"operation_id":command.operation_id}),
                        CommandScope {
                            conversation_id: Some(cid.clone()),
                            session_id: Some(command.session_id.clone()),
                            connection_id: command.connection_id.clone(),
                            operation_id: Some("cancel-private-telephone".into()),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap()
                    .ack()
                    .unwrap();
            }
            until(|| peer.methods.lock().unwrap().contains(&Method::Bye)).await;
            stopped(
                &f,
                &controller_key.id,
                &command.operation_id,
                OperationState::Unknown,
            )
            .await;
            assert_eq!(readiness(), 0);
            let raw = peer.calls.lock().unwrap().len();
            controller.transition(command).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(peer.calls.lock().unwrap().len(), raw);
            assert_eq!(peer.unique(), 1);
            probe(
                &mut peers,
                0,
                &[1],
                "telephone-unconfirmed-retains-original-remote",
            )
            .await;
            probe(
                &mut peers,
                1,
                &[0],
                "telephone-unconfirmed-retains-original-source",
            )
            .await;
            drop(peers);
            f.server.shutdown().await.unwrap();
            assert_sip_listener_released(sip_bind);
            return;
        }
        // Output alone cannot confirm. An early complete sequence stays ineligible.
        for digit in ["5", "5", "#"] {
            peer.digit(digit).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(readiness(), 0, "output-only confirmation became ready");
        // Actual decoded input establishes the live private IO incarnation.
        tokio::join!(peer.rtp(payload), async {
            private_input_isolation(&mut peers).await;
            peer.digit("9").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(readiness(), 0, "wrong digits became ready");
            let repeated = peer.digit("5").await;
            tokio::time::sleep(Duration::from_millis(80)).await;
            let remote = peer.dialog.lock().unwrap().as_ref().unwrap().remote;
            peer.socket.send_to(&repeated, remote).await.unwrap();
            peer.digit("#").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                readiness(),
                0,
                "retransmitted INFO counted as another digit"
            );
            for digit in ["5", "5", "#"] {
                peer.digit(digit).await;
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
            let ready = tokio::time::timeout(Duration::from_secs(15), async {
                while readiness() != 1 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await;
            if ready.is_err() {
                // Read diagnostics only after the original bound fails. This
                // must not delay positive digits or change their eligibility.
                let failed = status(&f, &controller_key.id, &command.operation_id).await;
                let phases = db
                    .prepare("SELECT phase FROM telephone_replacement_phases ORDER BY phase")
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                let effects = failed
                    .effects
                    .iter()
                    .map(|e| {
                        json!({
                            "provider":e.provider,"state":e.state,"attempt":e.attempt
                        })
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "private SIP confirmation readiness timed out: {}",
                    json!({
                        "algorithm":algorithm,"state":failed.state,
                        "problem_code":failed.problem.as_ref().and_then(|p|p.get("code")),
                        "phases":phases,"ready_count":readiness(),"effects":effects
                    })
                );
            }
        });
        let completed_until = tokio::time::Instant::now() + Duration::from_secs(10);
        let done = loop {
            let state = status(&f, &controller_key.id, &command.operation_id).await;
            if state.state == OperationState::Completed {
                break state;
            }
            assert!(
                !matches!(
                    state.state,
                    OperationState::Unknown | OperationState::Failed | OperationState::Cancelled
                ),
                "telephone movement stopped: {state:?}"
            );
            assert!(
                tokio::time::Instant::now() < completed_until,
                "telephone movement did not complete: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(done.result.as_ref().unwrap()["generation"], 2);
        assert_eq!(done.effects.len(), 8);
        assert!(done
            .effects
            .iter()
            .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
        let candidate: String = db
            .query_row(
                "SELECT candidate_runtime_id FROM native_replacements WHERE operation_record_id=?1",
                [&done.record_id],
                |r| r.get(0),
            )
            .unwrap();
        let candidate = rvoip_core::ids::ConnectionId::from_string(candidate);
        peers[1].live = false;
        sip_native_duplex(&peer, &mut peers, payload, "telephone-after-compound-move").await;
        if matches!(stop, Stop::StreamControl) {
            streams::controls(&f, &cid, &mut peers, &peer, payload, &done).await;
        }
        if matches!(stop, Stop::ReturnNative | Stop::ReturnObserver) {
            let observer = matches!(stop, Stop::ReturnObserver);
            if observer {
                roles::cycle(&f, &cid, &mut peers, &peer, payload, &mut controller).await;
            }
            return_native::run(
                &f,
                &cid,
                &mut peers,
                &peer,
                payload,
                &mut controller,
                &controller_key.id,
                &route_key.id,
                &command,
                &done,
                observer,
            )
            .await;
            drop(peers);
            f.server.shutdown().await.unwrap();
            assert_sip_listener_released(sip_bind);
            return;
        }
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let old = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        f.server.store.revoke(&admin, &old).unwrap();
        f.server.store.revoke(&admin, &controller_key.id).unwrap();
        let reader = f
            .server
            .store
            .issue(
                &f.client.identity_id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut cfg = canonical_http_config(&f);
        cfg.credential = reader.token.clone();
        controller = CanonicalHttpClient::open(cfg).await.unwrap();
        controller
            .request_value(
                "operation.cancel",
                json!({"operation_id":command.operation_id}),
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    session_id: Some("native-session".into()),
                    connection_id: command.connection_id.clone(),
                    operation_id: Some("late-cancel-moved-telephone".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .ack()
            .unwrap();
        let invites = peer.calls.lock().unwrap().len();
        controller.transition(command.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            peer.calls.lock().unwrap().len(),
            invites,
            "completed replay wrote another INVITE"
        );
        sip_native_duplex(
            &peer,
            &mut peers,
            payload,
            "telephone-independent-of-old-authority",
        )
        .await;
        assert_eq!(
            status(&f, &reader.id, &command.operation_id).await.state,
            OperationState::Completed
        );
        let expired: i64 = db
            .query_row(
                "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
                [&done.record_id],
                |r| r.get(0),
            )
            .unwrap();
        while chrono::Utc::now().timestamp() <= expired {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        sip_native_duplex(
            &peer,
            &mut peers,
            payload,
            "telephone-after-original-preparation-expiry",
        )
        .await;
        if matches!(stop, Stop::Dtmf) {
            dtmf::exercise(&f, &cid, &mut peers, &peer, payload, &done, &route_key.id).await;
        }
        if matches!(stop, Stop::DtmfReceive) {
            received::exercise(
                &f,
                &cid,
                &mut peers,
                &peer,
                payload,
                &done,
                &route_key.id,
                algorithm,
            )
            .await;
        }
        if matches!(stop, Stop::DtmfRefresh) {
            received::refresh::exercise(&f, &cid, &mut peers, &peer, payload, &done, algorithm)
                .await;
        }
        if matches!(stop, Stop::DtmfPrimary) {
            received::private_primary::exercise(&f, &peer, &mut peers, &cid, algorithm, &done)
                .await;
            drop(peers);
            f.server.shutdown().await.unwrap();
            assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
            assert_eq!(peer.unique(), 1);
            assert!(db.query_row("SELECT retired FROM telephone_connection_lifetimes WHERE operation_record_id=?1 AND generation=2",[&done.record_id],|r|r.get::<_,bool>(0)).unwrap());
            assert_sip_listener_released(sip_bind);
            return;
        }
        if matches!(stop, Stop::DtmfRetained) {
            received::retained::exercise(&f, &peer, &mut peers, &cid, algorithm, &done).await;
            drop(peers);
            f.server.shutdown().await.unwrap();
            assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
            assert_eq!(peer.unique(), 1);
            assert_eq!(db.query_row("SELECT retired FROM telephone_connection_lifetimes WHERE operation_record_id=?1 AND generation=2",[&done.record_id],|r|r.get::<_,bool>(0)).unwrap(),true);
            assert_sip_listener_released(sip_bind);
            return;
        }
        if matches!(stop, Stop::DtmfPrivacy) {
            received::privacy::exercise(&f, &peer, &mut peers, &cid, algorithm, &done).await;
            drop(peers);
            f.server.shutdown().await.unwrap();
            assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
            assert_eq!(peer.unique(), 1);
            assert_sip_listener_released(sip_bind);
            return;
        }
        let refresh_session = if matches!(stop, Stop::Refresh) {
            let coordinator = coordinator.as_ref().unwrap();
            let sessions = coordinator.list_sessions().await;
            assert_eq!(sessions.len(), 1, "one actual SIP callback session");
            let session = coordinator.session(&sessions[0].session_id);
            let identity = session.dialog_identity().await.unwrap().unwrap();
            assert_eq!(
                Some(identity.call_id),
                peer.calls.lock().unwrap().first().cloned()
            );
            assert_eq!(
                identity.remote_tag.as_deref(),
                Some("canonical-telephone-peer")
            );
            let before = peer.calls.lock().unwrap().len();
            let acks = peer
                .methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| **m == Method::Ack)
                .count();
            session
                .reinvite()
                .as_session_timer_refresh()
                .send()
                .await
                .unwrap();
            until(|| {
                peer.calls.lock().unwrap().len() == before + 1
                    && peer
                        .methods
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|m| **m == Method::Ack)
                        .count()
                        == acks + 1
            })
            .await;
            assert_eq!(peer.unique(), 1, "refresh retained the original call");
            sip_native_duplex(
                &peer,
                &mut peers,
                payload,
                "telephone-after-established-refresh",
            )
            .await;
            let before = peer.calls.lock().unwrap().len();
            let acks = peer
                .methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| **m == Method::Ack)
                .count();
            let offer = peer.dialog.lock().unwrap().as_ref().unwrap().offer.clone();
            peer.reject_refresh
                .store(true, std::sync::atomic::Ordering::Release);
            session.reinvite().with_sdp(offer).send().await.unwrap();
            until(|| {
                peer.calls.lock().unwrap().len() == before + 1
                    && peer
                        .methods
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|m| **m == Method::Ack)
                        .count()
                        == acks + 1
            })
            .await;
            assert_eq!(peer.unique(), 1, "rejected refresh created another call");
            sip_native_duplex(
                &peer,
                &mut peers,
                payload,
                "telephone-after-rejected-refresh-ack",
            )
            .await;
            Some(session)
        } else {
            None
        };
        if matches!(stop, Stop::CommittedRestart | Stop::CommittedDrop) {
            let before = status(&f, &reader.id, &command.operation_id).await;
            drop(peers);
            restart::without_redial(
                f,
                command,
                &reader.token,
                &peer,
                sip_bind,
                before,
                Some(payload),
                matches!(stop, Stop::CommittedDrop),
            )
            .await;
            return;
        }
        assert!(adapter
            .as_ref()
            .is_none_or(|a| a.is_connection_live(&candidate)));
        let store = f.server.store.clone();
        let admin = store.authenticate(&f.admin).unwrap();
        let key = route_key.id.clone();
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(move || store.revoke(&admin, &key)),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        until(|| {
            peer.methods.lock().unwrap().contains(&Method::Bye)
                && adapter
                    .as_ref()
                    .is_none_or(|a| !a.is_connection_live(&candidate))
        })
        .await;
        until(|| {
            db.query_row(
                "SELECT count(*) FROM telephone_connection_lifetimes WHERE retired=1",
                [],
                |r| r.get::<_, u64>(0),
            )
            .unwrap()
                == 1
        })
        .await;
        if let Some(session) = refresh_session {
            let before = peer.calls.lock().unwrap().len();
            let _ = session.reinvite().as_session_timer_refresh().send().await;
            tokio::time::sleep(Duration::from_millis(350)).await;
            assert_eq!(
                peer.calls.lock().unwrap().len(),
                before,
                "revoked route emitted a refresh"
            );
        }
        assert_eq!(
            status(&f, &reader.id, &command.operation_id).await.state,
            OperationState::Completed
        );
        probe(
            &mut peers,
            0,
            &[],
            "telephone-stop-preserves-retained-native",
        )
        .await;
        assert_eq!(peer.unique(), 1);
        drop(peers);
        f.server.shutdown().await.unwrap();
        if let Some(adapter) = &adapter {
            adapter.drain().await.unwrap();
        }
        if let Some(coordinator) = &coordinator {
            coordinator
                .shutdown_gracefully(Some(Duration::from_secs(3)))
                .await
                .unwrap();
        }
        if configured {
            assert_sip_listener_released(sip_bind);
        }
        return;
    }
    let during = status(&f, &controller_key.id, &command.operation_id).await;
    assert_eq!(during.state, OperationState::Preparing);
    assert!(during.result.is_none());
    assert_eq!(during.effects.len(), if answered.is_some() { 7 } else { 3 });
    let allocation = during
        .effects
        .iter()
        .find(|e| e.provider == "native-replacement-allocate")
        .unwrap();
    let submission = during
        .effects
        .iter()
        .find(|e| e.provider == "native-replacement-start")
        .unwrap();
    assert_eq!(allocation.state, EffectState::Confirmed);
    assert_eq!(
        submission.state,
        if answered.is_some() {
            EffectState::Confirmed
        } else {
            EffectState::Submitting
        }
    );
    assert_eq!(submission.correlation_key, peer.calls.lock().unwrap()[0]);
    assert_ne!(route_owner, f.client.identity_id);
    let candidate: String = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(
        "SELECT candidate_runtime_id FROM native_replacements WHERE operation_record_id=?1",
        [&during.record_id],
        |r| r.get(0),
    )
    .unwrap();
    let candidate = rvoip_core::ids::ConnectionId::from_string(candidate);
    match stop {
        Stop::RouteCredential | Stop::AnsweredPcmu | Stop::AnsweredPcma => {
            // A ringing call retains metadata, never a route-owner permission reader.
            let store = f.server.store.clone();
            let admin = store.authenticate(&f.admin).unwrap();
            let key = route_key.id.clone();
            tokio::time::timeout(
                Duration::from_secs(2),
                tokio::task::spawn_blocking(move || store.revoke(&admin, &key)),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        }
        Stop::PublicCancel => {
            controller
                .request_value(
                    "operation.cancel",
                    json!({"operation_id":command.operation_id}),
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        connection_id: command.connection_id.clone(),
                        operation_id: Some("cancel-ringing-telephone".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .ack()
                .unwrap();
        }
        Stop::Endpoint => {
            let store = f.server.store.clone();
            let key = route_key.id.clone();
            tokio::time::timeout(
                Duration::from_secs(2),
                tokio::task::spawn_blocking(move || {
                    store.execute_provisioning(
                        &key,
                        ProvisioningCommand::RevokeEndpoint {
                            operation_id: "revoke-ringing-endpoint".into(),
                            endpoint_id: endpoint,
                            expected_revision: 1,
                        },
                    )
                }),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        }
        Stop::Voicemail
        | Stop::RejectedConfirmation
        | Stop::PrivateCancel
        | Stop::PrivateRestart
        | Stop::CommittedRestart
        | Stop::PrivateDrop
        | Stop::CommittedDrop
        | Stop::StreamControl
        | Stop::Dtmf
        | Stop::DtmfReceive
        | Stop::DtmfRefresh
        | Stop::DtmfPrivacy
        | Stop::DtmfRetained
        | Stop::DtmfPrimary
        | Stop::ReturnNative
        | Stop::ReturnObserver
        | Stop::Refresh => {
            unreachable!("answered refusal cases end before ordinary cancellation")
        }
        Stop::Deadline | Stop::LostReply => {}
        Stop::Shutdown => unreachable!("shutdown case ends before ordinary cancellation"),
    }
    let terminal = if answered.is_some() {
        Method::Bye
    } else {
        Method::Cancel
    };
    until(|| peer.methods.lock().unwrap().contains(&terminal)).await;
    stopped(
        &f,
        &controller_key.id,
        &command.operation_id,
        OperationState::Unknown,
    )
    .await;
    until(|| {
        peer.methods.lock().unwrap().contains(&Method::Ack)
            && adapter
                .as_ref()
                .is_none_or(|a| !a.is_connection_live(&candidate))
    })
    .await;
    controller.transition(command).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(peer.unique(), 1);
    probe(&mut peers, 0, &[1], "telephone-after-stop-original-remote").await;
    probe(&mut peers, 1, &[0], "telephone-after-stop-original-source").await;
    drop(peers);
    f.server.shutdown().await.unwrap();
    if let Some(adapter) = &adapter {
        adapter.drain().await.unwrap();
    }
    if let Some(coordinator) = &coordinator {
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(3)))
            .await
            .unwrap();
    }
    if configured {
        assert_sip_listener_released(sip_bind);
    }
}

fn assert_sip_listener_released(address: std::net::SocketAddr) {
    let _udp = std::net::UdpSocket::bind(address).expect("configured SIP UDP listener released");
    let _tcp = std::net::TcpListener::bind(address).expect("configured SIP TCP listener released");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_established_refresh_uses_current_route_after_original_authority_expires(
) {
    telephone_case(Stop::Refresh, false).await;
}
