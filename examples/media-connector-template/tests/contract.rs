use bytes::Bytes;
use codec_core::{
    codecs::g711::{ulaw_compress, G711Codec, G711Variant},
    types::AudioCodec as _,
};
use connect_connector_api::{dispatch::Authority, media::*, DispatchPermit};
use connect_media_connector_template::{Adapter, Admission, Remote, Status, QUEUE_CAPACITY};
use rvoip_core::{adapter::*, ids::*, stream::*, Direction, Transport};
use rvoip_core_traits::peer_switch::PeerRouteTicket;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Weak,
    },
    time::{Duration, Instant},
};
struct Gate {
    active: AtomicBool,
    until: Instant,
}
impl Authority for Gate {
    fn current(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    fn deadline(&self) -> Instant {
        self.until
    }
}
fn permit(ttl: Duration, readonly: bool) -> (Arc<Gate>, DispatchPermit) {
    let g = Arc::new(Gate {
        active: AtomicBool::new(true),
        until: Instant::now() + ttl,
    });
    let p = if readonly {
        DispatchPermit::from_host_readonly(g.clone())
    } else {
        DispatchPermit::from_host(g.clone())
    };
    (g, p)
}
fn request(adapter: &Adapter, operation: &str, p: DispatchPermit) -> OriginateRequest {
    OriginateRequest::new(
        SessionId::from_string("fixture-session"),
        ParticipantId::from_string("fixture-participant"),
        "fixture:duplex",
        Direction::Outbound,
        adapter.capabilities(),
    )
    .with_transport(Transport::InProcessAi)
    .with_context(Admission {
        operation_id: operation.into(),
        permit: p,
    })
}
fn frame(value: i16) -> MediaFrame {
    MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from(vec![ulaw_compress(value); 160]),
        timestamp_rtp: 0,
        source_clock: None,
        captured_at: chrono::Utc::now(),
        payload_type: Some(0),
    }
}
async fn until(mut test: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(2);
    while !test() {
        assert!(Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn staged_admission_uses_current_effect_custody_and_exact_original_intent() {
    let remote = Arc::new(Remote::default());
    let adapter = Adapter::open(remote).await.unwrap();
    let mut events = adapter.subscribe_orchestrator_events();
    let (_, p) = permit(Duration::from_secs(5), false);
    let req = request(&adapter, "original", p.clone());
    let first = adapter.originate(req.clone()).await.unwrap();
    let same = adapter.originate(req.clone()).await.unwrap();
    assert_eq!(first.connection.id, same.connection.id);
    assert!(events.try_recv().is_err());
    let streams = adapter.streams(first.connection.id.clone()).await.unwrap();
    assert!(!streams[0].source_ready());
    assert!(streams[0].try_frames_in().is_err());
    assert!(streams[0].try_frames_out().is_err());
    assert_eq!(
        adapter.observe("original", &p).unwrap().state,
        Status::Prepared
    );
    assert_eq!(adapter.observe("original", &p).unwrap().effect_attempts, 0);
    let mut changed = req.clone();
    changed.participant_id = ParticipantId::from_string("forged-peer");
    assert!(adapter.originate(changed).await.is_err());
    adapter
        .activate_outbound(first.connection.id.clone())
        .await
        .unwrap();
    adapter
        .activate_outbound(first.connection.id.clone())
        .await
        .unwrap();
    assert_eq!(adapter.observe("original", &p).unwrap().effect_attempts, 1);
    assert!(matches!(
        events.recv().await,
        Some(OrchestratorAdapterEvent::Public(
            AdapterEvent::Connected { .. }
        ))
    ));
    adapter
        .end(first.connection.id.clone(), EndReason::Normal)
        .await
        .unwrap();
    adapter
        .end(first.connection.id, EndReason::Normal)
        .await
        .unwrap();
    let (_, readonly) = permit(Duration::from_secs(5), true);
    let stopped = adapter.observe("original", &readonly).unwrap();
    assert_eq!(stopped.state, Status::Stopped);
    assert_eq!(stopped.effect_attempts, 1);
    assert!(adapter.originate(req).await.is_err());
    let req = request(&adapter, "readonly", readonly.clone());
    let prepared = adapter.originate(req).await.unwrap();
    assert!(adapter
        .activate_outbound(prepared.connection.id)
        .await
        .is_err());
    assert_eq!(
        adapter
            .observe("readonly", &readonly)
            .unwrap()
            .effect_attempts,
        0
    );
    adapter.drain().await.unwrap();
}

#[tokio::test]
async fn actual_duplex_pcmu_decodes_speaker_markers_and_owned_end_stops_both_queues() {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let _events = adapter.subscribe_orchestrator_events();
    let (_, p) = permit(Duration::from_secs(5), false);
    let handle = adapter
        .originate(request(&adapter, "duplex", p.clone()))
        .await
        .unwrap();
    adapter
        .activate_outbound(handle.connection.id.clone())
        .await
        .unwrap();
    let stream = adapter
        .streams(handle.connection.id.clone())
        .await
        .unwrap()
        .remove(0);
    let reservation = stream.reserve_frames_in().unwrap();
    assert!(stream.reserve_frames_in().is_err());
    drop(reservation);
    let mut source = stream.try_frames_in().unwrap();
    assert!(stream.try_frames_in().is_err());
    let sink = stream.try_frames_out().unwrap();
    let mut decoder = G711Codec::new(G711Variant::MuLaw);
    let mut last = None;
    for _ in 0..8 {
        sink.send(frame(-8192)).await.unwrap();
        let packet = tokio::time::timeout(Duration::from_secs(1), source.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(packet.payload_type, Some(0));
        let samples = decoder.decode(&packet.payload).unwrap();
        assert_eq!(samples.len(), 160);
        assert!(samples.iter().all(|v| (i32::from(*v) - 4096).abs() < 256));
        if let Some(before) = last {
            assert_eq!(packet.timestamp_rtp, before + 160);
        }
        last = Some(packet.timestamp_rtp);
    }
    until(|| adapter.metrics(&handle.connection.id).unwrap().decoded == 8).await;
    assert!((adapter.metrics(&handle.connection.id).unwrap().last_sample + 8192).abs() < 256);
    assert!(!stream.has_quality_measurement());
    adapter
        .end(handle.connection.id.clone(), EndReason::Normal)
        .await
        .unwrap();
    let before = adapter.metrics(&handle.connection.id).unwrap();
    assert!(!stream.source_ready());
    assert!(sink.send(frame(8192)).await.is_err());
    assert!(stream.try_frames_out().is_err());
    while tokio::time::timeout(Duration::from_secs(1), source.recv())
        .await
        .unwrap()
        .is_some()
    {}
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(adapter.metrics(&handle.connection.id).unwrap(), before);
    adapter.drain().await.unwrap();
}

#[tokio::test]
async fn stale_buffered_generation_is_discarded_at_the_actual_decode() {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let _events = adapter.subscribe_orchestrator_events();
    let (_, p) = permit(Duration::from_secs(5), false);
    let h = adapter
        .originate(request(&adapter, "generations", p))
        .await
        .unwrap();
    adapter
        .activate_outbound(h.connection.id.clone())
        .await
        .unwrap();
    let stream = adapter
        .streams(h.connection.id.clone())
        .await
        .unwrap()
        .remove(0);
    let mut source = stream.try_frames_in().unwrap();
    let out = stream.try_peer_frames_out().unwrap();
    let old = PeerRouteTicket::initial();
    let next = old.stage().unwrap();
    let stale = rvoip_core::peer_media::PeerMediaFrame::new(frame(8192), old.clone());
    assert!(next.commit_from(&old));
    out.send(stale).await.unwrap();
    out.send(rvoip_core::peer_media::PeerMediaFrame::new(
        frame(-8192),
        next,
    ))
    .await
    .unwrap();
    until(|| adapter.metrics(&h.connection.id).unwrap().decoded == 1).await;
    assert!(adapter.metrics(&h.connection.id).unwrap().last_sample < 0);
    assert!(source.recv().await.is_some());
    adapter.drain().await.unwrap();
    assert_eq!(adapter.metrics(&h.connection.id).unwrap().decoded, 1);
}

#[tokio::test]
async fn revocation_expiry_and_backpressure_fence_media_instead_of_inventing_health() {
    for mode in 0..3 {
        let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
        let mut events = adapter.subscribe_orchestrator_events();
        let (gate, p) = permit(
            if mode == 1 {
                Duration::from_millis(50)
            } else {
                Duration::from_secs(5)
            },
            false,
        );
        let h = adapter
            .originate(request(&adapter, "fences", p.clone()))
            .await
            .unwrap();
        adapter
            .activate_outbound(h.connection.id.clone())
            .await
            .unwrap();
        let stream = adapter
            .streams(h.connection.id.clone())
            .await
            .unwrap()
            .remove(0);
        if mode == 0 {
            gate.active.store(false, Ordering::Release);
        }
        until(|| !stream.source_ready()).await;
        adapter.drain().await.unwrap();
        let metrics = adapter.metrics(&h.connection.id).unwrap();
        assert!(metrics.generated <= QUEUE_CAPACITY);
        assert!(!stream.has_quality_measurement());
        assert!(stream.try_frames_out().is_err());
        let mut terminal = None;
        while let Ok(event) = events.try_recv() {
            if let OrchestratorAdapterEvent::Public(AdapterEvent::Ended { reason, .. }) = event {
                terminal = Some(reason)
            }
        }
        if mode == 2 {
            assert!(matches!(terminal, Some(EndReason::Failed { .. })));
        }
        assert!(adapter
            .originate(request(
                &adapter,
                "fences",
                permit(Duration::from_secs(1), false).1
            ))
            .await
            .is_err());
    }
}
struct Sink {
    count: AtomicUsize,
    retired: AtomicBool,
    adapter: Weak<Adapter>,
}
#[async_trait::async_trait]
impl AdapterLifecycleSink for Sink {
    async fn deliver_terminal(&self, event: AdapterEvent) {
        if let AdapterEvent::Ended { connection_id, .. } = event {
            self.retired.store(
                !self
                    .adapter
                    .upgrade()
                    .unwrap()
                    .is_connection_live(&connection_id),
                Ordering::Release,
            );
            self.count.fetch_add(1, Ordering::AcqRel);
        }
    }
}
#[tokio::test]
async fn closed_terminal_queue_uses_owned_fallback_and_restart_keeps_original_effect() {
    let remote = Arc::new(Remote::default());
    let adapter = Adapter::open(remote.clone()).await.unwrap();
    assert!(Adapter::open(remote.clone()).await.is_err());
    let events = adapter.subscribe_orchestrator_events();
    let sink = Arc::new(Sink {
        count: AtomicUsize::new(0),
        retired: AtomicBool::new(false),
        adapter: Arc::downgrade(&adapter),
    });
    adapter.install_lifecycle_sink(sink.clone()).unwrap();
    assert!(adapter.install_lifecycle_sink(sink.clone()).is_err());
    let (_, p) = permit(Duration::from_secs(5), false);
    let req = request(&adapter, "restart", p.clone());
    let h = adapter.originate(req.clone()).await.unwrap();
    adapter
        .activate_outbound(h.connection.id.clone())
        .await
        .unwrap();
    drop(events);
    adapter
        .end(h.connection.id.clone(), EndReason::Normal)
        .await
        .unwrap();
    assert_eq!(sink.count.load(Ordering::Acquire), 1);
    assert!(sink.retired.load(Ordering::Acquire));
    drop(adapter);
    let next = Adapter::open(remote).await.unwrap();
    assert_eq!(next.observe("restart", &p).unwrap().effect_attempts, 1);
    assert_eq!(
        next.observe("restart", &p).unwrap().connection_id,
        h.connection.id.to_string()
    );
    assert!(next.originate(req).await.is_err());
    next.drain().await.unwrap();
}

#[tokio::test]
async fn capabilities_and_receipt_retention_refuse_unsupported_or_excess_work_before_effect() {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let _events = adapter.subscribe_orchestrator_events();
    let (_, p) = permit(Duration::from_secs(5), false);
    let mut bad = request(&adapter, "bad", p.clone());
    bad.capabilities.audio_codecs[0].name = "opus".into();
    assert!(adapter.originate(bad).await.is_err());
    assert!(adapter
        .send_dtmf(ConnectionId::new(), "123", 100)
        .await
        .is_err());
    for index in 0..8 {
        let (_, p) = permit(Duration::from_secs(5), false);
        adapter
            .originate(request(&adapter, &format!("operation-{index}"), p))
            .await
            .unwrap();
    }
    assert!(adapter
        .originate(request(&adapter, "capacity", p))
        .await
        .is_err());
    adapter.drain().await.unwrap();
    let m = connect_media_connector_template::manifest();
    m.validate().unwrap();
    let flags = adapter.lifecycle_capabilities();
    let runtime = MediaRuntime {
        binding: LiveBinding::Transport {
            transport: serde_json::to_value(adapter.transport())
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
        },
        codecs: LiveCodecs::Advertised {
            audio: adapter
                .capabilities()
                .audio_codecs
                .into_iter()
                .map(|c| connect_connector_api::media::AudioCodec {
                    name: c.name,
                    clock_rate_hz: c.clock_rate_hz,
                    channels: c.channels,
                })
                .collect(),
            video: vec![],
            data: vec![],
        },
        authoritative_liveness: flags.authoritative_liveness,
        atomic_inbound_handoff: flags.atomic_inbound_handoff,
        terminal_fallback: flags.terminal_fallback,
        staged_outbound_activation: flags.staged_outbound_activation,
        device_replacement: adapter.supports_device_replacement_preparation(),
    };
    m.validate_media_runtime(&runtime).unwrap();
    assert!(runtime.atomic_inbound_handoff);
    let MediaSupport::Live { controls, .. } = m.media else {
        panic!("live fixture")
    };
    assert!(!controls.contains(&LiveControl::InboundAdmission));
    assert!(!runtime.device_replacement);
}

struct BlockingSink {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl AdapterLifecycleSink for BlockingSink {
    async fn deliver_terminal(&self, _: AdapterEvent) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}
#[tokio::test]
async fn cancelling_drain_retains_the_original_join_for_cleanup_retry() {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let events = adapter.subscribe_orchestrator_events();
    let sink = Arc::new(BlockingSink {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    adapter.install_lifecycle_sink(sink.clone()).unwrap();
    let (_, p) = permit(Duration::from_secs(5), false);
    let h = adapter
        .originate(request(&adapter, "cancel-drain", p.clone()))
        .await
        .unwrap();
    adapter
        .activate_outbound(h.connection.id.clone())
        .await
        .unwrap();
    drop(events);
    let owner = adapter.clone();
    let id = h.connection.id.clone();
    let cleanup = tokio::spawn(async move { owner.end(id, EndReason::Normal).await });
    tokio::time::timeout(Duration::from_secs(1), sink.entered.notified())
        .await
        .unwrap();
    cleanup.abort();
    assert!(cleanup.await.unwrap_err().is_cancelled());
    sink.release.notify_one();
    adapter.drain().await.unwrap();
    assert_eq!(
        adapter.observe("cancel-drain", &p).unwrap().state,
        Status::Stopped
    );
    assert_eq!(
        adapter.observe("cancel-drain", &p).unwrap().effect_attempts,
        1
    );
}

#[tokio::test]
async fn guarded_core_registers_atomic_outbound_fixture_and_host_cancel_is_terminal() {
    let adapter = Adapter::open(Arc::new(Remote::default())).await.unwrap();
    let core = rvoip_core::Orchestrator::new(Default::default());
    let mut inbound = core
        .install_inbound_admission_gate(1, Duration::from_secs(1))
        .unwrap();
    core.register(adapter.clone()).unwrap();
    assert!(core.register(adapter.clone()).is_err());
    let (_, p) = permit(Duration::from_secs(5), false);
    let mut bad = request(&adapter, "inbound", p.clone());
    bad.direction = Direction::Inbound;
    assert!(adapter.originate(bad).await.is_err());
    assert!(adapter.accept(ConnectionId::new()).await.is_err());
    assert!(inbound.try_recv().is_err());
    let h = adapter
        .originate(request(&adapter, "dormant", p.clone()))
        .await
        .unwrap();
    adapter.cancel();
    assert!(!adapter.is_connection_live(&h.connection.id));
    assert!(adapter.activate_outbound(h.connection.id).await.is_err());
    assert!(adapter
        .originate(request(&adapter, "after-cancel", p.clone()))
        .await
        .is_err());
    assert_eq!(adapter.observe("dormant", &p).unwrap().effect_attempts, 0);
    adapter.drain().await.unwrap();
    core.drain_prepared_outbound_connections().await;
    core.drain_connection_lifecycle_tasks().await;
    assert_eq!(core.connection_lifecycle_task_count(), 0);
}
