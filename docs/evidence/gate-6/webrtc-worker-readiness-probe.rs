//! Actual Core RTC worker failure invalidates current readiness and retains
//! original cleanup custody. Real loopback RTP; isolated policy injects one
//! ingress panic. No Store, browser, provider or packaging qualification.
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
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
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
    let c = vec![MediaCapability {
        medium: Medium::Audio,
        formats: vec![MediaFormat::Encoded {
            name: "pcmu".into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }],
    }];
    EndpointMediaCapabilities {
        send: c.clone(),
        receive: c,
        max_streams: 1,
        renegotiation: true,
    }
}
#[derive(Default)]
struct State {
    owners: HashMap<ConnectionId, MediaOwner>,
    deny: HashSet<ConnectionId>,
}
#[derive(Default)]
struct Domain {
    state: Arc<RwLock<State>>,
    activation: Mutex<HashMap<ConnectionId, Arc<dyn MediaActivation>>>,
    // A static domain tag carries NO grant. Retained packet reads still take
    // independent fresh state; the tag lets this fixture fault fresh ingress
    // without accidentally faulting the separate conference mixer task.
    tag: Arc<()>,
    packet_reads: AtomicUsize,
    panic_target: Mutex<Option<ConnectionId>>,
    panic_once: AtomicBool,
    panics: AtomicUsize,
}
struct Lease {
    owner: MediaOwner,
    activation: Option<Arc<dyn MediaActivation>>,
    _guard: OwnedRwLockReadGuard<State>,
    context: Weak<dyn Any + Send + Sync>,
}
impl MediaAuthorityLease for Lease {
    fn packet_context(&self) -> Option<&(dyn Any + Send + Sync)> {
        Some(&self.context)
    }
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
        let activation = self.activation.lock().unwrap().get(s.connection).cloned();
        if let Some(a) = activation
            .as_ref()
            .filter(|a| a.confirmed_generation().is_some())
        {
            o = a.current(o)?;
        }
        let tag: Arc<dyn Any + Send + Sync> = self.tag.clone();
        Ok(Lease {
            context: Arc::downgrade(&tag),
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
                send_media: [Medium::Audio].into(),
                receive_media: [Medium::Audio].into(),
                local: caps(),
                remote: caps(),
                may_route_source: true,
            },
        );
    }
}
impl MediaPolicy for Domain {
    fn current(&self, s: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let target = self.panic_target.lock().unwrap().as_ref() == Some(s.connection);
        if matches!(&s.purpose, MediaPurpose::Publish)
            && target
            && self.panic_once.swap(false, Ordering::AcqRel)
        {
            self.panics.fetch_add(1, Ordering::AcqRel);
            panic!("controlled actual RTC ingress policy failure");
        }
        Ok(Box::new(self.lease(s)?))
    }
    fn current_packet(
        &self,
        s: &MediaScope<'_>,
        context: &(dyn Any + Send + Sync),
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let weak = context
            .downcast_ref::<Weak<dyn Any + Send + Sync>>()
            .ok_or(StreamControlError::Authority)?;
        let expected: Arc<dyn Any + Send + Sync> = self.tag.clone();
        if !Weak::ptr_eq(weak, &Arc::downgrade(&expected)) {
            return Err(StreamControlError::Authority);
        }
        self.packet_reads.fetch_add(1, Ordering::AcqRel);
        // Do not borrow authority from the opaque tag. This is a complete fresh
        // current read for this exact source/target, including every denial.
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
struct Side {
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
    let registry=rtc::peer_connection::configuration::interceptor_registry::register_owned_dtmf_interceptors(&mut media,rtc::interceptor::SenderClockHandle::default()).unwrap();
    let (g, mut gr) = mpsc::channel(1);
    let (c, mut cr) = mpsc::channel(1);
    let (r, remote) = mpsc::channel(1);
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
            seq = seq.wrapping_add(1);
        }
    });
    controller
        .change_with_held(
            0,
            &[DesiredStream {
                logical_id: StreamId::from_string(format!("audio_{name}")),
                medium: Medium::Audio,
                direction: StreamDirection::Both,
            }],
            &BTreeSet::new(),
            Instant::now() + Duration::from_secs(4),
            CancellationToken::new(),
            Arc::new(()),
        )
        .await
        .unwrap();
    assert_eq!(slots.available_permits(), 0);
    assert_eq!(owner.peer().snapshot().connections, 1);
    assert_eq!(controller.active_bindings().len(), 1);
    Side {
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
                rvoip_core::session::SessionMedium::Voice,
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
            "PASS unallocated resources refuse readiness then recover into two actual Core RTC ports without synthetic callbacks"
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
            "PASS Core participant bridge delivers independently decoded marked duplex audio through owned RTC egress pumps"
        );
        domain.state.write().await.deny.insert(a.id.clone());
        silent(&mut b).await;
        received(&mut a, -1).await;
        domain.state.write().await.deny.remove(&a.id);
        received(&mut b, 1).await;
        println!(
            "PASS fresh publication authority withdraws and restores actual decoded audio while the reverse direction continues"
        );
        for (op, role) in [
            ("observe_alice", ParticipantRole::Observer),
            ("speak_alice", ParticipantRole::Agent),
        ] {
            let mutation = f.intent(
                &f.session,
                "alice",
                op,
                rvoip_core::participant_transitions::ParticipantChange::SetSessionRole {
                    role: role.clone(),
                },
            );
            f.orch
                .change_session_participant_with_held(
                    &f.session,
                    actor("owner"),
                    mutation,
                    Instant::now() + Duration::from_secs(4),
                    CancellationToken::new(),
                    &(),
                )
                .await
                .unwrap();
            if role == ParticipantRole::Observer {
                silent(&mut b).await;
                received(&mut a, -1).await;
            } else {
                received(&mut b, 1).await;
            }
        }
        println!(
            "PASS actual Core observer and speaking role transitions gate decoded publication without reallocating the RTC peers"
        );
        assert!(domain.packet_reads.load(Ordering::Acquire) > 0);
        let owner_a = domain.state.read().await.owners[&a.id].clone();
        let owner_b = domain.state.read().await.owners[&b.id].clone();
        let activation_a = domain.activation.lock().unwrap()[&a.id].clone();
        let activation_b = domain.activation.lock().unwrap()[&b.id].clone();
        assert!(activation_a.current(owner_a.clone()).is_ok());
        *domain.panic_target.lock().unwrap() = Some(a.id.clone());
        domain.panic_once.store(true, Ordering::Release);
        bounded(async {
            while domain.panics.load(Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await;
        let invalidated = tokio::time::timeout(Duration::from_secs(1), async {
            while activation_a.current(owner_a.clone()).is_ok() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .is_ok();
        assert!(
            a.peer.is_connected() && b.peer.is_connected(),
            "worker failure must not be mistaken for parent RTC destruction"
        );
        assert!(
            activation_b.current(owner_b).is_ok(),
            "unrelated peer resources were disturbed"
        );
        assert_eq!(a.owner.peer().snapshot().connections, 1);
        assert_eq!(
            a.slots.available_permits(),
            0,
            "failed worker refunded parent capacity before joined cleanup"
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
        println!("PASS original RTC drivers and all Core port resources close before quota refund");
        assert_eq!(domain.panics.load(Ordering::Acquire), 1);
        assert!(
            invalidated,
            "cached ready observation survived actual ingress worker panic"
        );
        println!(
            "PASS actual ingress worker failure invalidates current media readiness while retaining peer quota and capacity until original joined cleanup"
        );
    });
}
