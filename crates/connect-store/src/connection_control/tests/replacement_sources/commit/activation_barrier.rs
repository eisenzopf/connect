//! Exact Store publication boundary; these cases do not claim native media IO.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Installation {
    calls: AtomicUsize,
    refuse_at: usize,
}
impl crate::NativeReplacementInstallationObservation for Installation {
    fn hold_installed(
        &self,
        _: &crate::NativeReplacementActivation,
    ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>> {
        Ok(
            (self.calls.fetch_add(1, Ordering::SeqCst) + 1 != self.refuse_at).then(|| {
                Box::new(MetadataOnlyInstallationGuard)
                    as Box<dyn crate::NativeReplacementInstallationGuard>
            }),
        )
    }
}
pub(super) fn captured(m: &Movement) -> crate::NativeReplacementCommitted {
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    let result = attempt.committed(expected, &observed).unwrap();
    drop(read);
    m.f.store
        .record_native_replacement_committed(&result)
        .unwrap();
    result
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
fn ready_events(m: &Movement, capture: &crate::NativeReplacementCommitted) -> u64 {
    m.f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [capture.activation().origin().offer_record_id()],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn readiness_requires_exact_installation_before_and_at_commit_and_failed_final_write_rolls_back() {
    let m = fixture(true);
    let capture = captured(&m);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    for refuse_at in [1, 2] {
        let observation = Installation {
            calls: AtomicUsize::new(0),
            refuse_at,
        };
        let error =
            m.f.store
                .project_native_replacement_committed(&capture, &observation)
                .unwrap_err();
        assert_eq!(error.code, "authority-busy");
        assert_eq!(observation.calls.load(Ordering::SeqCst), refuse_at);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        assert_eq!(ready_events(&m, &capture), 0);
        assert!(m
            .f
            .store
            .try_native_media_read(capture.activation().origin(), None)
            .is_err());
    }
    let foreign = rusqlite::Connection::open(m.f.store.path()).unwrap();
    foreign.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap_err();
    assert_eq!(error.code, "storage-unavailable");
    foreign.execute_batch("ROLLBACK").unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    let origin =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
            .unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
    assert!(ready_events(&m, &capture) > 0);
    let record =
        m.f.store
            .record_native_replacement_committed(&capture)
            .unwrap();
    assert_eq!(record.result.unwrap()["activation_state"], "ready");
    let after = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), after);
}

#[test]
fn stopped_before_activation_preserves_known_move_without_ready_or_retained_route_retirement() {
    for claimed in [false, true] {
        let m = if claimed {
            let (f, key, command, prepared, request, sources, _) =
                super::super::candidate::prepared_scene();
            fixture_prepared(true, (f, key, command, prepared, request, sources))
        } else {
            fixture(true)
        };
        let capture = captured(&m);
        let original = capture.activation().origin();
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        for absent in [
            Gone("wrong-target".into(), original.generation()),
            Gone(
                original.runtime_connection_id().into(),
                original.generation() + 1,
            ),
        ] {
            assert_eq!(
                m.f.store
                    .project_native_replacement_stopped(&capture, &absent)
                    .unwrap_err()
                    .code,
                "forbidden"
            );
            assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
        }
        let admin = m.f.store.revalidate(&m.f.admin).unwrap();
        m.f.store
            .revoke(&admin, &m.target_request.credentials[0])
            .unwrap();
        let absent = Gone(
            original.runtime_connection_id().into(),
            original.generation(),
        );
        let origin =
            m.f.store
                .project_native_replacement_stopped(&capture, &absent)
                .unwrap();
        assert_eq!(ready_events(&m, &capture), 0);
        assert!(m.f.store.try_native_media_read(&origin, None).is_err());
        assert!(m.f.store.native_data_writer(&origin).is_err());
        assert!(m
            .f
            .store
            .try_native_media_read(&m.sources[1], None)
            .unwrap()
            .current());
        let record =
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap();
        assert_eq!(record.state, OperationState::Completed);
        assert!(record.effects.iter().all(|e| e.attempt == 1));
        let result = record.result.unwrap();
        assert_eq!(result["activation_state"], "stopped");
        assert!(result["readiness"].is_null());
        assert!(result["physical_readiness"].is_object());
        assert!(result["retired_connection_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == origin.connection_id()));
        let (state,generation,observed,readiness):(String,u64,Option<String>,Option<String>) = m.f.store.connection().unwrap().query_row("SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id=?1", [origin.connection_id()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(state, "retired");
        assert_eq!(generation, origin.generation() + 1);
        assert!(observed.is_none() && readiness.is_none());
        let after = Store::inspect_snapshot(m.f.store.path()).unwrap();
        m.f.store
            .project_native_replacement_stopped(&capture, &absent)
            .unwrap();
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), after);
        assert_eq!(
            m.f.store
                .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
                .unwrap_err()
                .code,
            "conflict"
        );
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent)
            .unwrap();
        assert_eq!(ready_events(&m, &capture), 0);
    }
}

#[test]
fn only_known_current_pending_activation_is_temporary_and_revocation_remains_permanent() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let origin = attempt.activation().origin().clone();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    drop(read);
    assert_ne!(
        m.f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    let capture = attempt.committed(expected, &observed).unwrap();
    assert_ne!(
        m.f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    m.f.store
        .record_native_replacement_committed(&capture)
        .unwrap();
    assert_eq!(
        m.f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    let admin = m.f.store.revalidate(&m.f.admin).unwrap();
    m.f.store
        .revoke(&admin, &m.target_request.credentials[0])
        .unwrap();
    assert_ne!(
        m.f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert!(m
        .f
        .store
        .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(ready_events(&m, &capture), 0);
}

#[test]
fn installation_custody_survives_until_the_ready_transaction_is_visible() {
    struct Observation {
        path: std::path::PathBuf,
        record: String,
        drops: std::sync::Mutex<Vec<bool>>,
    }
    struct Guard<'a>(&'a Observation);
    impl crate::NativeReplacementInstallationGuard for Guard<'_> {
        fn publish(
            &self,
            commit: Box<dyn FnOnce() -> crate::Result<()> + '_>,
        ) -> crate::Result<()> {
            commit()
        }
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            // An independent reader can see only committed data. This checks
            // Store guard lifetime, not a Service/native removal race.
            let c = rusqlite::Connection::open(&self.0.path).unwrap();
            let ready: bool = c.query_row(
                "SELECT state='completed' AND json_extract(result_json,'$.activation_state')='ready' FROM operations WHERE record_id=?1",
                [&self.0.record], |r| Ok(r.get::<_, Option<bool>>(0)?.unwrap_or(false)),
            ).unwrap();
            self.0.drops.lock().unwrap().push(ready);
        }
    }
    impl crate::NativeReplacementInstallationObservation for Observation {
        fn hold_installed(
            &self,
            _: &crate::NativeReplacementActivation,
        ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>>
        {
            Ok(Some(Box::new(Guard(self))))
        }
    }
    let m = fixture(true);
    let capture = captured(&m);
    let observation = Observation {
        path: m.f.store.path().to_path_buf(),
        record: capture.activation().origin().offer_record_id().into(),
        drops: Default::default(),
    };
    m.f.store
        .project_native_replacement_committed(&capture, &observation)
        .unwrap();
    assert_eq!(*observation.drops.lock().unwrap(), vec![false, true]);
}

#[test]
fn final_publication_callback_refusal_rolls_back_without_ready_and_original_retry_commits_once() {
    struct Publication {
        refuse: std::sync::atomic::AtomicBool,
        calls: AtomicUsize,
    }
    struct Guard<'a>(&'a Publication);
    impl crate::NativeReplacementInstallationGuard for Guard<'_> {
        fn publish(
            &self,
            commit: Box<dyn FnOnce() -> crate::Result<()> + '_>,
        ) -> crate::Result<()> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            if self.0.refuse.load(Ordering::SeqCst) {
                return Err(Error::forbidden());
            }
            commit()
        }
    }
    impl crate::NativeReplacementInstallationObservation for Publication {
        fn hold_installed(
            &self,
            _: &crate::NativeReplacementActivation,
        ) -> crate::Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>>
        {
            Ok(Some(Box::new(Guard(self))))
        }
    }
    let m = fixture(true);
    let capture = captured(&m);
    let observation = Publication {
        refuse: std::sync::atomic::AtomicBool::new(true),
        calls: AtomicUsize::new(0),
    };
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .project_native_replacement_committed(&capture, &observation)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(observation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(ready_events(&m, &capture), 0);
    observation.refuse.store(false, Ordering::SeqCst);
    m.f.store
        .project_native_replacement_committed(&capture, &observation)
        .unwrap();
    let after = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let events = ready_events(&m, &capture);
    assert!(events > 0);
    assert_eq!(observation.calls.load(Ordering::SeqCst), 2);
    m.f.store
        .project_native_replacement_committed(&capture, &observation)
        .unwrap();
    assert_eq!(
        observation.calls.load(Ordering::SeqCst),
        2,
        "historical replay must not publish again"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), after);
    assert_eq!(ready_events(&m, &capture), events);
}
