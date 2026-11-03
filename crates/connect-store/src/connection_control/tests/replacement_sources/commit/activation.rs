//! Inactive installation metadata must never stand in for current authority.
use super::*;

#[test]
fn reserved_activation_metadata_is_inactive_until_exact_durable_completion_and_rechecks_revocation()
{
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let activation = attempt.activation().clone();
    let origin = activation.origin().clone();
    let expected = attempt.expected().unwrap();
    let observed = ready(&read, &expected);
    drop(read);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let writer =
        m.f.store
            .pending_native_replacement_data_writer(&activation)
            .unwrap();
    assert!(m.f.store.try_native_media_read(&origin, None).is_err());
    assert!(m.f.store.native_data_writer(&origin).is_err());
    assert!(m
        .f
        .store
        .try_reserve_native_data_write(
            &writer,
            "absent-operation",
            "absent-message",
            "absent-source"
        )
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert_eq!(activation.target().connection.state, "preparing");
    assert!(activation.target().publication_suspended);
    let other_dir = tempfile::tempdir().unwrap();
    let other =
        Store::initialize(&other_dir.path().join("other.sqlite3"), Default::default()).unwrap();
    assert!(other
        .pending_native_replacement_data_writer(&activation)
        .is_err());
    assert!(other.try_native_media_read(&origin, None).is_err());

    let captured = attempt.committed(expected, &observed).unwrap();
    assert!(Arc::ptr_eq(&activation, captured.activation()));
    assert!(m.f.store.try_native_media_read(&origin, None).is_err());
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    assert!(m.f.store.try_native_media_read(&origin, None).is_err());
    assert!(m.f.store.native_data_writer(&origin).is_err());

    // This is a Store lifecycle fixture, not evidence that the Service barrier
    // is installed. Both origins must resolve the same actual completed lifetime.
    let established =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(origin.offer_record_id(), established.offer_record_id());
    assert_eq!(
        origin.runtime_connection_id(),
        established.runtime_connection_id()
    );
    assert_eq!(origin.connection_id(), established.connection_id());
    assert_eq!(origin.generation(), established.generation());
    assert_eq!(origin.participant_id(), established.participant_id());
    assert_eq!(origin.membership_epoch(), established.membership_epoch());
    assert!(m
        .f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
    assert!(m.f.store.native_data_writer(&origin).is_ok());
    let completed = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let duplicate =
        m.f.store
            .project_native_replacement_committed(&captured)
            .unwrap();
    assert_eq!(
        duplicate.runtime_connection_id(),
        origin.runtime_connection_id()
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        completed
    );

    let admin = m.f.store.revalidate(&m.f.admin).unwrap();
    m.f.store
        .revoke(&admin, &m.target_request.credentials[0])
        .unwrap();
    assert!(m.f.store.try_native_media_read(&origin, None).is_err());
    assert!(m.f.store.try_native_media_read(&established, None).is_err());
}

#[test]
fn uncertain_commit_metadata_cannot_activate_a_route_or_reserve_data() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let activation = attempt.activation().clone();
    drop(read);
    let uncertain = attempt.uncertain();
    let result =
        m.f.store
            .native_replacement_commit_uncertain(&uncertain)
            .unwrap();
    assert_eq!(result.state, OperationState::Unknown);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let writer =
        m.f.store
            .pending_native_replacement_data_writer(&activation)
            .unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(activation.origin(), None)
        .is_err());
    assert!(m.f.store.native_data_writer(activation.origin()).is_err());
    assert!(m
        .f
        .store
        .try_reserve_native_data_write(
            &writer,
            "absent-operation",
            "absent-message",
            "absent-source"
        )
        .is_err());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
