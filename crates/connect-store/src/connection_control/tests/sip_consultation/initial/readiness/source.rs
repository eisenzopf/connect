//! Physical metadata and source discrimination only, not private scene/audio qualification.
use super::*;

#[test]
fn private_sip_source_requires_committed_readiness_and_never_seeds_public_cache() {
    let (f, parent, _, _, media, mut observed, _) = consent();
    let c = f.store.connection().unwrap();
    assert!(f
        .store
        .initial_telephone_audience_facts(&c, &media)
        .is_err());
    assert!(f
        .store
        .private_initial_telephone_origins
        .lock()
        .unwrap()
        .is_empty());
    let public = f.store.session_ready_origins.lock().unwrap().len();
    drop(c);
    observed.changed = true;
    assert!(f
        .store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .is_err());
    assert!(f
        .store
        .private_initial_telephone_origins
        .lock()
        .unwrap()
        .is_empty());
    observed.changed = false;
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f
        .store
        .initial_telephone_audience_facts(&c, &media)
        .unwrap();
    assert_eq!(binding["source_kind"], "private-initial-sip");
    assert_eq!(binding["consultation_record_id"], parent.record_id());
    assert_eq!(f.store.session_ready_origins.lock().unwrap().len(), public);
    assert_eq!(
        f.store
            .private_initial_telephone_origins
            .lock()
            .unwrap()
            .len(),
        1
    );
    assert!(f
        .store
        .sip_audience_snapshot_facts(&c, &binding, &f.cid, "session")
        .is_ok());
    assert!(f
        .store
        .sip_audience_snapshot_facts(&c, &binding, "foreign", "session")
        .is_err());
    drop(c);
    assert!(f
        .store
        .try_initial_telephone_media_read(media.clone())
        .is_err());
    assert!(f
        .store
        .try_initial_telephone_signal_read(
            media,
            crate::InitialTelephoneSignalDirection::FromTelephone
        )
        .is_err());
    // The registry and facts retain no authority reader across idle time.
    drop(
        f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
}

#[test]
fn private_sip_physical_metadata_is_independent_of_ended_controller_execution_but_grants_no_io() {
    for fault in 0..3 {
        let (f, parent, _, _, media, observed, _) = consent();
        f.store
            .record_private_initial_telephone_connected(media.clone(), &observed)
            .unwrap();
        let c = f.store.connection().unwrap();
        let (_, before, _) = f
            .store
            .initial_telephone_audience_facts(&c, &media)
            .unwrap();
        let (sql, key) = match fault {
            0 => ("UPDATE credentials SET revoked=1 WHERE id=?1", f.keys[0].as_str()),
            1 => ("UPDATE credentials SET expires_at=0 WHERE id=?1", f.keys[0].as_str()),
            _ => ("UPDATE operations SET lease_owner='scene-worker',state='preparing' WHERE record_id=?1", parent.record_id()),
        };
        c.execute(sql, [key]).unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &media)
            .unwrap();
        assert_eq!(binding, before, "fault {fault}");
        assert!(f
            .store
            .sip_audience_snapshot_facts(&c, &binding, &f.cid, "session")
            .is_ok());
        drop(c);
        assert!(f
            .store
            .try_private_initial_telephone_admission_read(media.clone())
            .is_err());
        assert!(f.store.try_initial_telephone_media_read(media).is_err());
    }
}

#[test]
fn private_sip_snapshot_refuses_public_kind_foreign_root_and_changed_original_physical_fields() {
    let (f, _, _, _, media, observed, _) = consent();
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f
        .store
        .initial_telephone_audience_facts(&c, &media)
        .unwrap();
    assert!(Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
    for (key, value) in [
        ("consultation_record_id", json!(media.record_id())),
        ("record_id", json!("other-child")),
        ("connection_id", json!("conn-0")),
        ("runtime_connection_id", json!("other-core")),
        ("sip_call_id", json!("other-call")),
        ("generation", json!(2)),
        ("participant_id", json!(f.pids[0])),
        ("membership_epoch", json!(999)),
        ("identity_id", json!("other-identity")),
        ("endpoint_id", json!("other-endpoint")),
        ("endpoint_revision", json!(999)),
        ("device_id", json!("other-device")),
        ("endpoint_credential_id", json!(f.keys[0])),
        ("route_credential_id", json!(f.keys[0])),
        ("route_owner_id", json!("other-owner")),
        ("physical_expires_at", json!(0)),
    ] {
        let mut forged = binding.clone();
        forged[key] = value;
        assert!(
            f.store
                .sip_audience_snapshot_facts(&c, &forged, &f.cid, "session")
                .is_err(),
            "{key}"
        );
    }
    let mut public = binding.clone();
    public["source_kind"] = json!("initial-sip");
    // Strict typed deserialization also rejects smuggled private fields.
    assert!(Store::recorded_sip_live(&c, &public, f.store.runtime_epoch()).is_err());
    public
        .as_object_mut()
        .unwrap()
        .remove("consultation_record_id");
    assert!(!Store::recorded_sip_live(&c, &public, f.store.runtime_epoch()).unwrap());
    assert!(f
        .store
        .sip_audience_snapshot_facts(&c, &public, &f.cid, "session")
        .is_err());
    assert!(f
        .store
        .record_initial_telephone_audience_revision(&c, &public, f.store.runtime_epoch(), 2, 3)
        .is_err());
}

#[test]
fn private_sip_metadata_rechecks_own_keys_endpoint_device_presence_and_original_runtime() {
    for fault in 0..12 {
        let (f, _, policy, initial, media, observed, _) = consent();
        f.store
            .record_private_initial_telephone_connected(media.clone(), &observed)
            .unwrap();
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &media)
            .unwrap();
        let (sql, key) = match fault {
            0 => ("UPDATE credentials SET revoked=1 WHERE id=?1", policy.endpoint_credential_id.as_str()),
            1 => ("UPDATE credentials SET revoked=1 WHERE id=?1", policy.route_credential_id.as_str()),
            2 => ("UPDATE credentials SET operations='[]' WHERE id=?1", policy.endpoint_credential_id.as_str()),
            3 => ("UPDATE credentials SET operations='[]' WHERE id=?1", policy.route_credential_id.as_str()),
            4 => ("UPDATE endpoints SET revision=revision+1 WHERE id=?1", policy.endpoint_id.as_str()),
            5 => ("UPDATE devices SET revoked=1 WHERE id=?1", initial.source().device_id.as_str()),
            6 => ("UPDATE participants SET active=0 WHERE participant_id=?1", initial.source().target.participant_id.as_str()),
            7 => ("UPDATE session_participants SET state='left' WHERE participant_id=?1", initial.source().target.participant_id.as_str()),
            8 => ("UPDATE connections SET generation=generation+1 WHERE id=?1", initial.connection_id()),
            9 => ("UPDATE connections SET observed_runtime_id='other-runtime' WHERE id=?1", initial.connection_id()),
            10 => ("UPDATE connections SET readiness_json=json_set(readiness_json,'$.runtime_connection_id','other-core') WHERE id=?1", initial.connection_id()),
            _ => ("UPDATE operations SET cancel_requested=1 WHERE record_id=?1", initial.record_id()),
        };
        c.execute(sql, [key]).unwrap();
        assert!(
            f.store
                .initial_telephone_audience_facts(&c, &media)
                .is_err(),
            "fault {fault}"
        );
        assert!(
            f.store
                .sip_audience_snapshot_facts(&c, &binding, &f.cid, "session")
                .is_err(),
            "fault {fault}"
        );
        // Physical observation remains distinct from permission withdrawal.
        if fault < 8 {
            assert!(
                Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap(),
                "fault {fault}"
            );
        }
    }
}

#[test]
fn private_sip_registry_is_runtime_local_and_required_for_snapshot_resolution() {
    let (f, _, _, _, media, observed, _) = consent();
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f
        .store
        .initial_telephone_audience_facts(&c, &media)
        .unwrap();
    f.store
        .private_initial_telephone_origins
        .lock()
        .unwrap()
        .clear();
    assert!(f
        .store
        .sip_audience_snapshot_facts(&c, &binding, &f.cid, "session")
        .is_err());
    drop(c);
    // An actual trusted-observer replay can restore metadata, never an IO grant.
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let backup = f._dir.path().join("private-source.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    let c = restored.connection().unwrap();
    assert!(restored
        .private_initial_telephone_origins
        .lock()
        .unwrap()
        .is_empty());
    assert!(restored
        .initial_telephone_audience_facts(&c, &media)
        .is_err());
    assert!(restored
        .sip_audience_snapshot_facts(&c, &binding, &f.cid, "session")
        .is_err());
}

#[test]
fn private_sip_captured_revision_and_cleanup_keep_original_kind_without_reauthorizing_it() {
    let (f, _, policy, _, media, observed, signal) = consent();
    f.store
        .record_private_initial_telephone_connected(media.clone(), &observed)
        .unwrap();
    let binding = {
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &media)
            .unwrap();
        f.store
            .record_sip_audience_revision(&c, &binding, f.store.runtime_epoch(), 2, 3)
            .unwrap();
        assert_eq!(c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1", [media.connection_id()], |r|r.get::<_, u64>(0)).unwrap(), 3);
        assert_eq!(c.query_row("SELECT json_extract(connected_json,'$.stream_revision') FROM initial_telephone_answers WHERE operation_record_id=?1", [media.record_id()], |r|r.get::<_, u64>(0)).unwrap(), 2);
        c.execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [policy.endpoint_credential_id],
        )
        .unwrap();
        assert!(Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
        assert!(!Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
        binding
    };
    f.store
        .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
        .unwrap();
    assert!(f
        .store
        .private_initial_telephone_origins
        .lock()
        .unwrap()
        .is_empty());
    let c = f.store.connection().unwrap();
    assert!(!Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
    assert!(Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
    let mut public = binding.clone();
    public["source_kind"] = json!("initial-sip");
    public
        .as_object_mut()
        .unwrap()
        .remove("consultation_record_id");
    assert!(!Store::recorded_sip_departure(&c, &public, f.store.runtime_epoch()).unwrap());
    let mut foreign = binding;
    foreign["consultation_record_id"] = json!("other-root");
    assert!(!Store::recorded_sip_departure(&c, &foreign, f.store.runtime_epoch()).unwrap());
}
