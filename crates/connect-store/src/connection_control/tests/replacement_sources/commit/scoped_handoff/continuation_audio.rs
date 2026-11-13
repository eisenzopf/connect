//! Actual Store continuation publication -> Core original PCM collect.
//! Store movement/readiness observations remain controlled. Core relationship
//! setup/PCMU negotiation is controlled; actual native handoff/transport is NOT
//! claimed. Every audio policy grant owns a real current Store snapshot lease.
use super::*;
use rvoip_core::{
    identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance},
    ids::{
        ConnectionId, ConversationId, DeviceId, EndpointId, IdentityId, ParticipantId, SessionId,
        StreamId,
    },
    participant::{ParticipantKind, ParticipantRole},
    relationships::{
        ConnectionBinding, EndpointBinding, IdentityBinding, Membership, SessionAppearance,
    },
    staged_streams::{test_support::ControlledAudioScene, *},
    stream::{MediaFrame, StreamKind},
    stream_lifecycle::*,
    Transport,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    panic::AssertUnwindSafe,
};
use tokio_util::sync::CancellationToken;

async fn caught<F: Future>(future: F) -> std::thread::Result<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
            Err(error) => std::task::Poll::Ready(Err(error)),
        }
    })
    .await
}
struct Cleanup;
#[async_trait::async_trait]
impl MediaPortCleanup for Cleanup {
    async fn close(&self) -> ControlResult<()> {
        Ok(())
    }
}
#[derive(Default)]
struct Adapter(Mutex<Option<AdapterMediaPort>>);
#[async_trait::async_trait]
impl StagedMediaAdapter for Adapter {
    async fn prepare(
        &self,
        ports: Vec<PortPreparation>,
        _: tokio::time::Instant,
        _: CancellationToken,
    ) -> ControlResult<()> {
        for p in ports {
            if p.is_new {
                p.port.install_cleanup(Arc::new(Cleanup))?;
                p.port
                    .observe_ready(&p.port.binding().negotiated.format, true, true, Some(0))?;
            }
            *self.0.lock().unwrap() = Some(p.port);
        }
        Ok(())
    }
}
fn principal(identity: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: identity.into(),
        tenant: Some("continuation-conformance".into()),
        issuer: Some("continuation-conformance".into()),
        scopes: vec![],
        expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        method: AuthenticationMethod::Bearer,
        assurance: IdentityAssurance::Anonymous,
    }
}
// Controlled Core relationship objects are derived from exact current Store
// identity/session/participant/generation facts, not used as Store permission.
fn owner(view: &crate::PeerConnectionProjection) -> MediaOwner {
    let c = &view.connection;
    assert_eq!(
        c.membership_epoch, 1,
        "component supports this actual fixture epoch only"
    );
    assert!((1..=2).contains(&c.generation));
    let identity = &view.session.actor.identity_id;
    let now = chrono::Utc::now();
    let binding = IdentityBinding::from_authenticated(
        &principal(identity),
        IdentityId::from_string(identity),
        now,
    )
    .unwrap();
    let member = Membership::admit(
        binding.clone(),
        ConversationId::from_string(&c.conversation_id),
        ParticipantId::from_string(&c.participant_id),
        ParticipantKind::Human,
        ParticipantRole::Agent,
    )
    .unwrap();
    let presence =
        SessionAppearance::admit(&member, SessionId::from_string(&c.session_id), None).unwrap();
    let mut endpoint = EndpointBinding::provision(
        binding.clone(),
        EndpointId::from_string(&c.endpoint_id),
        DeviceId::from_string(&view.device_id),
        Transport::Quic,
        view.endpoint_address.clone(),
    )
    .unwrap();
    endpoint
        .verify(&binding, "controlled-component-endpoint".into(), now)
        .unwrap();
    let runtime = ConnectionId::from_string(&view.runtime_connection_id);
    let mut connection = ConnectionBinding::prepare(
        &member,
        &presence,
        &endpoint,
        if c.generation == 1 {
            runtime.clone()
        } else {
            ConnectionId::from_string("component-original-before-replacement")
        },
    )
    .unwrap();
    connection
        .observe_ready(&member, &presence, &endpoint, 1)
        .unwrap();
    connection.activate(&member, &presence, &endpoint).unwrap();
    if c.generation == 2 {
        let mut candidate = connection
            .replacement(&member, &presence, &endpoint, runtime)
            .unwrap();
        candidate
            .observe_ready(&member, &presence, &endpoint, 2)
            .unwrap();
        connection
            .commit_replacement(&mut candidate, &member, &presence, &endpoint)
            .unwrap();
        connection = candidate;
    }
    let capability = MediaCapability {
        medium: Medium::Audio,
        formats: vec![MediaFormat::Encoded {
            name: "pcmu".into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }],
    };
    let profile = EndpointMediaCapabilities {
        send: vec![capability.clone()],
        receive: vec![capability],
        max_streams: 1,
        renegotiation: true,
    };
    MediaOwner {
        actor: member.clone(),
        member,
        presence,
        endpoint,
        connection,
        may_control: true,
        send_media: [Medium::Audio].into_iter().collect(),
        receive_media: [Medium::Audio].into_iter().collect(),
        local: profile.clone(),
        remote: profile,
        may_route_source: true,
    }
}
struct Epoch(crate::NativeMediaSnapshotEpoch);
impl PacketEpoch for Epoch {
    fn current(&self) -> bool {
        self.0.current()
    }
}
struct Lease {
    owner: MediaOwner,
    read: crate::NativeMediaRead,
}
impl MediaAuthorityLease for Lease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn packet_epoch(&self) -> Option<Arc<dyn PacketEpoch>> {
        self.read
            .packet_epoch()
            .map(|e| Arc::new(Epoch(e)) as Arc<dyn PacketEpoch>)
    }
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self.read.expires_at(), 0)
    }
}
struct Policy {
    candidate: ConnectionId,
    receiver: ConnectionId,
    owners: BTreeMap<ConnectionId, MediaOwner>,
    candidate_snapshot: crate::NativeMediaSnapshot,
    receiver_snapshot: crate::NativeMediaSnapshot,
    pair_snapshot: crate::NativeMediaSnapshot,
}
impl Policy {
    fn grant(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let owner = self
            .owners
            .get(scope.connection)
            .ok_or(StreamControlError::Authority)?;
        if scope.principal.subject != owner.member.identity().as_str()
            || scope.conversation != owner.member.conversation()
            || scope.session != owner.presence.session()
        {
            return Err(StreamControlError::Authority);
        }
        let snapshot = match scope.purpose {
            MediaPurpose::Publish | MediaPurpose::Control
                if *scope.connection == self.candidate =>
            {
                &self.candidate_snapshot
            }
            MediaPurpose::Publish | MediaPurpose::Control if *scope.connection == self.receiver => {
                &self.receiver_snapshot
            }
            MediaPurpose::Receive { source }
                if *scope.connection == self.receiver && source.connection == self.candidate =>
            {
                let original = &self.owners[&self.candidate];
                if source.connection_generation != original.connection.generation()
                    || source.participant != *original.member.participant()
                    || source.membership_epoch != original.member.epoch()
                    || source.conversation != *scope.conversation
                    || source.session != *scope.session
                {
                    return Err(StreamControlError::Authority);
                }
                &self.pair_snapshot
            }
            _ => return Err(StreamControlError::Authority),
        };
        let read = snapshot.admit().map_err(|e| {
            if e.code == "authority-busy" {
                StreamControlError::NotReady
            } else {
                StreamControlError::Authority
            }
        })?;
        if !read.current() || read.target().runtime_connection_id != scope.connection.as_str() {
            return Err(StreamControlError::Authority);
        }
        if let MediaPurpose::Receive { source } = scope.purpose {
            if !read.permits_subscriptions()
                || read.source().is_none_or(|v| {
                    v.runtime_connection_id != source.connection.as_str()
                        || v.connection.generation != source.connection_generation
                })
            {
                return Err(StreamControlError::Authority);
            }
        }
        Ok(Box::new(Lease {
            owner: owner.clone(),
            read,
        }))
    }
}
impl MediaPolicy for Policy {
    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.grant(scope)
    }
    fn current_queued(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.grant(scope)
    }
}
fn continuation_snapshot(
    store: &Arc<Store>,
    target: &Arc<NativeMediaOrigin>,
    target_activation: Option<&Arc<crate::NativeReplacementActivation>>,
    source: Option<&Arc<NativeMediaOrigin>>,
    source_activation: Option<&Arc<crate::NativeReplacementActivation>>,
) -> crate::NativeMediaSnapshot {
    let weak = Arc::downgrade(store);
    let target = target.clone();
    let source = source.cloned();
    let target_activation = target_activation.cloned();
    let source_activation = source_activation.cloned();
    let registration_target = target.clone();
    let registration_source = source.clone();
    store
        .register_native_media_snapshot_for(
            &registration_target,
            registration_source.as_deref(),
            move || {
                weak.upgrade()
                    .ok_or_else(Error::forbidden)?
                    .try_native_replacement_continuation_media_read(
                        &target,
                        target_activation.as_deref(),
                        source
                            .as_ref()
                            .map(|s| (s.as_ref(), source_activation.as_deref())),
                        None,
                    )
            },
        )
        .unwrap()
}
fn publish(adapter: &Adapter, snapshot: &crate::NativeMediaSnapshot, timestamp: u32) {
    let read = admit(snapshot);
    let epoch = read
        .packet_epoch()
        .expect("clock source must be a real published interval");
    let source_clock = rvoip_core::stream::MediaSourceClock {
        source_id: 1,
        generation: read.target().connection.generation,
        anchor: None,
        supported: true,
        rtp_offset: 0,
        is_current: Arc::new(move || epoch.current()),
    };
    drop(read);
    let port = adapter.0.lock().unwrap().as_ref().unwrap().clone();
    port.try_publish(&MediaFrame {
        stream_id: port.binding().transport_id.clone(),
        kind: StreamKind::Audio,
        payload: vec![0xd5; 160].into(),
        timestamp_rtp: timestamp,
        source_clock: Some(source_clock),
        captured_at: chrono::Utc::now(),
        payload_type: Some(0),
    })
    .unwrap();
}
#[derive(Clone, Copy, Debug)]
enum Fault {
    FinalReady,
    Revocation,
    ChangedBinding,
    CandidateWriter,
}
struct Outcome {
    baseline_timestamp: u32,
    fault: Fault,
    epoch_current: bool,
    first_error: Option<String>,
    failed_wait_delta: u64,
    old_source_fenced: bool,
    healthy_current: bool,
    unchanged: bool,
    collect_authority: u64,
    collect_ok: u64,
    output_timestamp: Option<u32>,
    output_bytes: usize,
    original_age_ms: u128,
    recovered: bool,
}

async fn exercise(fault: Fault) -> Outcome {
    let (setup, call, old_source) = prepare(Kind::PublicReady);
    let Call::Public(capture) = call else {
        unreachable!()
    };
    let healthy_source = setup.healthy_source.clone();
    let healthy_target = setup.healthy_target.clone();
    let m = setup.movement;
    let receiver_origin = m.sources[1].clone();
    let activation = capture.activation().clone();
    let candidate_origin = activation.origin().clone();
    let revoke_credential = m.target_request.credentials[0].clone();
    let store = Arc::new(m.f.store);
    let revoke_actor = store.revalidate(&m.f.admin).unwrap();
    // Block the genuinely changed OLD source's Writer, not the unchanged
    // candidate. This grants no media and causes no external SQLite write.
    let blocker = store
        .acquire_authority(
            &[crate::AuthorityScope::connection(
                old_source.connection_id(),
                crate::AuthorityMode::Read,
            )
            .unwrap()],
            &crate::AuthorityWait::new(Duration::from_millis(5)),
        )
        .unwrap();
    let old_snapshot = register_pair(&store, &old_source, None);
    let healthy_snapshot = register_pair(&store, &healthy_target, Some(&healthy_source));
    let candidate_snapshot =
        continuation_snapshot(&store, &candidate_origin, Some(&activation), None, None);
    let receiver_snapshot = register_pair(&store, &receiver_origin, None);
    let pair_snapshot = continuation_snapshot(
        &store,
        &receiver_origin,
        None,
        Some(&candidate_origin),
        Some(&activation),
    );
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let mut scene = None;
    let mut controllers: Vec<Arc<StreamController>> = Vec::new();
    let mut outputs = Vec::new();
    let mut diagnostic = None;
    // Catch includes all Core setup. Every controller is registered immediately
    // on creation, before its first awaited change/attach can fail.
    let work = caught(async {
        let healthy_read = admit(&healthy_snapshot);
        let healthy_epoch = healthy_read.packet_epoch().unwrap();
        let healthy_stamp = healthy_read.snapshot_stamp().unwrap();
        assert!(healthy_read.current() && healthy_epoch.current() && healthy_read.permits_subscriptions());
        assert_eq!(healthy_read.target().connection.connection_id, healthy_target.connection_id());
        assert_eq!(healthy_read.source().unwrap().connection.connection_id, healthy_source.connection_id());
        for view in [healthy_read.target(), healthy_read.source().unwrap()] {
            assert!(view.session.session.media.iter().any(|m| m == "audio") && view.session.session.media.iter().any(|m| m == "video"));
        }
        let mut healthy_read = Some(healthy_read);
        let old_read = admit(&old_snapshot);
        let old_epoch = old_read.packet_epoch().unwrap();
        let old_stamp = old_read.snapshot_stamp().unwrap();
        assert!(old_read.current() && old_epoch.current());
        drop(old_read);
        let unary = admit(&candidate_snapshot);
        assert!(unary.current() && unary.snapshot_stamp().is_some());
        let candidate_read = crate::AuthorityScope::connection(candidate_origin.connection_id(), crate::AuthorityMode::Read).unwrap();
        let old_read_scope = crate::AuthorityScope::connection(old_source.connection_id(), crate::AuthorityMode::Read).unwrap();
        assert!(unary.interval().covers(std::slice::from_ref(&candidate_read)));
        assert!(!unary.interval().covers(std::slice::from_ref(&old_read_scope)),
            "candidate publication itself depends on the blocked old source; isolation counter would be invalid");
        let epoch = unary
            .packet_epoch()
            .expect("raw factory instead of real publication");
        assert!(epoch.current());
        let candidate_view = unary.target().clone();
        assert_eq!(
            candidate_view.connection.connection_id,
            candidate_origin.connection_id()
        );
        assert_eq!(
            candidate_view.runtime_connection_id,
            candidate_origin.runtime_connection_id()
        );
        assert_eq!(
            candidate_view.connection.generation,
            candidate_origin.generation()
        );
        drop(unary);
        let receiver_read = admit(&receiver_snapshot);
        let receiver_view = receiver_read.target().clone();
        drop(receiver_read);
        let pair = admit(&pair_snapshot);
        assert!(pair.current() && pair.permits_subscriptions());
        assert!(pair.interval().covers(std::slice::from_ref(&candidate_read)));
        assert!(!pair.interval().covers(std::slice::from_ref(&old_read_scope)),
            "receiver pair itself depends on blocked old source; isolation counter would be invalid");
        assert_eq!(
            pair.source().unwrap().runtime_connection_id,
            candidate_origin.runtime_connection_id()
        );
        drop(pair);
        let cid = ConversationId::from_string(candidate_origin.conversation_id());
        let sid = SessionId::from_string(candidate_origin.session_id());
        let ids = [
            ConnectionId::from_string(candidate_origin.runtime_connection_id()),
            ConnectionId::from_string(receiver_origin.runtime_connection_id()),
        ];
        let owners = [
            (ids[0].clone(), owner(&candidate_view)),
            (ids[1].clone(), owner(&receiver_view)),
        ]
        .into_iter()
        .collect();
        let policy = Arc::new(Policy {
            candidate: ids[0].clone(),
            receiver: ids[1].clone(),
            owners,
            candidate_snapshot: candidate_snapshot.clone(),
            receiver_snapshot: receiver_snapshot.clone(),
            pair_snapshot: pair_snapshot.clone(),
        });
        scene = Some(ControlledAudioScene::new(
            cid.clone(),
            sid.clone(),
            policy.clone(),
        ));
        let handles = Arc::new(StreamHandleAllocator::default());
        let mut adapters = Vec::new();
        for (i, view) in [&candidate_view, &receiver_view].iter().enumerate() {
            let adapter = Arc::new(Adapter::default());
            let controller = StreamController::new(
                cid.clone(),
                sid.clone(),
                ids[i].clone(),
                principal(&view.session.actor.identity_id),
                policy.clone(),
                adapter.clone(),
                handles.clone(),
                2,
            )
            .unwrap();
            controllers.push(controller.clone());
            controller
                .change(
                    controller.revision(),
                    &[DesiredStream {
                        logical_id: StreamId::from_string("audio"),
                        medium: Medium::Audio,
                        direction: StreamDirection::Both,
                    }],
                    &BTreeSet::new(),
                    tokio::time::Instant::now() + Duration::from_secs(2),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            scene
                .as_ref()
                .unwrap()
                .attach(
                    controller.clone(),
                    tokio::time::Instant::now() + Duration::from_secs(2),
                )
                .await
                .unwrap();
            outputs.push(
                adapter
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .take_egress()
                    .unwrap(),
            );
            adapters.push(adapter);
        }
        scene
            .as_ref()
            .unwrap()
            .change_subscriptions(
                &ids[1],
                controllers[1].revision(),
                audio_subscriptions::AudioSubscriptionChange::Subscribe(vec![
                    audio_subscriptions::AudioSubscription::Connection(ids[0].clone()),
                ]),
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await
            .unwrap();
        diagnostic = Some(audio_diagnostic::arm(&controllers).unwrap());
        publish(&adapters[0], &candidate_snapshot, 0);
        scene.as_ref().unwrap().tick();
        let mut baseline = None;
        outputs[1]
            .try_recv()
            .expect("positive actual PCM before fault missing")
            .deliver(|frame| {
                baseline = Some((frame.timestamp_rtp, frame.payload.len()));
                async { Ok(()) }
            })
            .await
            .unwrap();
        let (baseline_timestamp, baseline_bytes) = baseline.unwrap();
        assert_eq!(baseline_bytes, 160);
        let before = Store::inspect_snapshot(store.path()).unwrap();
        let admission = capture.projection_diagnostics();
        let initial_wait = admission["activation"]["failed_authority_wait"]
            .as_u64()
            .unwrap();
        let before_counts = diagnostic.as_ref().unwrap().snapshot();
        let before_collect = &before_counts
            .entries
            .iter()
            .find(|e| e.connection == ids[0].as_str())
            .unwrap()
            .stages
            .iter()
            .find(|s| s.0 == "collect-admission")
            .unwrap()
            .1;
        // One original accepted PCMU frame. No IO reader survives this enqueue.
        let queued_at = Instant::now();
        publish(&adapters[0], &candidate_snapshot, 160);

        let mut blocker = Some(blocker);
        let (first_error, failed_wait_delta, unchanged) = match fault {
            Fault::FinalReady => {
                // Metadata-only earlier refusals may retry, but the FIRST
                // actual failed file admission ends this call sequence. Every
                // attempt uses the SAME capture and original PCM's 200ms age.
                let error = loop {
                    let result = store
                        .project_native_replacement_committed(&capture, &MetadataOnlyInstalled);
                    let now = capture.projection_diagnostics();
                    if now["activation"]["failed_authority_wait"].as_u64().unwrap() > initial_wait {
                        break result
                            .err()
                            .expect("old source Reader failed to exclude its actual Writer");
                    }
                    let e = result
                        .err()
                        .expect("publication bypassed genuine old-source exclusion");
                    assert_eq!(e.code, "authority-busy");
                    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
                    assert!(healthy_epoch.current() && healthy_read.as_ref().is_some_and(|read| read.current()),
                        "earlier refusal cut disjoint healthy AV; candidate-only causal qualification invalid");
                    assert!(
                        epoch.current(),
                        "earlier non-Writer metadata refusal invalidated candidate publication"
                    );
                    assert!(
                        queued_at.elapsed() < Duration::from_millis(150),
                        "counter did not reach actual Writer within original PCM age"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                };
                let delta = capture.projection_diagnostics()["activation"]["failed_authority_wait"]
                    .as_u64()
                    .unwrap()
                    - initial_wait;
                (
                    Some(error.code),
                    delta,
                    Store::inspect_snapshot(store.path()).unwrap() == before,
                )
            }
            Fault::Revocation => {
                // Genuine authority-changing DML must not be blocked by the
                // unrelated live IO intentionally retained for the main case.
                drop(healthy_read.take());
                drop(blocker.take());
                // Supported actual credential revocation performs genuine
                // authority-changing DML beneath the normal broad Writer.
                store.revoke(&revoke_actor, &revoke_credential).unwrap();
                assert!(store.revalidate(&revoke_credential).is_err());
                (
                    None,
                    0,
                    Store::inspect_snapshot(store.path()).unwrap() == before,
                )
            }
            Fault::CandidateWriter => {
                drop(blocker.take());
                // A real authority-changing intent for the current candidate
                // must still cut original epochs. This is the same actual
                // Manager boundary used by withdrawal/generation writers, not
                // a fabricated packet token or direct epoch mutation.
                let admission = match store.acquire_authority(
                    &[crate::AuthorityScope::connection(
                        candidate_origin.connection_id(),
                        crate::AuthorityMode::Write,
                    ).unwrap()],
                    &crate::AuthorityWait::new(Duration::from_millis(5)),
                ) {
                    Ok(writer) => { drop(writer); "ok".to_owned() }
                    Err(error) if error.code == "authority-busy" => error.code,
                    Err(error) => panic!("candidate Writer failed outside expected admission: {error:?}"),
                };
                // Real Writer intent fences before waiting for kernel readers.
                // A metadata-only refusal cannot pass the original epochfalse
                // and actual collectAuthority oracle after all owners join.
                (Some(admission), 0, Store::inspect_snapshot(store.path()).unwrap() == before)
            }
            Fault::ChangedBinding => {
                drop(blocker.take());
                controllers[0]
                    .set_media_gate(
                        controllers[0].revision(),
                        &[StreamId::from_string("audio")].into_iter().collect(),
                        StreamGateAction::Mute,
                        tokio::time::Instant::now() + Duration::from_secs(2),
                    )
                    .await
                    .unwrap();
                (
                    None,
                    0,
                    Store::inspect_snapshot(store.path()).unwrap() == before,
                )
            }
        };
        // Never restore/replace an original
        // epoch and never replay the already admitted input under a newer grant.

        let healthy_current = healthy_read.as_ref().is_some_and(|read| read.current())
            && healthy_epoch.current() && healthy_snapshot.admit_original(healthy_stamp).is_ok();
        let epoch_current = epoch.current();
        let old_source_fenced =
            !old_epoch.current() && old_snapshot.admit_original(old_stamp).is_err();
        drop(blocker.take());
        scene.as_ref().unwrap().tick();
        let after_counts = diagnostic.as_ref().unwrap().snapshot();
        let after_collect = &after_counts
            .entries
            .iter()
            .find(|e| e.connection == ids[0].as_str())
            .unwrap()
            .stages
            .iter()
            .find(|s| s.0 == "collect-admission")
            .unwrap()
            .1;
        let collect_authority = after_collect[3] - before_collect[3];
        let collect_ok = after_collect[0] - before_collect[0];
        let mut delivered = None;
        if let Ok(queued) = outputs[1].try_recv() {
            queued
                .deliver(|frame| {
                    delivered = Some((frame.timestamp_rtp, frame.payload.len()));
                    async { Ok(()) }
                })
                .await
                .unwrap();
        }
        let original_age_ms = queued_at.elapsed().as_millis();
        drop(healthy_read.take());
        // Recovery is a new actual input, not replay/reclock of the lost one.
        // It has a separate bounded 2s watcher-recovery observation; the old
        // quantum's original 200ms age was measured above. Only fresh snapshot
        // eligibility may retry here, never the refused original frame.
        let recovered = if matches!(fault, Fault::FinalReady) {
            let until = Instant::now() + Duration::from_secs(2);
            while (candidate_snapshot.admit().is_err() || pair_snapshot.admit().is_err())
                && Instant::now() < until
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            let current = admit(&candidate_snapshot);
            assert!(current.current());
            drop(current);
            publish(&adapters[0], &candidate_snapshot, 320);
            scene.as_ref().unwrap().tick();
            let mut value = None;
            if let Ok(queued) = outputs[1].try_recv() {
                queued
                    .deliver(|frame| {
                        value = Some(frame.timestamp_rtp);
                        async { Ok(()) }
                    })
                    .await
                    .unwrap();
            }
            value == Some(baseline_timestamp.wrapping_add(320))
        } else {
            true
        };
        Outcome {
            baseline_timestamp,
            fault,
            epoch_current,
            first_error,
            failed_wait_delta,
            old_source_fenced,
            healthy_current,
            unchanged,
            collect_authority,
            collect_ok,
            output_timestamp: delivered.map(|v| v.0),
            output_bytes: delivered.map_or(0, |v| v.1),
            original_age_ms,
            recovered,
        }
    })
    .await;
    // Tear down every original owner before any intended failing oracle.
    drop(diagnostic.take());
    drop(scene.take());
    outputs.clear();
    let mut cleanup = Vec::new();
    for controller in &controllers {
        cleanup.push(
            caught(controller.shutdown(tokio::time::Instant::now() + Duration::from_secs(2))).await,
        );
    }
    controllers.clear();
    let watcher_result = watcher.shutdown();
    let clean = cleanup.iter().all(|result| matches!(result, Ok(0)));
    match work {
        Err(error) => {
            eprintln!(
                "continuation counter cleanup core={clean} watcher={}",
                watcher_result.is_ok()
            );
            std::panic::resume_unwind(error)
        }
        Ok(value) => {
            assert!(clean, "original Core shutdown failed/panicked");
            watcher_result.unwrap();
            value
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn failed_final_ready_keeps_original_candidate_pcm_under_registered_continuation() {
    let o = exercise(Fault::FinalReady).await;
    eprintln!("continuation_pcm fault={:?} error={:?} failed_wait={} old_fenced={} healthy_current={} unchanged={} epoch_current={} collect_authority={} collect_ok={} output_ts={:?} output_bytes={} original_age_ms={} fresh_recovered={}",o.fault,o.first_error,o.failed_wait_delta,o.old_source_fenced,o.healthy_current,o.unchanged,o.epoch_current,o.collect_authority,o.collect_ok,o.output_timestamp,o.output_bytes,o.original_age_ms,o.recovered);
    assert_eq!(o.first_error.as_deref(), Some("authority-busy"));
    assert_eq!(o.failed_wait_delta, 1);
    assert!(
        o.old_source_fenced,
        "genuine old-source boundary was not fenced"
    );
    assert!(o.unchanged && o.original_age_ms < 200 && o.recovered);
    assert!(
        o.healthy_current,
        "CID-wide/broad fallback cut a disjoint healthy AV pair; candidate-only cause unqualified"
    );
    // Old qualifying failure must reach actual collect's Authority branch and
    // lose exactly this genuine PCM quantum, not time out/fail setup or IO.
    if !o.epoch_current {
        assert_eq!(o.collect_authority, 1);
        assert_eq!(o.collect_ok, 0);
        assert_eq!(o.output_timestamp, None);
    }
    assert!(
        o.epoch_current
            && o.collect_ok == 1
            && o.output_timestamp == Some(o.baseline_timestamp.wrapping_add(160))
            && o.output_bytes == 160,
        "definite no-effect finalReady refusal discarded the original candidate PCM quantum"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn genuine_revocation_and_changed_binding_still_refuse_original_candidate_pcm() {
    for fault in [Fault::Revocation, Fault::ChangedBinding] {
        let o = exercise(fault).await;
        eprintln!("continuation_pcm_negative fault={fault:?} epoch_current={} collect_authority={} collect_ok={} output={:?} unchanged={} age_ms={}",o.epoch_current,o.collect_authority,o.collect_ok,o.output_timestamp,o.unchanged,o.original_age_ms);
        assert_eq!(o.unchanged, !matches!(fault, Fault::Revocation));
        assert!(o.original_age_ms < 200);
        assert!(
            o.output_timestamp.is_none(),
            "revocation or changed binding revived an original quantum"
        );
        if matches!(fault, Fault::Revocation) {
            assert!(!o.epoch_current && o.collect_authority == 1);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn genuine_candidate_writer_still_fences_original_continuation_pcm() {
    let o = exercise(Fault::CandidateWriter).await;
    eprintln!("continuation_pcm_candidate_writer admission={:?} unchanged={} epoch_current={} healthy_current={} collect_authority={} collect_ok={} output={:?} age_ms={}",
        o.first_error,o.unchanged,o.epoch_current,o.healthy_current,o.collect_authority,o.collect_ok,o.output_timestamp,o.original_age_ms);
    // exercise has already joined every original Core owner and watcher.
    assert!(matches!(
        o.first_error.as_deref(),
        Some("ok" | "authority-busy")
    ));
    assert!(o.unchanged && o.healthy_current && o.original_age_ms < 200);
    assert!(!o.epoch_current && o.collect_authority == 1 && o.collect_ok == 0);
    assert!(o.output_timestamp.is_none());
}
