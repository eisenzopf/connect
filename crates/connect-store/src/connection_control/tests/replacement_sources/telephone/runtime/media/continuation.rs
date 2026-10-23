//! Controlled captures qualify journal fences; they do not qualify physical SIP IO.
use super::*;
use crate::{
    TelephoneReplacementPhase as Step, TelephoneReplacementResourcesCapture,
    TelephoneReplacementSceneRead,
};
fn captured(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    key: &str,
) -> Arc<TelephoneReplacementResourcesCapture> {
    let (_, media) = known(f, origin, sources, key);
    let read = f.store.try_telephone_replacement_media_read(media).unwrap();
    let attempt = f
        .store
        .begin_telephone_replacement_resources(&read)
        .unwrap();
    let capture = Arc::new(attempt.prepared(resources(&read)).unwrap());
    drop(read);
    f.store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    capture
}
fn observation(read: &TelephoneReplacementSceneRead, phase: &str) -> serde_json::Value {
    json!({"candidate":read.audio().candidate,"generation":read.audio().generation,"phase":phase,"attached":true,"private":true,"activated_muted":phase!="attach","media_ready":phase=="readiness"})
}
#[test]
fn telephone_private_scene_requires_all_native_origins_and_fences_route_writer_without_fake_peer() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let capture = captured(&f, &origin, &sources, &key);
    assert!(f
        .store
        .try_telephone_replacement_scene_read(capture.clone(), &sources[..1])
        .is_err());
    assert!(f
        .store
        .try_telephone_replacement_private_read(capture.clone())
        .is_err());
    let read = f
        .store
        .try_telephone_replacement_scene_read(capture.clone(), &sources)
        .unwrap();
    assert!(read.current());
    assert!(read.existing(&read.audio().candidate, None).is_none());
    for source in &sources {
        assert!(read
            .existing(source.runtime_connection_id(), None)
            .is_some());
    }
    let scope = crate::AuthorityScope::identity(
        &read.media().route_actor().identity_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    assert!(f.store.try_acquire_authority(&[scope.clone()]).is_err());
    drop(read);
    let writer = f.store.try_acquire_authority(&[scope]).unwrap();
    assert!(f
        .store
        .try_telephone_replacement_scene_read(capture, &sources)
        .is_err());
    drop(writer);
}
#[test]
fn telephone_private_phases_are_ordered_once_and_actual_audience_revisions_are_required() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let capture = captured(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_scene_read(capture.clone(), &sources)
        .unwrap();
    assert!(f
        .store
        .begin_telephone_replacement_phase(&read, Step::Activate)
        .is_err());
    let attempt = f
        .store
        .begin_telephone_replacement_phase(&read, Step::Attach)
        .unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_phase(&read, Step::Attach)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let attached = attempt.applied(observation(&read, "attach")).unwrap();
    drop(read);
    f.store
        .record_telephone_replacement_phase(&attached)
        .unwrap();
    f.store
        .record_telephone_replacement_phase(&attached)
        .unwrap();
    let read = f
        .store
        .try_telephone_replacement_scene_read(capture.clone(), &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_telephone_replacement_phase(&read, Step::Activate)
        .unwrap();
    let applied = attempt.applied(observation(&read, "activate")).unwrap();
    assert!(read
        .capture_activation_revisions(Default::default())
        .is_err());
    let mut observed = std::collections::BTreeMap::new();
    for source in &sources {
        let rev:u64=f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[source.connection_id()],|r|r.get(0)).unwrap();
        observed.insert(source.runtime_connection_id().into(), rev + 1);
    }
    observed.insert(read.audio().candidate.clone(), 3);
    let audience = read.capture_activation_revisions(observed).unwrap();
    drop(read);
    assert!(f
        .store
        .record_telephone_replacement_phase(&applied)
        .is_err());
    let activated = applied.with_audience(audience).unwrap();
    f.store
        .record_telephone_replacement_phase(&activated)
        .unwrap();
    assert!(f
        .store
        .try_telephone_replacement_private_read(capture)
        .unwrap()
        .current());
    assert_eq!(f.count("telephone_replacement_phases"), 2);
}
#[test]
fn telephone_private_unknown_then_late_capture_preserves_knowledge_without_restarting() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let capture = captured(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_scene_read(capture.clone(), &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_telephone_replacement_phase(&read, Step::Attach)
        .unwrap();
    let attached = attempt.applied(observation(&read, "attach")).unwrap();
    drop(read);
    assert_eq!(
        f.store
            .record_telephone_replacement_phase(&attempt.uncertain())
            .unwrap()
            .state,
        OperationState::Unknown
    );
    let late = f
        .store
        .record_telephone_replacement_phase(&attached)
        .unwrap();
    assert_eq!(late.state, OperationState::Unknown);
    assert!(late.result.is_none());
    assert!(f
        .store
        .try_telephone_replacement_scene_read(capture, &sources)
        .is_err());
    assert_eq!(f.count("telephone_replacement_phases"), 1);
}
#[test]
fn telephone_private_capture_rollback_retries_same_fact_and_immutable_phase_is_retained() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let capture = captured(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_scene_read(capture, &sources)
        .unwrap();
    let attempt = f
        .store
        .begin_telephone_replacement_phase(&read, Step::Attach)
        .unwrap();
    let attached = attempt.applied(observation(&read, "attach")).unwrap();
    drop(read);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER deny_phase_record BEFORE UPDATE OF observation_json ON telephone_replacement_phases BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .record_telephone_replacement_phase(&attached)
        .is_err());
    let state:String=f.store.connection().unwrap().query_row("SELECT e.state FROM telephone_replacement_phases p JOIN operation_effects e ON e.id=p.effect_id",[],|r|r.get(0)).unwrap();
    assert_eq!(state, "submitting");
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER deny_phase_record")
        .unwrap();
    f.store
        .record_telephone_replacement_phase(&attached)
        .unwrap();
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute_batch("DELETE FROM telephone_replacement_phases")
        .is_err());
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute_batch("UPDATE telephone_replacement_phases SET observation_json='{}'")
        .is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("telephone-private.sqlite3");
    let snapshot = f.store.backup(&path).unwrap();
    assert_eq!(snapshot.schema, crate::SCHEMA_VERSION);
    assert_eq!(crate::Store::inspect_snapshot(&path).unwrap(), snapshot);
    let backup = rusqlite::Connection::open(&path).unwrap();
    backup.execute_batch("DROP TRIGGER telephone_replacement_phase_immutable; UPDATE telephone_replacement_phases SET observation_json=json_set(observation_json,'$.private',0);").unwrap();
    drop(backup);
    assert_ne!(
        crate::Store::inspect_snapshot(&path).unwrap().digest,
        snapshot.digest
    );
}

mod commit;
