//! Actual Store Registry/authority custody only. Controlled native ready/flow
//! observations create real lifecycle facts; no packets, decoder, PCM, Core
//! movement or remote-media success is claimed by these Store controls.
use super::positive_capture::admit;
use super::*;
use connect_protocol::transitions::TransitionIntent;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant},
};

struct Flow(crate::NativeConnectionFlowFact);
impl crate::NativeConnectionFlowObservation for Flow {
    fn observe(&self) -> crate::Result<crate::NativeConnectionFlowFact> {
        Ok(self.0.clone())
    }
}
fn flow(f: &Fixture, origin: &Arc<NativeMediaOrigin>, video: bool) {
    let read = f.store.try_native_media_read(origin, None).unwrap();
    let fact = Flow(crate::NativeConnectionFlowFact {
        runtime_connection_id: origin.runtime_connection_id().into(),
        generation: origin.generation(),
        stream_revision: 1,
        credential_revision: 1,
        observed_at_millis: chrono::Utc::now().timestamp_millis(),
        streams: if video {
            vec!["audio".into(), "video".into()]
        } else {
            vec!["audio".into()]
        },
        negotiated_capabilities: if video {
            json!({"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"clock_rate":90000}}]})
        } else {
            json!({"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}]})
        },
    });
    f.store
        .record_native_connection_flow(origin, &read, &fact)
        .unwrap();
}
// Actual provisioning uses a distinct endpoint address for the return device.
// The generic fixture helper assigns 4556 to every non-default label.
fn return_endpoint(f: &Fixture) -> String {
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "provision-isolation-return-target".into(),
                identity_id: identity.clone(),
                device: DeviceSpec {
                    id: "device-isolation-return-target".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: "quic://localhost:4557".into(),
                capabilities: vec!["audio".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let writer = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    f.store.connection_with_authority(&writer).unwrap().execute(
        "UPDATE endpoints SET verification_state='verified',verification_evidence='controlled-fixture',verified_at=?2 WHERE id=?1",
        params![endpoint_id, crate::now()],
    ).unwrap();
    endpoint_id
}
fn ready_with_lease(
    f: &Fixture,
    actor: usize,
    request: crate::PeerAuthorityRequest,
    video: bool,
    with_flow: bool,
) -> (Arc<NativeMediaOrigin>, crate::NativeOfferLease) {
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[actor], &proof, Some(EndpointTransport::Quic))
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    drop(proof);
    let writer = f.controlled_conversation_writer(actor);
    let mut c = f.store.connection_with_authority(&writer).unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    let streams = if video {
        json!(["audio", "video"])
    } else {
        json!(["audio"])
    };
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":streams,"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    drop(c);
    drop(writer);
    if with_flow {
        flow(f, &origin, video);
    }
    (origin, lease)
}
struct Setup {
    movement: Movement,
    original_lease: Option<crate::NativeOfferLease>,
    other_primary_leases: Vec<crate::NativeOfferLease>,
    healthy_source: Arc<NativeMediaOrigin>,
    healthy_target: Arc<NativeMediaOrigin>,
}
fn setup(with_flow: bool) -> Setup {
    setup_options(with_flow, None, false)
}
// Only negative controls opt into a short ORIGINAL operator TTL or omit the
// source0 flow observation. Positive OLD4 fixtures keep their original facts.
fn setup_options(with_flow: bool, root_lifetime: Option<i64>, omit_source0_flow: bool) -> Setup {
    setup_options_with_source_ttl(with_flow, root_lifetime, omit_source0_flow, None)
}
fn setup_options_with_source_ttl(
    with_flow: bool,
    root_lifetime: Option<i64>,
    omit_source0_flow: bool,
    source0_lifetime: Option<i64>,
) -> Setup {
    let mut f = Fixture::new();
    for actor in 0..4 {
        let identity = f.store.revalidate(&f.keys[actor]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[actor] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let mut leases = Vec::new();
    let mut sources = Vec::new();
    for actor in 0..4 {
        let lifetime = if actor == 0 {
            source0_lifetime.unwrap_or(600)
        } else {
            600
        };
        let request = f.native_request_for(actor, lifetime, &format!("isolation-primary-{actor}"));
        let (origin, lease) = ready_with_lease(
            &f,
            actor,
            request,
            false,
            with_flow && !(omit_source0_flow && actor == 0),
        );
        sources.push(origin);
        leases.push(lease);
    }
    // Same four identities across4+2 real Connections/two Sessions. The AV pair
    // belongs to actors1/2; their existing native-key1/2 are reused exactly.
    let sid = "isolation-av";
    f.store
        .execute(
            &f.keys[1],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "isolation-av-invite".into(),
                    conversation_id: f.cid.clone(),
                    session_id: sid.into(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::Invite {
                        participants: vec![f.pids[2].clone()],
                        medium: "voice+video".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[2],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "isolation-av-accept".into(),
                    conversation_id: f.cid.clone(),
                    session_id: sid.into(),
                    by_participant: None,
                    expected_revision: Some(0),
                    action: Action::Accept {
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    let mut healthy = Vec::new();
    for actor in [1, 2] {
        // Do NOT call native_request_for twice: it creates a new signing key
        // under the original operationID and would fail before our boundary.
        let mut intent = f.request(actor, &format!("isolation-av-{actor}")).intent;
        intent.session_id = sid.into();
        intent.offer["streams_offered"].as_array_mut().unwrap().push(
            json!({"id":"video","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}));
        let request = crate::PeerAuthorityRequest {
            credentials: vec![f.keys[actor].clone()],
            device: crate::PeerDeviceClaim {
                id: format!("device-{actor}"),
                kind: "desktop".into(),
                platform: "isolated-store-fixture".into(),
            },
            key_id: format!("native-key-{actor}"),
            algorithm: "EdDSA".into(),
            command: crate::PeerCommand::NativeConnectionOffer(intent),
        };
        healthy.push(ready_with_lease(&f, actor, request, true, with_flow).0);
    }
    let target = super::super::endpoint(&f, vec!["audio".into()], "isolation-target");
    let key = if let Some(seconds) = root_lifetime {
        assert!(seconds > 0);
        let owner = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.extend([
            "connection.replace".into(),
            "operation.cancel".into(),
            "operation.execute".into(),
        ]);
        f.store
            .issue(&owner, scope, crate::now() + seconds)
            .unwrap()
            .id
    } else {
        super::super::key(&f)
    };
    let topology = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "isolation-move-0".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: topology,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target),
            candidate_connid: None,
            retained_connid: sources[1].connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let prepared = super::super::allocation::prepared_target_for_scene_named(
        &["stream.subscribe"],
        (f, key, command, origin, sources),
        "isolation-first",
    );
    let movement = fixture_prepared(true, prepared);
    Setup {
        movement,
        original_lease: Some(leases.remove(0)),
        other_primary_leases: leases,
        healthy_source: healthy.remove(0),
        healthy_target: healthy.remove(0),
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    PhysicalCommit,
    PublicReady,
    OriginalCleanup,
    AdoptedCleanup,
}
enum Call {
    Physical {
        origin: Arc<NativeReplacementMediaOrigin>,
        sources: Vec<Arc<NativeMediaOrigin>>,
    },
    Public(crate::NativeReplacementCommitted),
    Original(crate::NativeOfferLease),
    Adopted(Arc<NativeMediaOrigin>),
}
fn prepare(kind: Kind) -> (Setup, Call, Arc<NativeMediaOrigin>) {
    let mut setup = setup(true);
    match kind {
        Kind::PhysicalCommit => {
            let old = setup.movement.sources[0].clone();
            let call = Call::Physical {
                origin: setup.movement.origin.clone(),
                sources: setup.movement.sources.clone(),
            };
            (setup, call, old)
        }
        Kind::PublicReady => {
            let capture = activation_barrier::captured(&setup.movement);
            let old = setup.movement.sources[0].clone();
            (setup, Call::Public(capture), old)
        }
        Kind::OriginalCleanup => {
            // Known physical observation and a still-bounded current prepared
            // handoff, BEFORE public publication; target row remains absent.
            let _capture = activation_barrier::captured(&setup.movement);
            let old = setup.movement.sources[0].clone();
            let call = Call::Original(setup.original_lease.take().unwrap());
            (setup, call, old)
        }
        Kind::AdoptedCleanup => {
            let first = activation_barrier::captured(&setup.movement);
            let replaced = setup
                .movement
                .f
                .store
                .project_native_replacement_committed(&first, &MetadataOnlyInstalled)
                .unwrap();
            let old = replaced.clone();
            let first_m = setup.movement;
            let sources = std::iter::once(replaced)
                .chain(first_m.sources[1..].iter().cloned())
                .collect::<Vec<_>>();
            let f = first_m.f;
            let endpoint = return_endpoint(&f);
            let key = super::super::key(&f);
            let topology = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT topology_revision FROM sessions WHERE id='session'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let command = TransitionCommand {
                operation_id: "isolation-return-0".into(),
                conversation_id: f.cid.clone(),
                session_id: "session".into(),
                connection_id: Some(old.connection_id().into()),
                expected_revision: topology,
                intent: TransitionIntent::Replace {
                    endpoint_id: Some(endpoint),
                    candidate_connid: None,
                    retained_connid: sources[1].connection_id().into(),
                    video: None,
                },
                payload_extensions: Default::default(),
            };
            f.store
                .admit_transition_command(&key, command.clone())
                .unwrap();
            let origin = f
                .store
                .native_replacement_origin(&key, command.clone())
                .unwrap();
            let prepared = super::super::allocation::prepared_target_for_scene_named(
                &["stream.subscribe"],
                (f, key, command, origin, sources),
                "isolation-return",
            );
            let movement = fixture_prepared(true, prepared);
            let _capture = activation_barrier::captured(&movement);
            setup = Setup {
                movement,
                original_lease: setup.original_lease,
                other_primary_leases: setup.other_primary_leases,
                healthy_source: setup.healthy_source,
                healthy_target: setup.healthy_target,
            };
            (setup, Call::Adopted(old.clone()), old)
        }
    }
}

struct Rendezvous {
    calls: AtomicUsize,
    publications: AtomicUsize,
    emitted: std::sync::atomic::AtomicBool,
    entered: mpsc::SyncSender<bool>,
    release: Mutex<mpsc::Receiver<()>>,
    until: Instant,
}
impl Rendezvous {
    fn held(&self) -> crate::Result<()> {
        if !self.emitted.swap(true, Ordering::SeqCst) {
            self.entered.send(true).map_err(|_| Error::storage())?;
            self.release
                .lock()
                .unwrap()
                .recv_timeout(self.until.saturating_duration_since(Instant::now()))
                .map_err(|_| {
                    Error::new("counter-timeout", "actual Writer checkpoint not released")
                })?;
        }
        Ok(())
    }
}
struct Absent<'a> {
    origin: &'a NativeMediaOrigin,
    probe: Option<&'a Rendezvous>,
}
impl crate::NativeOfferRuntimeObservation for Absent<'_> {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        if let Some(probe) = self.probe {
            if probe.calls.fetch_add(1, Ordering::SeqCst) == 1 {
                probe.held()?;
            }
        }
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.origin.runtime_connection_id().into(),
            generation: self.origin.generation(),
        })
    }
}
struct Installed<'a> {
    probe: &'a Rendezvous,
    calls: AtomicUsize,
}
struct InstalledGuard<'a>(&'a Rendezvous);
impl crate::NativeReplacementInstallationGuard for InstalledGuard<'_> {
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        self.0.publications.fetch_add(1, Ordering::SeqCst);
        commit()
    }
}
impl crate::NativeReplacementInstallationObservation for Installed<'_> {
    fn hold_installed(
        &self,
        _: &crate::NativeReplacementActivation,
    ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>> {
        // Initial observation and final installed-owner observation belong
        // to THIS Store invocation, not a prior definite pre-effect refusal.
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            self.probe.held()?;
        }
        Ok(Some(Box::new(InstalledGuard(self.probe))))
    }
}
impl Call {
    fn run(
        &self,
        store: &Store,
        affected: &NativeMediaOrigin,
        probe: &Rendezvous,
    ) -> crate::Result<()> {
        match self {
            Self::Physical { origin, sources } => {
                let writer = store.try_native_replacement_commit_scene_write(origin, sources)?;
                if !writer.current() {
                    return Err(Error::forbidden());
                }
                probe.held()?;
                drop(writer);
                Ok(())
            }
            Self::Public(capture) => store
                .project_native_replacement_committed(
                    capture,
                    &Installed {
                        probe,
                        calls: AtomicUsize::new(0),
                    },
                )
                .map(drop),
            Self::Original(lease) => store.try_settle_native_offer_cleanup(
                lease,
                &Absent {
                    origin: affected,
                    probe: Some(probe),
                },
            ),
            Self::Adopted(origin) => store.settle_native_replacement_cleanup(
                origin,
                &Absent {
                    origin: affected,
                    probe: Some(probe),
                },
            ),
        }
    }
    // Same opaque metadata receipt/absence only, AFTER all live IO and watcher
    // owners drain. Never invokes Core or resubmits an original physical effect.
    fn settle_after_draining(
        &self,
        store: &Store,
        affected: &NativeMediaOrigin,
    ) -> crate::Result<()> {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let result = match self {
                Self::Physical { .. } => Ok(()),
                Self::Public(capture) => store
                    .project_native_replacement_committed(capture, &MetadataOnlyInstalled)
                    .map(drop),
                Self::Original(lease) => store.settle_native_offer_cleanup_with_wait(
                    lease,
                    &Absent {
                        origin: affected,
                        probe: None,
                    },
                    &crate::AuthorityWait::new(until.saturating_duration_since(Instant::now())),
                ),
                Self::Adopted(origin) => store.settle_native_replacement_cleanup(
                    origin,
                    &Absent {
                        origin: affected,
                        probe: None,
                    },
                ),
            };
            match result {
                Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                result => return result,
            }
        }
    }
}
fn register_pair(
    store: &Arc<Store>,
    target: &Arc<NativeMediaOrigin>,
    source: Option<&Arc<NativeMediaOrigin>>,
) -> crate::NativeMediaSnapshot {
    let weak = Arc::downgrade(store);
    let to = target.clone();
    let from = source.cloned();
    store
        .register_native_media_snapshot_for(target, source.map(AsRef::as_ref), move || {
            weak.upgrade()
                .ok_or_else(Error::forbidden)?
                .try_native_media_read(&to, from.as_deref())
        })
        .unwrap()
}
fn lifecycle(store: &Store, sid: &str) -> String {
    use sha2::{Digest, Sha256};
    let c = rusqlite::Connection::open_with_flags(
        store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut all = Vec::new();
    for (table, column) in [
        ("sessions", "id"),
        ("session_participants", "session_id"),
        ("session_connected_presence", "session_id"),
        ("session_participant_facts", "session_id"),
        ("session_started_facts", "session_id"),
        ("session_reconnect_boundaries", "session_id"),
    ] {
        let mut q = c
            .prepare(&format!(
                "SELECT * FROM {table} WHERE {column}=?1 ORDER BY 1,2"
            ))
            .unwrap();
        let columns = q.column_count();
        let rows = q
            .query_map([sid], |r| {
                Ok((0..columns)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>())
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        all.push((table, rows));
    }
    hex::encode(Sha256::digest(serde_json::to_vec(&all).unwrap()))
}
fn run(kind: Kind) {
    let (setup, call, affected) = prepare(kind);
    let Setup {
        movement: m,
        healthy_source,
        healthy_target,
        ..
    } = setup;
    // All real current/root/ready/flow qualifiers precede any epoch admission.
    let sid = m.command.session_id.clone();
    let crate::PeerCommand::NativeConnectionOffer(candidate) = &m.target_request.command else {
        panic!()
    };
    let pending_target_absent: bool =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM connections WHERE id=?1)",
                [&candidate.connection_id],
                |r| r.get(0),
            )
            .unwrap();
    let joined_presence:u64=m.f.store.connection().unwrap().query_row(
        "SELECT COUNT(*) FROM session_connected_presence WHERE session_id=?1 AND state='joined'",[&sid],|r|r.get(0)).unwrap();
    let started: u64 =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM session_started_facts WHERE session_id=?1",
                [&sid],
                |r| r.get(0),
            )
            .unwrap();
    assert!(
        joined_presence >= 4 && started == 1,
        "nonvacuous actual controlled-flow lifecycle setup missing"
    );
    assert!(
        pending_target_absent,
        "candidate unexpectedly materialized before publication"
    );
    let store = Arc::new(m.f.store);
    let before_lifecycle = lifecycle(&store, &sid);
    let before_database = Store::inspect_snapshot(store.path()).unwrap();
    let snapshot = register_pair(&store, &healthy_target, Some(&healthy_source));
    let affected_snapshot = register_pair(&store, &affected, None);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let original = admit(&snapshot);
    let old = admit(&affected_snapshot);
    let stamp = original.snapshot_stamp().unwrap();
    let epoch = original.packet_epoch().unwrap();
    let old_stamp = old.snapshot_stamp().unwrap();
    let old_epoch = old.packet_epoch().unwrap();
    assert!(original.current() && epoch.current() && old.current() && old_epoch.current());
    assert!(original.permits_subscriptions());
    assert_eq!(
        original.target().connection.connection_id,
        healthy_target.connection_id()
    );
    assert_eq!(
        original.source().unwrap().connection.connection_id,
        healthy_source.connection_id()
    );
    for view in [original.target(), original.source().unwrap()] {
        assert!(
            view.session.session.media.iter().any(|m| m == "audio")
                && view.session.session.media.iter().any(|m| m == "video")
        );
        assert!(view.offer["streams_offered"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "audio"
                && s["codec_preferences"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == "opus")));
        assert!(view.offer["streams_offered"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "video"
                && s["codec_preferences"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == "vp8")));
    }
    // The affected epoch remains queued, but it must not pin Writer admission.
    drop(old);
    let until = Instant::now() + Duration::from_secs(2);
    let (entered, events) = mpsc::sync_channel(2);
    let (release, resume) = mpsc::sync_channel(1);
    let probe = Rendezvous {
        calls: AtomicUsize::new(0),
        publications: AtomicUsize::new(0),
        emitted: std::sync::atomic::AtomicBool::new(false),
        entered: entered.clone(),
        release: Mutex::new(resume),
        until,
    };
    let pre_effect_registry_retries = AtomicUsize::new(0);
    let admission_trace = admission_trace::buffer();
    let refused_attempts = Mutex::new(Vec::<serde_json::Value>::new());
    let (
        held_checkpoint,
        first_result,
        survived,
        same_stamp,
        fresh_pair,
        affected_fenced,
        within_original_cutoff,
    ) = std::thread::scope(|workers| {
        let worker = workers.spawn(|| {
            let result = loop {
                let before_admission = if let Call::Public(capture) = &call {
                    Some(capture.projection_diagnostics())
                } else {
                    None
                };
                let result = if matches!(kind, Kind::PublicReady) {
                    admission_trace::capture(&admission_trace, || {
                        call.run(&store, &affected, &probe)
                    })
                } else {
                    call.run(&store, &affected, &probe)
                };
                if let (Call::Public(capture), Err(error)) = (&call, &result) {
                    let mut attempts = refused_attempts.lock().unwrap();
                    if attempts.len() < 32 {
                        let unchanged = Store::inspect_snapshot(store.path())
                            .map(|snapshot| snapshot == before_database);
                        attempts.push(json!({
                            "code":error.code, "message":error.message,
                            "before":before_admission, "after":capture.projection_diagnostics(),
                            "final_installed_checkpoint":probe.emitted.load(Ordering::SeqCst),
                            "publisher_callbacks":probe.publications.load(Ordering::SeqCst),
                            "durable_snapshot_unchanged":unchanged.as_ref().ok(),
                            "snapshot_error":unchanged.as_ref().err().map(|e|(&e.code,&e.message))
                        }));
                    }
                }
                if matches!(kind, Kind::PublicReady)
                    && !probe.emitted.load(Ordering::SeqCst)
                    && result.as_ref().err().is_some_and(|e| {
                        e.code == "authority-busy"
                            && e.message == "Current authority reader registry is busy"
                    })
                    && Instant::now() < until
                    && original.current()
                    && epoch.current()
                {
                    let same = snapshot.admit_original(stamp);
                    let same_current = same.as_ref().is_ok_and(|r| r.current());
                    drop(same);
                    if !same_current {
                        break result;
                    }
                    pre_effect_registry_retries.fetch_add(1, Ordering::SeqCst);
                    // SAME immutable capture and original2s cutoff; no complete
                    // Writer checkpoint/publisher was reached. Never retry a
                    // kernel refusal, later error, effect or completed callback.
                    std::thread::sleep(
                        Duration::from_millis(1)
                            .min(until.saturating_duration_since(Instant::now())),
                    );
                    continue;
                }
                break result;
            };
            let _ = entered.send(false);
            result
        });
        let held = events.recv_timeout(Duration::from_secs(3)) == Ok(true);
        let survived = original.current() && epoch.current();
        let same = snapshot.admit_original(stamp);
        let same_stamp = same.as_ref().is_ok_and(|r| r.current());
        drop(same);
        let affected_fenced =
            !old_epoch.current() && affected_snapshot.admit_original(old_stamp).is_err();
        let _ = release.send(());
        let result = worker.join();
        // The second observation parks a real Writer and primary SQL Unit.
        // Exact original stamp/current epoch are sampled WHILE held above.
        // Fresh SQL derivation is AFTER return, still before IO/watcher
        // drain and under the SAME original metadata cutoff.
        let fresh_started_in_time = Instant::now() < until;
        let fresh = store.try_native_media_read(&healthy_target, Some(&healthy_source));
        let fresh_pair = fresh
            .as_ref()
            .is_ok_and(|r| r.current() && r.permits_subscriptions());
        drop(fresh);
        let within_original_cutoff = fresh_started_in_time && Instant::now() < until;

        (
            held,
            result,
            survived,
            same_stamp,
            fresh_pair,
            affected_fenced,
            within_original_cutoff,
        )
    });
    // Record original sample BEFORE watcher shutdown revokes publications.
    let queued_survived = epoch.current();
    let affected_still_fenced_before_shutdown =
        !old_epoch.current() && affected_snapshot.admit_original(old_stamp).is_err();
    drop(original);
    let watcher_result = watcher.shutdown();
    let first_result = match first_result {
        Ok(r) => r,
        Err(first) => std::panic::resume_unwind(first),
    };
    // All writer and watcher owners are closed BEFORE the causal assertions.
    // Complete old definite-Busy metadata after reader drain to observe actual
    // reconcile behavior; do not simulate readiness or repair appearance rows.
    let settlement = if first_result.is_err() {
        call.settle_after_draining(&store, &affected)
    } else {
        Ok(())
    };
    let after_lifecycle = lifecycle(&store, &sid);
    let after_database = Store::inspect_snapshot(store.path()).unwrap();
    watcher_result.unwrap();
    settlement.unwrap();
    let old_epoch_never_restored =
        !old_epoch.current() && affected_snapshot.admit_original(old_stamp).is_err();
    if matches!(kind, Kind::PublicReady) {
        // All original worker/IO/watcher owners have joined before formatting
        // these bounded scalar events. No additional retry is introduced.
        eprintln!(
            "scoped-public-admission events={:?} refused_attempts={} publisher_callbacks={}",
            admission_trace::snapshot(&admission_trace),
            serde_json::to_string(&*refused_attempts.lock().unwrap()).unwrap(),
            probe.publications.load(Ordering::SeqCst)
        );
    }
    eprintln!(
        "scoped-handoff pre_effect_reader_registry_retries={} original_cutoff_unchanged=true",
        pre_effect_registry_retries.load(Ordering::SeqCst)
    );
    eprintln!("scoped-handoff phase={kind:?} full_writer_checkpoint={held_checkpoint} first_error={:?} original_current={survived} original_stamp_current={same_stamp} fresh_pair_current={fresh_pair} queued_epoch_current={queued_survived} affected_fenced={affected_fenced} old_epoch_never_restored={old_epoch_never_restored} affected_still_fenced_before_shutdown={affected_still_fenced_before_shutdown} within_original_cutoff={within_original_cutoff} lifecycle_unchanged={} controlled_store_flow_facts=true cleanup_joined=true",first_result.as_ref().err().map(|e|(&e.code,&e.message)),before_lifecycle==after_lifecycle);
    assert!(within_original_cutoff, "actual original/stamp/fresh observation exceeded unchanged2s cutoff; causal result unqualified");
    assert!(
        affected_fenced && affected_still_fenced_before_shutdown && old_epoch_never_restored,
        "affected source0 original epoch was not permanently fenced"
    );
    if !held_checkpoint {
        assert_eq!(
            first_result.as_ref().err().map(|e| e.code.as_str()),
            Some("authority-busy"),
            "not the actual broad-authority exclusion counter"
        );
    }
    if matches!(kind, Kind::PhysicalCommit) {
        assert_eq!(
            before_database, after_database,
            "counter must not invoke Core or persist a Commit attempt"
        );
    }
    // OLD fails HERE after all qualifiers, real method attempts and cleanup.
    assert!(survived&&same_stamp&&fresh_pair&&queued_survived,
        "scoped handoff Writer revoked the SAME healthy source1/target2 AV publication although neither binding is affected");
    assert!(
        held_checkpoint && first_result.is_ok(),
        "exact scoped Writer did not complete while unrelated original IO remained live"
    );
    if matches!(kind, Kind::OriginalCleanup | Kind::AdoptedCleanup) {
        assert_eq!(before_lifecycle,after_lifecycle,"valid current bounded handoff must preserve existing joined appearance until its real successor publishes or the claim expires/cancels");
    }
}
#[test]
fn physical_commit_writer_preserves_unaffected_original_av_pair_epoch_and_stamp() {
    run(Kind::PhysicalCommit);
}
#[test]
fn public_ready_writer_preserves_unaffected_original_av_pair_epoch_and_stamp() {
    run(Kind::PublicReady);
}
#[test]
fn original_custodian_cleanup_preserves_current_pending_handoff_presence_and_unaffected_av_epoch() {
    run(Kind::OriginalCleanup);
}
#[test]
fn adopted_return_cleanup_preserves_current_pending_handoff_presence_and_unaffected_av_epoch() {
    run(Kind::AdoptedCleanup);
}

mod negatives;

mod sql_controls;

mod sql_ready_controls;

mod admission_trace;
