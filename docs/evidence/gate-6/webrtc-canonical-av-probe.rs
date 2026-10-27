//! G6-051 actual canonical Core audio/VP8 port, peer and lifecycle probe.
//! Membership, identity, current policy and capture are isolated fixtures.
//! Source pictures are FFmpeg/libvpx-generated 64x64 red/blue keyframes; the
//! received pictures are independently decoded by FFmpeg's native VP8 decoder.
//! Set CONNECT_AV_FIXTURE_DIR to the generated fixture directory and
//! CONNECT_AV_FFMPEG to the decoder binary when compiling this probe.
//! This is not Store/signed-WSS, browser, AV timing or full Gate qualification.
#![allow(dead_code)]
#[path = "../../../.deps/rvoip/crates/foundation/rvoip-core/tests/support/participant_control_fixture.rs"]
mod fixture;
use rtc::{media_stream::MediaStreamTrack, rtp_transceiver::rtp_sender::*};
use rvoip_core::{
    adapter::*, identity::*, ids::*, orchestrator::peer_native::*, participant::*,
    relationships::*, staged_streams::*, stream_lifecycle::*,
};
use rvoip_webrtc::{
    WebRtcConfig,
    canonical::*,
    peer::{PeerRole, RvoipPeerConnection},
};
use std::{
    any::Any,
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{OwnedRwLockReadGuard, RwLock, mpsc},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use webrtc::{
    media_stream::{
        track_local::{TrackLocal, static_rtp::TrackLocalStaticRTP},
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::*,
};
fn actor(n: &str) -> AuthenticatedPrincipal {
    let mut p = fixture::principal(n);
    p.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(1));
    p.assurance = IdentityAssurance::Identified {
        credential_kind: CredentialKind::Oidc,
    };
    p
}
fn endpoint(n: &str) -> EndpointBinding {
    let p = actor(n);
    let b = IdentityBinding::from_authenticated(
        &p,
        IdentityId::from_string(format!("id_{n}")),
        chrono::Utc::now(),
    )
    .unwrap();
    let mut e = EndpointBinding::provision(
        b.clone(),
        EndpointId::from_string(format!("ep_{n}")),
        DeviceId::from_string(format!("device_{n}")),
        rvoip_core::Transport::WebRtc,
        "fixture".into(),
    )
    .unwrap();
    e.verify(&b, "actual-fixture-auth".into(), chrono::Utc::now())
        .unwrap();
    e
}
struct Authority {
    authority: PeerNativeAuthority,
}
impl PeerNativeAuthorityLease for Authority {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.authority
    }
}
fn caps() -> EndpointMediaCapabilities {
    let c = vec![
        MediaCapability {
            medium: Medium::Audio,
            formats: vec![MediaFormat::Encoded {
                name: "pcmu".into(),
                clock_rate_hz: 8000,
                channels: 1,
                fmtp: None,
            }],
        },
        MediaCapability {
            medium: Medium::Video,
            formats: vec![MediaFormat::Encoded {
                name: "vp8".into(),
                clock_rate_hz: 90000,
                channels: 0,
                fmtp: None,
            }],
        },
    ];
    EndpointMediaCapabilities {
        send: c.clone(),
        receive: c,
        max_streams: 2,
        renegotiation: true,
    }
}
#[derive(Default)]
struct State {
    owners: HashMap<ConnectionId, MediaOwner>,
    deny: HashSet<ConnectionId>,
    deny_video: HashSet<ConnectionId>,
}
#[derive(Default)]
struct Domain {
    state: Arc<RwLock<State>>,
    activation: Mutex<HashMap<ConnectionId, Arc<dyn MediaActivation>>>,
}
struct Lease {
    owner: MediaOwner,
    activation: Option<Arc<dyn MediaActivation>>,
    _guard: OwnedRwLockReadGuard<State>,
}
impl MediaAuthorityLease for Lease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(match &self.activation {
            Some(a) if a.confirmed_generation().is_some() => a.current(self.owner.clone())?,
            _ => self.owner.clone(),
        }))
    }
    fn control_prepared(
        &self,
        p: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(
            self.activation
                .as_ref()
                .ok_or(StreamControlError::NotReady)?
                .prepare(p, self.owner.clone())?,
        ))
    }
    fn control_receive(&self, _: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        self.control_owner()
    }
    fn control_participant_receive(
        &self,
        _: &MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        self.control_owner()
    }
}
impl Domain {
    fn lease(&self, s: &MediaScope<'_>) -> ControlResult<Lease> {
        let guard = self
            .state
            .clone()
            .try_read_owned()
            .map_err(|_| StreamControlError::Backpressure)?;
        let mut o = guard
            .owners
            .get(s.connection)
            .ok_or(StreamControlError::Authority)?
            .clone();
        if o.member.conversation() != s.conversation || o.presence.session() != s.session {
            return Err(StreamControlError::Authority);
        }
        o.member
            .revalidate_principal(s.principal, chrono::Utc::now())
            .map_err(|_| StreamControlError::Authority)?;
        if guard.deny.contains(s.connection) {
            // Core receives this endpoint's publication. Outgoing audio to it remains granted.
            o.receive_media.clear();
        }
        if guard.deny_video.contains(s.connection) {
            o.receive_media.remove(&Medium::Video);
        }
        let activation = self.activation.lock().unwrap().get(s.connection).cloned();
        if let Some(a) = activation
            .as_ref()
            .filter(|a| a.confirmed_generation().is_some())
        {
            o = a.current(o)?;
        }
        Ok(Lease {
            owner: o,
            activation,
            _guard: guard,
        })
    }
    async fn insert(&self, f: &fixture::Fixture, n: &str, id: &ConnectionId, ep: &EndpointBinding) {
        let member = f.policy.0.read().unwrap().members[&fixture::participant(n)].clone();
        let presence = SessionAppearance::admit(&member, f.session.clone(), None).unwrap();
        let connection = ConnectionBinding::prepare(&member, &presence, ep, id.clone()).unwrap();
        self.state.write().await.owners.insert(
            id.clone(),
            MediaOwner {
                actor: member.clone(),
                member,
                presence,
                endpoint: ep.clone(),
                connection,
                may_control: true,
                send_media: [Medium::Audio, Medium::Video].into(),
                receive_media: [Medium::Audio, Medium::Video].into(),
                local: caps(),
                remote: caps(),
                may_route_source: true,
            },
        );
    }
}
impl MediaPolicy for Domain {
    fn current(&self, s: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        Ok(Box::new(self.lease(s)?))
    }
    fn current_participant_receive(
        &self,
        s: &MediaScope<'_>,
        _: &MediaParticipantSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.current(s)
    }
    fn current_control(
        &self,
        s: &MediaScope<'_>,
        _: &MediaControlIntent<'_>,
        _: &(dyn Any + Send + Sync),
    ) -> ControlResult<MediaControl> {
        MediaControl::new(
            Box::new(self.lease(s)?),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
    }
}
struct Controls(Arc<fixture::TestPolicy>);
struct HeldControls(Arc<fixture::TestPolicy>);
fn participant_authority(
    policy: &fixture::TestPolicy,
    scope: &rvoip_core::orchestrator::participant_control::ParticipantScope<'_>,
) -> rvoip_core::orchestrator::participant_control::ControlResult<
    rvoip_core::orchestrator::participant_control::ParticipantAuthority,
> {
    use rvoip_core::orchestrator::participant_control::*;
    let lease = policy.current(scope)?;
    let a = lease.authority();
    Ok(ParticipantAuthority {
        actor: a.actor.clone(),
        target: a.target.clone(),
        endpoint: a.endpoint.clone(),
        may_send_audio: a.may_send_audio,
        may_receive_audio: a.may_receive_audio,
        confirmation_required: a.confirmation_required,
        confirmed_generation: a.confirmed_generation,
    })
}
impl rvoip_core::orchestrator::participant_control::ParticipantControlLease for HeldControls {
    fn current(
        &self,
        scope: &rvoip_core::orchestrator::participant_control::ParticipantScope<'_>,
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<
        rvoip_core::orchestrator::participant_control::ParticipantAuthority,
    > {
        participant_authority(&self.0, scope)
    }
}
impl rvoip_core::orchestrator::participant_control::ParticipantPolicy for Controls {
    fn current(
        &self,
        scope: &rvoip_core::orchestrator::participant_control::ParticipantScope<'_>,
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<
        Box<dyn rvoip_core::orchestrator::participant_control::ParticipantAuthorityLease + '_>,
    > {
        self.0.current(scope)
    }
    fn current_control(
        &self,
        _scope: &rvoip_core::orchestrator::participant_control::ParticipantScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<
        rvoip_core::orchestrator::participant_control::ParticipantControl,
    > {
        use rvoip_core::orchestrator::participant_control::*;
        if !held.is::<()>() {
            return Err(ParticipantControlError::Authority);
        }
        ParticipantControl::new(
            Box::new(HeldControls(self.0.clone())),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
    }
}
struct Handler {
    gather: mpsc::Sender<()>,
    connected: mpsc::Sender<()>,
    remote: mpsc::Sender<Arc<dyn TrackRemote>>,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, s: RTCIceGatheringState) {
        if s == RTCIceGatheringState::Complete {
            let _ = self.gather.try_send(());
        }
    }
    async fn on_connection_state_change(&self, s: RTCPeerConnectionState) {
        if s == RTCPeerConnectionState::Connected {
            let _ = self.connected.try_send(());
        }
    }
    async fn on_track(&self, t: Arc<dyn TrackRemote>) {
        let _ = self.remote.try_send(t);
    }
}
async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .expect("bounded actual Core/RTC operation")
}
mod video_watch {
    use rtc::{
        interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket, interceptor},
        sansio,
        shared::error::Error,
    };
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Instant,
    };
    #[derive(Default)]
    pub struct State {
        pub armed: AtomicBool,
        pub dropped: Mutex<Option<rtc::rtp::Packet>>,
        pub entered: tokio::sync::Notify,
        pub repairs: Mutex<std::collections::VecDeque<rtc::rtp::Packet>>,
    }
    #[derive(Interceptor)]
    pub struct Watch<I: Interceptor> {
        #[next]
        pub inner: I,
        pub state: Arc<State>,
    }
    #[interceptor]
    impl<I: Interceptor> Watch<I> {
        #[overrides]
        fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
            if let Packet::Rtp(p) = &msg.message {
                if p.header.payload_type == 121 {
                    let mut repairs = self.state.repairs.lock().unwrap();
                    if repairs.len() == 16 {
                        repairs.pop_front();
                    }
                    repairs.push_back(p.clone());
                }
                if p.header.payload_type == 120 && self.state.armed.swap(false, Ordering::SeqCst) {
                    *self.state.dropped.lock().unwrap() = Some(p.clone());
                    self.state.entered.notify_one();
                    return Ok(());
                }
            }
            self.inner.handle_read(msg)
        }
    }
}
struct Side {
    watch: Arc<video_watch::State>,
    owner: rvoip_core::peer_admission::PeerAdmissionOwner,
    id: ConnectionId,
    physical: Arc<WebRtcPhysicalBinding>,
    slots: Arc<tokio::sync::Semaphore>,
    cached_remote: Option<Arc<dyn TrackRemote>>,
    peer: Arc<RvoipPeerConnection>,
    controller: Arc<StreamController>,
    raw: Arc<dyn PeerConnection>,
    remote: mpsc::Receiver<Arc<dyn TrackRemote>>,
    send: tokio::task::JoinHandle<()>,
    stop: CancellationToken,
}
async fn side(
    f: &fixture::Fixture,
    adapter: &Arc<CanonicalWebRtcAdapter>,
    domain: &Arc<Domain>,
    name: &str,
    ssrc: u32,
    payload: u8,
) -> Side {
    let p = actor(name);
    let owner = rvoip_core::peer_admission::PeerAdmissionOwner::new(
        &p,
        IdentityId::from_string(format!("id_{name}")),
        DeviceId::from_string(format!("device_{name}")),
        CancellationToken::new(),
        Default::default(),
    )
    .unwrap();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let physical = WebRtcPhysicalBinding::with_capacity(
        &owner.peer(),
        CancellationToken::new(),
        slots.clone().try_acquire_owned().unwrap(),
    );
    let id = ConnectionId::new();
    let ep = endpoint(name);
    let a = Authority {
        authority: PeerNativeAuthority {
            actor: f.policy.0.read().unwrap().members[&fixture::participant(name)].clone(),
            endpoint: ep.clone(),
            may_prepare: true,
        },
    };
    f.orch
        .prepare_peer_webrtc_connection(
            owner.peer(),
            &p,
            PeerWebRtcIntent {
                session: f.session.clone(),
                connection: id.clone(),
            },
            &a,
            physical.clone(),
        )
        .unwrap()
        .retain(&p, &a)
        .unwrap();
    f.policy
        .0
        .write()
        .unwrap()
        .endpoints
        .insert(id.clone(), ep.clone());
    domain.insert(f, name, &id, &ep).await;
    let (controller, activation) = adapter
        .configure_media(&id, p, f.conversation.clone(), domain.clone(), 4, &())
        .unwrap();
    domain
        .activation
        .lock()
        .unwrap()
        .insert(id.clone(), activation);
    let premature = controller
        .change_with_held(
            0,
            &[DesiredStream {
                logical_id: StreamId::from_string(format!("premature_{name}")),
                medium: Medium::Audio,
                direction: StreamDirection::Both,
            }],
            &BTreeSet::new(),
            Instant::now() + Duration::from_millis(100),
            CancellationToken::new(),
            Arc::new(()),
        )
        .await;
    assert!(
        premature.is_err(),
        "allocation reservation alone must not establish media readiness"
    );
    assert!(controller.active_bindings().is_empty());
    assert_eq!(controller.revision(), 0);
    assert_eq!(slots.available_permits(), 0);
    assert_eq!(owner.peer().snapshot().connections, 1);
    let mut config = WebRtcConfig::loopback();
    config.capabilities.audio_codecs = vec![rvoip_core::capability::CodecInfo {
        name: "g.711-mu".into(),
        clock_rate_hz: 8000,
        channels: 1,
        fmtp: None,
        payload_type: None,
    }];
    config.require_vp8_audio_video_profile = true;
    physical.start_peer(config, PeerRole::Answerer).unwrap();
    let peer = physical
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let codec = RTCRtpCodec {
        mime_type: "audio/PCMU".into(),
        clock_rate: 8000,
        channels: 1,
        ..Default::default()
    };
    let mut media = rtc::peer_connection::configuration::media_engine::MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: codec.clone(),
                payload_type: 109,
            },
            RtpCodecKind::Audio,
        )
        .unwrap();
    let video_codec = RTCRtpCodec {
        mime_type: "video/VP8".into(),
        clock_rate: 90000,
        ..Default::default()
    };
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: video_codec.clone(),
                payload_type: 120,
            },
            RtpCodecKind::Video,
        )
        .unwrap();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: "video/rtx".into(),
                    clock_rate: 90000,
                    sdp_fmtp_line: "apt=120".into(),
                    ..Default::default()
                },
                payload_type: 121,
            },
            RtpCodecKind::Video,
        )
        .unwrap();
    media
        .register_header_extension(
            RTCRtpHeaderExtensionCapability {
                uri: "urn:ietf:params:rtp-hdrext:sdes:mid".into(),
            },
            RtpCodecKind::Video,
            None,
        )
        .unwrap();
    let watch = Arc::new(video_watch::State::default());
    let registry=rtc::peer_connection::configuration::interceptor_registry::register_owned_dtmf_interceptors(&mut media,rtc::interceptor::SenderClockHandle::default()).unwrap().with(|inner|video_watch::Watch{inner,state:watch.clone()});
    let (g, mut gr) = mpsc::channel(1);
    let (c, mut cr) = mpsc::channel(1);
    let (r, remote) = mpsc::channel(4);
    let raw: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(Handler {
                gather: g,
                connected: c,
                remote: r,
            }))
            .with_udp_addrs(vec!["127.0.0.1:0"])
            .build()
            .await
            .unwrap(),
    );
    let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        name.into(),
        format!("raw_{name}"),
        "fixture".into(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec,
            ..Default::default()
        }],
    )));
    raw.add_track(track.clone()).await.unwrap();
    let video_track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        name.into(),
        format!("raw_video_{name}"),
        "fixture-video".into(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc + 10000),
                ..Default::default()
            },
            codec: video_codec,
            ..Default::default()
        }],
    )));
    raw.add_track(video_track.clone()).await.unwrap();
    let color = if name == "alice" { "red" } else { "blue" };
    let frame = std::fs::read(
        std::path::Path::new(env!("CONNECT_AV_FIXTURE_DIR")).join(format!("{color}.vp8")),
    )
    .unwrap();

    raw.set_local_description(raw.create_offer(None).await.unwrap())
        .await
        .unwrap();
    bounded(gr.recv()).await.unwrap();
    let offer = raw.local_description().await.unwrap();
    let answer = peer.accept_offer_and_gather(&offer.sdp).await.unwrap();
    raw.set_remote_description(RTCSessionDescription::answer(answer).unwrap())
        .await
        .unwrap();
    bounded(cr.recv()).await.unwrap();
    peer.wait_connected(Duration::from_secs(3)).await.unwrap();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let send = tokio::spawn(async move {
        let mut seq = 1u16;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tokio::select! {_=cancel.cancelled()=>break,_=tick.tick()=>{}}
            let p = rtc::rtp::Packet {
                header: rtc::rtp::Header {
                    version: 2,
                    payload_type: 109,
                    ssrc,
                    sequence_number: seq,
                    timestamp: u32::from(seq) * 160,
                    ..Default::default()
                },
                payload: vec![payload; 160].into(),
            };
            if track.write_rtp(p).await.is_err() {
                break;
            }
            if seq % 3 == 1 {
                let mut body = vec![0x10];
                body.extend_from_slice(&frame);
                if video_track
                    .write_rtp(rtc::rtp::Packet {
                        header: rtc::rtp::Header {
                            version: 2,
                            payload_type: 120,
                            ssrc: ssrc + 10000,
                            sequence_number: seq / 3 + 1,
                            timestamp: u32::from(seq) * 1800,
                            marker: true,
                            ..Default::default()
                        },
                        payload: body.into(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            seq = seq.wrapping_add(1);
        }
    });
    controller
        .change_with_held(
            0,
            &[
                DesiredStream {
                    logical_id: StreamId::from_string(format!("audio_{name}")),
                    medium: Medium::Audio,
                    direction: StreamDirection::Both,
                },
                DesiredStream {
                    logical_id: StreamId::from_string(format!("video_{name}")),
                    medium: Medium::Video,
                    direction: StreamDirection::Both,
                },
            ],
            &BTreeSet::new(),
            Instant::now() + Duration::from_secs(4),
            CancellationToken::new(),
            Arc::new(()),
        )
        .await
        .unwrap();
    assert_eq!(slots.available_permits(), 0);
    assert_eq!(owner.peer().snapshot().connections, 1);
    assert_eq!(controller.active_bindings().len(), 2);
    Side {
        watch,
        owner,
        id,
        physical,
        slots,
        cached_remote: None,
        peer,
        controller,
        raw,
        remote,
        send,
        stop,
    }
}
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_webrtc=warn")
        .with_ansi(false)
        .init();
    webrtc::runtime::block_on(async {
        let mut f = fixture::Fixture::new_without_adapter().await;
        let unused_session = f.session.clone();
        let adapter = CanonicalWebRtcAdapter::new();
        f.orch.register(adapter.clone()).unwrap();
        let domain = Arc::new(Domain::default());
        f.session = f
            .orch
            .start_session(
                f.conversation.clone(),
                rvoip_core::session::SessionMedium::VoiceVideo,
                vec![],
            )
            .await
            .unwrap();
        f.orch
            .enable_staged_participant_control(
                &f.session,
                Arc::new(Controls(f.policy.clone())),
                domain.clone(),
                Default::default(),
            )
            .unwrap();
        f.orch
            .end_session(unused_session, EndReason::Normal)
            .await
            .unwrap();
        let mut a = side(&f, &adapter, &domain, "alice", 1111, 0x80).await;
        let mut b = side(&f, &adapter, &domain, "bob", 2222, 0x00).await;
        println!(
            "PASS unallocated resources refuse readiness then recover into four actual Core AV ports without synthetic callbacks"
        );
        for (name, s) in [("alice", &a), ("bob", &b)] {
            let mutation = f.intent(
                &f.session,
                name,
                &format!("admit_{name}"),
                rvoip_core::participant_transitions::ParticipantChange::Admit {
                    connection: s.id.clone(),
                    role_override: None,
                },
            );
            let admitted = f
                .orch
                .admit_peer_session_connection_with_held(
                    s.owner.peer(),
                    &f.session,
                    actor(name),
                    mutation.clone(),
                    Instant::now() + Duration::from_secs(4),
                    CancellationToken::new(),
                    &(),
                )
                .await
                .unwrap();
            let replay = f
                .orch
                .admit_peer_session_connection_with_held(
                    s.owner.peer(),
                    &f.session,
                    actor(name),
                    mutation,
                    Instant::now() + Duration::from_secs(4),
                    CancellationToken::new(),
                    &(),
                )
                .await
                .unwrap();
            assert_eq!(replay.revision, admitted.revision);
            assert_eq!(replay.operation_id, admitted.operation_id);
            assert_eq!(s.owner.peer().snapshot().connections, 1);
        }
        println!(
            "PASS actual staged RTC Controllers admit and replay original participant operations with one peer charge"
        );
        for (side, source) in [(&a, "bob"), (&b, "alice")] {
            use rvoip_core::staged_streams::audio_subscriptions::*;
            f.orch
                .change_staged_audio_subscriptions(
                    &f.session,
                    &side.id,
                    side.controller.revision(),
                    AudioSubscriptionChange::Subscribe(vec![AudioSubscription::Participant(
                        fixture::participant(source),
                    )]),
                    Instant::now() + Duration::from_secs(4),
                    CancellationToken::new(),
                    Some(&()),
                )
                .await
                .unwrap();
        }
        println!(
            "PASS explicit current Core participant subscriptions authorize both receive directions"
        );
        async fn track(side: &mut Side) -> Arc<dyn TrackRemote> {
            if side.cached_remote.is_none() {
                side.cached_remote = Some(bounded(side.remote.recv()).await.unwrap());
            }
            side.cached_remote.as_ref().unwrap().clone()
        }
        async fn received(side: &mut Side, expected: i16) {
            use rvoip_media_core::codec::factory::CodecFactory;
            let remote = track(side).await;
            let mut decoder = CodecFactory::create_codec(0, Some(8000), Some(1)).unwrap();
            bounded(async {
                loop {
                    if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = remote.poll().await {
                        let decoded = decoder.decode(&p.payload).unwrap();
                        if decoded.samples.len() == 160
                            && decoded.samples.iter().all(|v| {
                                v.signum() == expected.signum() && v.unsigned_abs() > 20_000
                            })
                        {
                            assert_eq!(p.header.ssrc, side.peer.local_audio_ssrc().unwrap());
                            assert_eq!(p.header.payload_type, 109);
                            break;
                        }
                    }
                }
            })
            .await;
        }
        async fn silent(side: &mut Side) {
            use rvoip_media_core::codec::factory::CodecFactory;
            let remote = track(side).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            while tokio::time::timeout(Duration::from_millis(1), remote.poll())
                .await
                .is_ok()
            {}
            let mut decoder = CodecFactory::create_codec(0, Some(8000), Some(1)).unwrap();
            let until = Instant::now() + Duration::from_millis(400);
            loop {
                let event = tokio::select! {_=tokio::time::sleep_until(until)=>break,event=remote.poll()=>event};
                if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = event {
                    let decoded = decoder.decode(&p.payload).unwrap();
                    assert_eq!(decoded.samples.len(), 160);
                    assert!(
                        decoded.samples.iter().all(|v| v.unsigned_abs() < 500),
                        "withdrawn source remained audible"
                    );
                } else if event.is_none() {
                    panic!("original RTC track ended during permission withdrawal");
                }
            }
        }

        received(&mut a, -1).await;
        received(&mut b, 1).await;
        println!(
            "PASS independently decoded duplex audio continues on the actual canonical AV peers"
        );
        async fn video_track(s: &Side) -> Option<Arc<dyn TrackRemote>> {
            for r in s.raw.get_receivers().await {
                let t = r.track();
                if t.kind().await == RtpCodecKind::Video {
                    return Some(t.clone());
                }
            }
            None
        }
        assert!(video_track(&a).await.is_none());
        assert!(video_track(&b).await.is_none());
        println!("PASS no implicit video playback before explicit Core selection");
        async fn select(
            f: &fixture::Fixture,
            target: &Side,
            source: &Side,
            source_name: &str,
            target_name: &str,
        ) {
            use rvoip_core::staged_streams::direct_subscriptions::*;
            f.orch
                .change_staged_direct_subscriptions(
                    &f.session,
                    &target.id,
                    target.controller.revision(),
                    DirectSubscriptionChange::Subscribe(vec![DirectSubscription::new(
                        &source.controller,
                        StreamId::from_string(format!("video_{source_name}")),
                        StreamId::from_string(format!("video_{target_name}")),
                    )]),
                    Instant::now() + Duration::from_secs(3),
                    CancellationToken::new(),
                    Some(&()),
                )
                .await
                .unwrap();
        }
        select(&f, &a, &b, "bob", "alice").await;
        select(&f, &b, &a, "alice", "bob").await;
        async fn picture(s: &Side) -> rtc::rtp::Packet {
            let t = bounded(async {
                loop {
                    if let Some(t) = video_track(s).await {
                        break t;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await
                }
            })
            .await;
            bounded(async {
                loop {
                    if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = t.poll().await {
                        if p.header.marker && p.payload.len() > 10 {
                            assert_eq!(p.header.payload_type, 120);
                            assert_eq!(p.header.ssrc, s.peer.local_video_ssrc().unwrap());
                            assert_eq!(p.payload[0], 0x10);
                            break p;
                        }
                    }
                }
            })
            .await
        }
        let pa = picture(&a).await;
        let pb = picture(&b).await;
        async fn mid(s: &Side, p: &rtc::rtp::Packet) {
            use rvoip_sip_core::types::sdp::{ParsedAttribute, SdpSession};
            let local = s.raw.local_description().await.unwrap();
            let remote = s.raw.remote_description().await.unwrap();
            let binding = rvoip_webrtc::media::core_video_profile::negotiated_profile(
                &local.sdp,
                &remote.sdp,
            )
            .unwrap();
            let parsed: SdpSession = remote.sdp.parse().unwrap();
            let v = parsed
                .media_descriptions
                .iter()
                .find(|m| m.media == "video" && m.port != 0)
                .unwrap();
            let id = v
                .generic_attributes
                .iter()
                .find_map(|a| {
                    if let ParsedAttribute::ExtMap(id, _, uri, _) = a {
                        (uri == "urn:ietf:params:rtp-hdrext:sdes:mid").then_some(*id)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(
                p.header.get_extension(id as u8).unwrap().as_ref(),
                binding.mid.as_ref().unwrap().as_bytes()
            );
        }
        mid(&a, &pa).await;
        mid(&b, &pb).await;
        println!(
            "PASS actual negotiated VP8 PT/SSRC/MID and original pictures traverse both canonical Core AV routes"
        );
        // Drop one real received/decrypted primary before application dispatch.
        // The sender already completed actual encrypted UDP submission. Repair
        // must restore this genuinely missing frame, not an observed duplicate.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        async fn repair(s: &Side) {
            use rtc::rtcp::transport_feedbacks::transport_layer_nack::{
                NackPair, TransportLayerNack,
            };
            s.watch
                .armed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            bounded(s.watch.entered.notified()).await;
            let original = s.watch.dropped.lock().unwrap().take().unwrap();
            let t = video_track(s).await.unwrap();
            t.write_rtcp(vec![Box::new(TransportLayerNack {
                sender_ssrc: 8888,
                media_ssrc: original.header.ssrc,
                nacks: vec![NackPair {
                    packet_id: original.header.sequence_number,
                    lost_packets: 0,
                }],
            })])
            .await
            .unwrap();
            bounded(async {
                loop {
                    if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = t.poll().await {
                        if p.header.sequence_number == original.header.sequence_number {
                            assert_eq!(p.payload, original.payload);
                            assert_eq!(p.header.timestamp, original.header.timestamp);
                            assert_eq!(p.header.ssrc, original.header.ssrc);
                            mid(s, &p).await;
                            break;
                        }
                    }
                }
            })
            .await;
            let repairs = s.watch.repairs.lock().unwrap();
            assert!(repairs.iter().any(|p| p.header.payload_type == 121
                && p.header.ssrc != original.header.ssrc
                && p.payload[..2] == original.header.sequence_number.to_be_bytes()
                && p.payload[2..] == original.payload[..]));
        }
        repair(&a).await;
        repair(&b).await;
        println!(
            "PASS actual canonical NACK/RTX repair preserves original picture identity and negotiated MID"
        );
        fn decode(label: &str, p: &rtc::rtp::Packet, red: bool) {
            let dir = std::path::Path::new(env!("CONNECT_AV_FIXTURE_DIR"));
            let ivf = dir.join(format!("received-{label}.ivf"));
            let rgb = dir.join(format!("received-{label}.rgb"));
            let frame = &p.payload[1..];
            let mut header = b"DKIF".to_vec();
            header.extend_from_slice(&0u16.to_le_bytes());
            header.extend_from_slice(&32u16.to_le_bytes());
            header.extend_from_slice(b"VP80");
            header.extend_from_slice(&64u16.to_le_bytes());
            header.extend_from_slice(&64u16.to_le_bytes());
            header.extend_from_slice(&90000u32.to_le_bytes());
            header.extend_from_slice(&1u32.to_le_bytes());
            header.extend_from_slice(&1u32.to_le_bytes());
            header.extend_from_slice(&0u32.to_le_bytes());
            header.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            header.extend_from_slice(&u64::from(p.header.timestamp).to_le_bytes());
            header.extend_from_slice(frame);
            std::fs::write(&ivf, header).unwrap();
            let out = std::process::Command::new(env!("CONNECT_AV_FFMPEG"))
                .args([
                    "-nostdin",
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "ivf",
                    "-c:v",
                    "vp8",
                    "-i",
                ])
                .arg(ivf)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24"])
                .arg(&rgb)
                .output()
                .unwrap();
            std::fs::write(dir.join(format!("decode-{label}.log")), &out.stderr).unwrap();
            assert!(out.status.success());
            let pixels = std::fs::read(rgb).unwrap();
            assert_eq!(pixels.len(), 64 * 64 * 3);
            let sums = (0..3)
                .map(|channel| {
                    pixels
                        .chunks_exact(3)
                        .map(|p| u64::from(p[channel]))
                        .sum::<u64>()
                        / 4096
                })
                .collect::<Vec<_>>();
            if red {
                assert!(sums[0] > 200 && sums[1] < 60 && sums[2] < 60, "{sums:?}")
            } else {
                assert!(sums[2] > 200 && sums[0] < 60 && sums[1] < 60, "{sums:?}")
            }
        }
        decode("alice", &pa, false);
        decode("bob", &pb, true);
        println!(
            "PASS independent FFmpeg decoding identifies the other participant's distinct 64x64 picture in each direction"
        );
        async fn no_video(s: &Side) {
            let t = video_track(s).await.unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
            while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(5), t.poll()).await {
            }
            let none = tokio::time::timeout(Duration::from_millis(100), async {
                loop {
                    if let Some(TrackRemoteEvent::OnRtpPacket(_, _)) = t.poll().await {
                        break;
                    }
                }
            })
            .await;
            assert!(none.is_err());
        }
        use rvoip_core::staged_streams::direct_subscriptions::*;
        f.orch
            .change_staged_direct_subscriptions(
                &f.session,
                &a.id,
                a.controller.revision(),
                DirectSubscriptionChange::Clear,
                Instant::now() + Duration::from_secs(3),
                CancellationToken::new(),
                Some(&()),
            )
            .await
            .unwrap();
        no_video(&a).await;
        received(&mut a, -1).await;
        picture(&b).await;
        select(&f, &a, &b, "bob", "alice").await;
        decode("alice-reselected", &picture(&a).await, false);
        println!(
            "PASS video selection withdrawal and restoration leave duplex audio and reverse video running"
        );
        domain.state.write().await.deny_video.insert(a.id.clone());
        no_video(&b).await;
        received(&mut b, 1).await;
        picture(&a).await;
        domain.state.write().await.deny_video.remove(&a.id);
        decode("bob-restored", &picture(&b).await, true);
        println!(
            "PASS current video-only publication permission withdraws/restores independently of audio and reverse video"
        );
        a.stop.cancel();
        b.stop.cancel();
        a.send.await.unwrap();
        b.send.await.unwrap();
        f.orch
            .end_session(f.session.clone(), EndReason::Normal)
            .await
            .unwrap();
        adapter.end(a.id.clone(), EndReason::Normal).await.unwrap();
        adapter.end(b.id.clone(), EndReason::Normal).await.unwrap();
        a.raw.close().await.unwrap();
        b.raw.close().await.unwrap();
        assert_eq!(adapter.route_count(), 0);
        assert_eq!(a.slots.available_permits(), 1);
        assert_eq!(b.slots.available_permits(), 1);
        fixture::until(|| {
            a.owner.peer().snapshot().connections == 0 && b.owner.peer().snapshot().connections == 0
        })
        .await;
        println!(
            "PASS both original canonical AV peers and all four actual Core ports join before quota refund"
        );
    });
}
