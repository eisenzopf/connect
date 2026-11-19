//! Real Store transactions with deliberately controlled runtime observations.
//! These guards are NOT native/Core/F/decoded evidence or Service qualification.
use super::*;
use crate::{
    NativeReplacementPublicVideoExpected as Expected,
    NativeReplacementPublicVideoNativeBinding as Binding,
    NativeReplacementPublicVideoRuntimeGuard as RuntimeGuard,
    NativeReplacementPublicVideoRuntimeMetadata as Metadata,
    NativeReplacementPublicVideoRuntimeObservation as Observation,
    NativeReplacementPublicVideoStage as Stage,
};
use connect_protocol::transitions::OperationState;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum Publish {
    Commit,
    Refuse,
    Skip,
    SwallowExpired,
    SwallowRetired,
}
struct Controlled<'a> {
    original: &'a NativeReplacementPublicVideoReservation,
    operation: &'a str,
    stage: Stage,
    metadata: Metadata,
    available: bool,
    metadata_error: bool,
    publish: Publish,
    holds: AtomicUsize,
    publishes: AtomicUsize,
    callback_errors: AtomicUsize,
}
impl<'a> Controlled<'a> {
    fn new(
        r: &'a NativeReplacementPublicVideoReservation,
        c: &'a TransitionCommand,
        stage: Stage,
    ) -> Self {
        let binding = Binding {
            public_stream_id: r.intent().public_stream_id.clone(),
            mid: "2".into(),
            stream_generation: 3,
            physical_writer_incarnation: "controlled-store-writer-not-native-proof".into(),
            connection_generation: 7,
            primary_ssrc: 0,
            payload_type: 96,
            clock_rate_hz: 90000,
            codec: "vp8".into(),
        };
        let private_readiness_nonce = "original-private-challenge".to_owned();
        let metadata = match stage {
            Stage::NegotiatedNeverFed => Metadata::NegotiatedNeverFed {
                binding,
                private_readiness_nonce,
            },
            Stage::CutBoundPostF => Metadata::CutBoundPostF {
                binding,
                private_readiness_nonce,
                source_cut_id: "controlled-original-cut".into(),
                public_readiness_nonce: "original-public-challenge".into(),
            },
            Stage::DecodedReady => Metadata::DecodedReady {
                binding,
                private_readiness_nonce,
                source_cut_id: "controlled-original-cut".into(),
                public_readiness_nonce: "original-public-challenge".into(),
                decoded_frames: 1,
                decoded_keyframes: None,
            },
        };
        Self {
            original: r,
            operation: &c.operation_id,
            stage,
            metadata,
            available: true,
            metadata_error: false,
            publish: Publish::Commit,
            holds: AtomicUsize::new(0),
            publishes: AtomicUsize::new(0),
            callback_errors: AtomicUsize::new(0),
        }
    }
}
struct Held<'a, 'r>(&'a Controlled<'r>);
impl Observation for Controlled<'_> {
    fn hold_current(
        &self,
        expected: &Expected<'_>,
    ) -> crate::Result<Option<Box<dyn RuntimeGuard + '_>>> {
        self.holds.fetch_add(1, Ordering::SeqCst);
        assert!(expected.reservation().same_original(self.original));
        assert_eq!(expected.operation_id(), self.operation);
        assert_eq!(expected.stage(), self.stage);
        Ok(self
            .available
            .then(|| Box::new(Held(self)) as Box<dyn RuntimeGuard + '_>))
    }
}
impl RuntimeGuard for Held<'_, '_> {
    fn metadata(&self) -> crate::Result<Metadata> {
        if self.0.metadata_error {
            return Err(crate::Error::forbidden());
        }
        Ok(self.0.metadata.clone())
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        self.0.publishes.fetch_add(1, Ordering::SeqCst);
        match self.0.publish {
            Publish::Commit => commit(),
            Publish::Refuse => Err(crate::Error::forbidden()),
            Publish::Skip => Ok(()),
            Publish::SwallowExpired => {
                // A controlled retained-owner expiry, not native IO or a new
                // budget. Force the ORIGINAL callback's actual current check.
                let _clock = crate::test_clock::Clock::at(self.0.original.expires_at());
                let result = commit();
                assert_eq!(
                    result
                        .err()
                        .expect("the real callback must reject the expired original interval")
                        .code,
                    "forbidden"
                );
                self.0.callback_errors.fetch_add(1, Ordering::SeqCst);
                Ok(()) // Deliberately faulty trusted SPI; Store must still refuse.
            }
            Publish::SwallowRetired => unreachable!(),
        }
    }
}
fn attempt_observed(
    f: &Fixture,
    r: &NativeReplacementPublicVideoReservation,
    stage: Stage,
    observed: &Controlled<'_>,
) -> crate::Result<crate::PublicVideoOperationSnapshot> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    loop {
        let result = f
            .store
            .record_native_replacement_public_video_phase(r, stage, observed);
        if observed.holds.load(Ordering::SeqCst) == 0
            && result
                .as_ref()
                .err()
                .is_some_and(|e| e.code == "authority-busy")
            && std::time::Instant::now() < until
        {
            assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
            std::thread::yield_now();
        } else {
            return result;
        }
    }
}
fn issue(
    f: &Fixture,
    r: &NativeReplacementPublicVideoReservation,
    c: &TransitionCommand,
    stage: Stage,
) {
    let observation = Controlled::new(r, c, stage);
    retry_effect_free_metadata(f, || {
        f.store
            .record_native_replacement_public_video_phase(r, stage, &observation)
    })
    .unwrap();
    assert!(observation.holds.load(Ordering::SeqCst) >= 1);
    assert_eq!(observation.publishes.load(Ordering::SeqCst), 1);
}
fn decoded(f: &Fixture, r: &NativeReplacementPublicVideoReservation, c: &TransitionCommand) {
    for stage in [
        Stage::NegotiatedNeverFed,
        Stage::CutBoundPostF,
        Stage::DecodedReady,
    ] {
        issue(f, r, c, stage);
    }
}
fn snapshot(f: &Fixture, key: &str, c: &TransitionCommand) -> serde_json::Value {
    let value = same_original_snapshot(f, key, c);
    json!({"operation": value.operation, "intent": value.intent, "phase": value.phase})
}
#[test]
fn runtime_issuer_derives_exact_original_stages_and_replays_without_new_rows_or_nonce() {
    let (f, key, c, p, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(p.clone())
    })
    .unwrap();
    for (index, stage) in [
        Stage::NegotiatedNeverFed,
        Stage::CutBoundPostF,
        Stage::DecodedReady,
    ]
    .into_iter()
    .enumerate()
    {
        issue(&f, &r, &c, stage);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let projected = snapshot(&f, &key, &c);
        assert_eq!(projected["operation"]["state"], "preparing");
        let phase = &projected["phase"];
        assert_eq!(phase["reservation_id"], r.intent().reservation_id);
        assert_eq!(phase["operation_record_id"], r.operation_record_id());
        assert_eq!(phase["operation_id"], c.operation_id);
        assert_eq!(phase["target_connection_id"], r.candidate_connection_id());
        assert_eq!(phase["target_generation"], r.target_generation());
        assert_eq!(phase["binding"]["stream_id"], "public-b");
        assert_eq!(phase["binding"]["preview_stream_id"], "camera-a-0");
        assert_eq!(phase["binding"]["primary_ssrc"], 0);
        assert_eq!(phase["binding"]["stream_generation"], 3);
        assert_eq!(phase["binding"]["connection_generation"], 7);
        assert_eq!(
            f.count("native_replacement_public_video_phases"),
            index as u64 + 1
        );
        issue(&f, &r, &c, stage);
        assert_eq!(snapshot(&f, &key, &c), projected);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
#[test]
fn runtime_issuer_rejects_missing_retired_wrong_stage_binding_nonce_and_decoded_facts_without_mutation(
) {
    let (f, key, c, p, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(p.clone())
    })
    .unwrap();
    for fault in 0..8 {
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let mut observed = Controlled::new(&r, &c, Stage::NegotiatedNeverFed);
        match fault {
            0 => observed.available = false,
            1 => observed.metadata_error = true,
            2 => observed.metadata = Controlled::new(&r, &c, Stage::CutBoundPostF).metadata,
            3..=5 => {
                if let Metadata::NegotiatedNeverFed {
                    binding,
                    private_readiness_nonce,
                } = &mut observed.metadata
                {
                    if fault == 3 {
                        binding.public_stream_id = r.intent().preview_stream_id.clone();
                    }
                    if fault == 4 {
                        binding.mid = "wrong mid".into();
                    }
                    if fault == 5 {
                        *private_readiness_nonce = r.intent().nonce.clone();
                    }
                }
            }
            6 => observed.publish = Publish::Refuse,
            7 => observed.publish = Publish::Skip,
            _ => unreachable!(),
        }
        let result = attempt_observed(&f, &r, Stage::NegotiatedNeverFed, &observed);
        assert!(result.is_err(), "fault={fault}");
        assert_eq!(observed.holds.load(Ordering::SeqCst), 1, "fault={fault}");
        assert_eq!(
            Store::inspect_snapshot(f.store.path()).unwrap(),
            before,
            "fault={fault}"
        );
        assert!(snapshot(&f, &key, &c)["phase"].is_null());
    }
    issue(&f, &r, &c, Stage::NegotiatedNeverFed);
    for fault in 0..4 {
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let mut observed = Controlled::new(&r, &c, Stage::CutBoundPostF);
        if let Metadata::CutBoundPostF {
            binding,
            private_readiness_nonce,
            public_readiness_nonce,
            ..
        } = &mut observed.metadata
        {
            match fault {
                0 => binding.physical_writer_incarnation = "different-writer".into(),
                1 => binding.connection_generation += 1,
                2 => *public_readiness_nonce = private_readiness_nonce.clone(),
                3 => *public_readiness_nonce = r.intent().nonce.clone(),
                _ => unreachable!(),
            }
        }
        assert!(attempt_observed(&f, &r, Stage::CutBoundPostF, &observed).is_err());
        assert_eq!(observed.holds.load(Ordering::SeqCst), 1);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    issue(&f, &r, &c, Stage::CutBoundPostF);
    for frames in [0, 9007199254740992] {
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let mut observed = Controlled::new(&r, &c, Stage::DecodedReady);
        if let Metadata::DecodedReady { decoded_frames, .. } = &mut observed.metadata {
            *decoded_frames = frames;
        }
        assert!(attempt_observed(&f, &r, Stage::DecodedReady, &observed).is_err());
        assert_eq!(observed.holds.load(Ordering::SeqCst), 1);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
#[test]
fn runtime_issuer_sql_busy_foreign_and_actual_cancel_revoke_expiry_never_invoke_runtime_or_mint_phase(
) {
    for terminal in 0..3 {
        let (f, key, c, p, _, _) = setup(1, None);
        let r = retry_effect_free_metadata(&f, || {
            f.store
                .reserve_native_replacement_public_video_intent(p.clone())
        })
        .unwrap();
        let observed = Controlled::new(&r, &c, Stage::NegotiatedNeverFed);
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let sql = f.store.try_connection().unwrap();
        assert_eq!(
            f.store
                .record_native_replacement_public_video_phase(
                    &r,
                    Stage::NegotiatedNeverFed,
                    &observed
                )
                .err()
                .unwrap()
                .code,
            "authority-busy"
        );
        drop(sql);
        assert_eq!(observed.holds.load(Ordering::SeqCst), 0);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
        assert!(foreign
            .record_native_replacement_public_video_phase(&r, Stage::NegotiatedNeverFed, &observed)
            .is_err());
        drop(foreign);
        match terminal {
            0 => custody::cancel(&f, &key, &c),
            1 => {
                let admin = f.store.revalidate(&f.admin).unwrap();
                f.store.revoke(&admin, &key).unwrap();
            }
            2 => {}
            _ => unreachable!(),
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let _expired = (terminal == 2).then(|| crate::test_clock::Clock::at(r.expires_at()));
        assert!(f
            .store
            .record_native_replacement_public_video_phase(&r, Stage::NegotiatedNeverFed, &observed)
            .is_err());
        assert_eq!(observed.holds.load(Ordering::SeqCst), 0);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        assert_eq!(f.count("native_replacement_public_video_phases"), 0);
    }
}
#[test]
fn runtime_issuer_rejects_guard_ok_after_swallowed_actual_original_expiry_callback_error() {
    let (f, _key, c, p, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(p.clone())
    })
    .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let mut observed = Controlled::new(&r, &c, Stage::NegotiatedNeverFed);
    observed.publish = Publish::SwallowExpired;
    assert_eq!(
        attempt_observed(&f, &r, Stage::NegotiatedNeverFed, &observed)
            .err()
            .unwrap()
            .code,
        "storage-unavailable"
    );
    assert_eq!(observed.callback_errors.load(Ordering::SeqCst), 1);
    assert_eq!(observed.publishes.load(Ordering::SeqCst), 1);
    assert_eq!(f.count("native_replacement_public_video_phases"), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

struct Started {
    f: Fixture,
    key: String,
    command: TransitionCommand,
    reservation: NativeReplacementPublicVideoReservation,
    origin: Arc<crate::NativeReplacementMediaOrigin>,
    sources: Vec<Arc<crate::NativeMediaOrigin>>,
}
fn started() -> Started {
    let (f, key, command, prepared, mut request, sources) = setup(1, None);
    let reservation = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["webrtc_video_receive_reservation"] =
        serde_json::to_value(reservation.intent()).unwrap();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    drop(proof);
    f.store
        .record_native_replacement_allocated(&allocated)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f.store.native_replacement_media_origin(started).unwrap();
    for phase in [
        crate::NativeReplacementMediaPhase::Attach,
        crate::NativeReplacementMediaPhase::Activate,
        crate::NativeReplacementMediaPhase::Readiness,
    ] {
        let read = f
            .store
            .try_native_replacement_scene_write(&origin, &sources)
            .unwrap();
        let attempt = f
            .store
            .begin_native_replacement_media_phase(origin.clone(), phase, read.media())
            .unwrap();
        let expected = attempt.expected().unwrap();
        let capture = attempt.applied(expected).unwrap();
        drop(read);
        f.store
            .record_native_replacement_media_phase(&capture)
            .unwrap();
    }
    Started {
        f,
        key,
        command,
        reservation,
        origin,
        sources,
    }
}
fn capture(s: &Started) -> crate::NativeReplacementCommitted {
    let read =
        s.f.store
            .try_native_replacement_scene_write(&s.origin, &s.sources)
            .unwrap();
    let attempt =
        s.f.store
            .begin_native_replacement_commit(s.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let t = read.media().target();
    let observed = crate::NativeReadyObservation {
        private_consultation: None,
        handoff_preparation: false,
        conversation_id: t.connection.conversation_id.clone(),
        session_id: t.connection.session_id.clone(),
        runtime_connection_id: t.runtime_connection_id.clone(),
        participant_id: t.connection.participant_id.clone(),
        membership_epoch: t.connection.membership_epoch,
        endpoint_id: t.connection.endpoint_id.clone(),
        device_id: t.device_id.clone(),
        generation: t.connection.generation,
        stream_revision: 3,
        participant_revision: expected["participant_revision"].as_u64().unwrap(),
        streams: t.offer["streams_offered"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().into())
            .collect(),
    };
    let capture = attempt.committed(expected, &observed).unwrap();
    drop(read);
    s.f.store
        .record_native_replacement_committed(&capture)
        .unwrap();
    capture
}
struct Installation<'a> {
    original: &'a NativeReplacementPublicVideoReservation,
    operation: &'a str,
    available: bool,
    public_ready: bool,
    publish: Publish,
    checks: AtomicUsize,
    publishes: AtomicUsize,
    retire_owner: Option<Arc<crate::installation_ownership::InstallationOwnership>>,
    callback_errors: AtomicUsize,
}
impl<'a> Installation<'a> {
    fn new(s: &'a Started) -> Self {
        Self {
            original: &s.reservation,
            operation: &s.command.operation_id,
            available: true,
            public_ready: true,
            publish: Publish::Commit,
            checks: AtomicUsize::new(0),
            publishes: AtomicUsize::new(0),
            retire_owner: None,
            callback_errors: AtomicUsize::new(0),
        }
    }
}
struct Installed<'a, 'r>(&'a Installation<'r>);
impl crate::NativeReplacementInstallationObservation for Installation<'_> {
    fn hold_installed(
        &self,
        expected: &crate::NativeReplacementActivation,
    ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>> {
        assert_eq!(
            expected.target().connection.connection_id,
            self.original.candidate_connection_id()
        );
        assert_eq!(
            expected.target().connection.generation,
            self.original.target_generation()
        );
        Ok(self.available.then(|| {
            Box::new(Installed(self)) as Box<dyn crate::NativeReplacementInstallationGuard + '_>
        }))
    }
}
impl crate::NativeReplacementInstallationGuard for Installed<'_, '_> {
    fn require_public_video_ready(
        &self,
        expected: &crate::NativeReplacementPublicVideoPublication<'_>,
    ) -> crate::Result<()> {
        self.0.checks.fetch_add(1, Ordering::SeqCst);
        assert!(expected.reservation().same_original(self.0.original));
        assert_eq!(expected.operation_id(), self.0.operation);
        assert_eq!(expected.binding().stream_id, "public-b");
        assert_eq!(expected.binding().preview_stream_id, "camera-a-0");
        assert_eq!(expected.binding().primary_ssrc, 0);
        assert_eq!(expected.binding().connection_generation, 7);
        assert_eq!(expected.source_cut_id(), "controlled-original-cut");
        assert_eq!(
            expected.private_readiness_nonce(),
            "original-private-challenge"
        );
        assert_eq!(
            expected.public_readiness_nonce(),
            "original-public-challenge"
        );
        if self.0.public_ready {
            Ok(())
        } else {
            Err(crate::Error::forbidden())
        }
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        self.0.publishes.fetch_add(1, Ordering::SeqCst);
        match self.0.publish {
            Publish::Commit => commit(),
            Publish::Refuse => Err(crate::Error::forbidden()),
            Publish::Skip => Ok(()),
            Publish::SwallowExpired => unreachable!(),
            Publish::SwallowRetired => {
                // Actual original Store owner retirement, with no Store call,
                // SQL, new authorization or native operation in the guard.
                self.0
                    .retire_owner
                    .as_ref()
                    .expect("captured original owner")
                    .retire();
                let error = commit()
                    .err()
                    .expect("actual publication callback must reject retired original owner");
                assert_eq!(error.code, "forbidden");
                self.0.callback_errors.fetch_add(1, Ordering::SeqCst);
                Ok(()) // Faulty trusted SPI must not fabricate durable success.
            }
        }
    }
}
// Real old guard implementation intentionally omits the new method, proving its
// default refusal. This is not a native installation receipt.
struct DefaultInstallation;
struct DefaultGuard;
impl crate::NativeReplacementInstallationObservation for DefaultInstallation {
    fn hold_installed(
        &self,
        _: &crate::NativeReplacementActivation,
    ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>> {
        Ok(Some(Box::new(DefaultGuard)))
    }
}
impl crate::NativeReplacementInstallationGuard for DefaultGuard {
    fn publish(&self, commit: Box<dyn FnOnce() -> crate::Result<()> + '_>) -> crate::Result<()> {
        commit()
    }
}
#[test]
fn runtime_issuer_final_publication_requires_same_ready_guard_and_commits_activated_with_completed_atomically(
) {
    // Even an allowing controlled installation cannot replace a missing actual
    // ledger stage. All publication DML must roll back before it is consulted.
    for count in 0..3 {
        let s = started();
        for stage in [Stage::NegotiatedNeverFed, Stage::CutBoundPostF]
            .into_iter()
            .take(count)
        {
            issue(&s.f, &s.reservation, &s.command, stage);
        }
        let captured = capture(&s);
        let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
        let installed = Installation::new(&s);
        assert!(s
            .f
            .store
            .project_native_replacement_committed(&captured, &installed)
            .is_err());
        assert_eq!(installed.checks.load(Ordering::SeqCst), 0);
        assert_eq!(installed.publishes.load(Ordering::SeqCst), 0);
        assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
        assert!(s
            .f
            .store
            .try_native_media_read(captured.activation().origin(), None)
            .is_err());
    }
    let s = started();
    decoded(&s.f, &s.reservation, &s.command);
    let captured = capture(&s);
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    assert!(s
        .f
        .store
        .project_native_replacement_committed(&captured, &DefaultInstallation)
        .is_err());
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
    for fault in 0..4 {
        let mut installed = Installation::new(&s);
        match fault {
            0 => installed.available = false,
            1 => installed.public_ready = false,
            2 => installed.publish = Publish::Refuse,
            3 => installed.publish = Publish::Skip,
            _ => unreachable!(),
        }
        let result =
            s.f.store
                .project_native_replacement_committed(&captured, &installed);
        assert!(result.is_err(), "fault={fault}");
        assert_eq!(
            Store::inspect_snapshot(s.f.store.path()).unwrap(),
            before,
            "fault={fault}"
        );
        assert_eq!(
            snapshot(&s.f, &s.key, &s.command)["phase"]["phase"],
            "decoded-ready"
        );
        assert!(s
            .f
            .store
            .try_native_media_read(captured.activation().origin(), None)
            .is_err());
        if fault >= 2 {
            assert_eq!(installed.checks.load(Ordering::SeqCst), 1);
            assert_eq!(installed.publishes.load(Ordering::SeqCst), 1);
        }
    }
    let installed = Installation::new(&s);
    let established =
        s.f.store
            .project_native_replacement_committed(&captured, &installed)
            .unwrap();
    assert_eq!(installed.checks.load(Ordering::SeqCst), 1);
    assert_eq!(installed.publishes.load(Ordering::SeqCst), 1);
    let completed = snapshot(&s.f, &s.key, &s.command);
    assert_eq!(completed["operation"]["state"], "completed");
    assert_eq!(completed["phase"]["phase"], "activated");
    assert_eq!(
        completed["phase"]["source_cut_id"],
        "controlled-original-cut"
    );
    assert!(s
        .f
        .store
        .try_native_media_read(&established, None)
        .unwrap()
        .current());
    let after = Store::inspect_snapshot(s.f.store.path()).unwrap();
    s.f.store
        .project_native_replacement_committed(&captured, &installed)
        .unwrap();
    assert_eq!(snapshot(&s.f, &s.key, &s.command), completed);
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), after);
    assert_eq!(s.f.count("native_replacement_public_video_phases"), 4);
    let observed = Controlled::new(&s.reservation, &s.command, Stage::DecodedReady);
    assert!(s
        .f
        .store
        .record_native_replacement_public_video_phase(
            &s.reservation,
            Stage::DecodedReady,
            &observed
        )
        .is_err());
    assert_eq!(observed.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), after);

    // Historical known-move publication deliberately has a different lifetime
    // from preparation. Exercise its REAL owner.current() callback fence,
    // never assume the preparation deadline is its commit cutoff.
    let retired = started();
    decoded(&retired.f, &retired.reservation, &retired.command);
    let captured = capture(&retired);
    let before = Store::inspect_snapshot(retired.f.store.path()).unwrap();
    let mut installed = Installation::new(&retired);
    installed.publish = Publish::SwallowRetired;
    installed.retire_owner = Some(retired.f.store.require_runtime_owner().unwrap());
    assert_eq!(
        retired
            .f
            .store
            .project_native_replacement_committed(&captured, &installed)
            .err()
            .unwrap()
            .code,
        "storage-unavailable"
    );
    assert_eq!(installed.checks.load(Ordering::SeqCst), 1);
    assert_eq!(installed.publishes.load(Ordering::SeqCst), 1);
    assert_eq!(installed.callback_errors.load(Ordering::SeqCst), 1);
    assert_eq!(
        Store::inspect_snapshot(retired.f.store.path()).unwrap(),
        before
    );
    assert!(retired
        .f
        .store
        .try_native_media_read(captured.activation().origin(), None)
        .is_err());
}
struct Gone(String, u64);
impl crate::NativeOfferRuntimeObservation for Gone {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.0.clone(),
            generation: self.1,
        })
    }
}
#[test]
fn runtime_issuer_actual_commit_unknown_and_cancelled_terminal_refuse_new_observation_without_mutation(
) {
    let s = started();
    decoded(&s.f, &s.reservation, &s.command);
    let read =
        s.f.store
            .try_native_replacement_scene_write(&s.origin, &s.sources)
            .unwrap();
    let attempt =
        s.f.store
            .begin_native_replacement_commit(s.origin.clone(), &read)
            .unwrap();
    drop(read);
    let unknown =
        s.f.store
            .native_replacement_commit_uncertain(&attempt.uncertain())
            .unwrap();
    assert_eq!(unknown.state, OperationState::Unknown);
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    let observed = Controlled::new(&s.reservation, &s.command, Stage::DecodedReady);
    assert!(s
        .f
        .store
        .record_native_replacement_public_video_phase(
            &s.reservation,
            Stage::DecodedReady,
            &observed
        )
        .is_err());
    assert_eq!(observed.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);

    // The internal unbound terminal kernel intentionally refuses ANY prior
    // public binding phase. Keep this separate original before allocation and
    // all phase issuance; never fabricate physical Discarded from a bare enum.
    let (f, key, c, prepared, _, _) = setup(1, None);
    let r = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    custody::cancel(&f, &key, &c);
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let cleanup = f
        .store
        .native_replacement_known_unpublished_read(prepared, &wait)
        .unwrap();
    assert!(cleanup.current());
    f.store
        .test_append_public_video_terminal(&cleanup, wire::DiscardReason::Cancelled)
        .unwrap();
    drop(cleanup);
    assert_eq!(f.count("native_replacement_public_video_phases"), 0);
    assert_eq!(f.count("native_replacement_public_video_terminals"), 1);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let observed = Controlled::new(&r, &c, Stage::NegotiatedNeverFed);
    assert!(f
        .store
        .record_native_replacement_public_video_phase(&r, Stage::NegotiatedNeverFed, &observed)
        .is_err());
    assert_eq!(observed.holds.load(Ordering::SeqCst), 0);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn runtime_issuer_known_stopped_original_settles_without_activated_and_cannot_repair_it_on_replay()
{
    let s = started();
    decoded(&s.f, &s.reservation, &s.command);
    let captured = capture(&s);
    let original = captured.activation().origin();
    let gone = Gone(
        original.runtime_connection_id().into(),
        original.generation(),
    );
    let stopped =
        s.f.store
            .project_native_replacement_stopped(&captured, &gone)
            .unwrap();
    let result = snapshot(&s.f, &s.key, &s.command);
    assert_eq!(result["operation"]["state"], "completed");
    assert_eq!(result["operation"]["result"]["activation_state"], "stopped");
    assert_eq!(result["phase"]["phase"], "decoded-ready");
    assert!(s.f.store.try_native_media_read(&stopped, None).is_err());
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    s.f.store
        .project_native_replacement_stopped(&captured, &gone)
        .unwrap();
    assert!(s
        .f
        .store
        .project_native_replacement_committed(&captured, &Installation::new(&s))
        .is_err());
    assert_eq!(Store::inspect_snapshot(s.f.store.path()).unwrap(), before);
    assert_eq!(s.f.count("native_replacement_public_video_phases"), 3);
}

#[path = "runtime_issuer/closed.rs"]
mod closed;

#[path = "runtime_issuer/ready_peer.rs"]
mod ready_peer;
