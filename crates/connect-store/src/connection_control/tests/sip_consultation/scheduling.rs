//! Scheduling/execution lane isolation only, never allocation or media authority.
use super::*;

#[test]
fn reserved_sip_metadata_excludes_generic_execution_and_compensation_before_any_child() {
    let (f, root, policy) = fixture();
    f.store.reserve_sip_consultation_leg(&root, policy).unwrap();
    let before = counts(&f);
    assert!(f
        .store
        .claim_transition(&f.admin, &root.request().operation_id, 60)
        .unwrap()
        .is_none());
    assert_eq!(counts(&f), before);
    let c = f.store.connection().unwrap();
    assert!(c.query_row("SELECT state='accepted' AND lease_owner IS NULL AND worker_runtime IS NULL FROM operations WHERE record_id=?1",[root.record_id()],|r|r.get::<_,bool>(0)).unwrap());
    c.execute(
        "UPDATE operations SET state='preparing' WHERE record_id=?1",
        [root.record_id()],
    )
    .unwrap();
    c.execute(
        "UPDATE operations SET state='unknown' WHERE record_id=?1",
        [root.record_id()],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .claim_transition_cleanup(&f.admin, &root.request().operation_id, 60)
            .unwrap_err()
            .code,
        "conflict"
    );
    let c = f.store.connection().unwrap();
    assert!(c.query_row("SELECT lease_owner IS NULL AND worker_runtime IS NULL FROM operations WHERE record_id=?1",[root.record_id()],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM initial_telephone_operations",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn sip_ready_hint_refuses_public_sibling_and_unobserved_ready_metadata_without_effects() {
    let (f, root, policy) = fixture();
    assert!(!f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    let candidate = f.store.reserve_sip_consultation_leg(&root, policy).unwrap();
    assert!(!f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    let before = counts(&f);
    f.store.connection().unwrap().execute("UPDATE connections SET state='ready',observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![candidate.connection_id(),f.store.runtime_epoch(),json!({"private_audio":true,"private_consultation_record_id":root.record_id(),"runtime_connection_id":"unobserved","call_id":"unobserved"}).to_string()]).unwrap();
    assert!(!f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    assert_eq!(counts(&f), before);
}

#[test]
fn native_ready_hint_still_requires_the_original_registered_private_candidate() {
    let (f, sources, _, root) = consultation_reads::fixture();
    assert!(!f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    let candidate = consultation_audiences::candidate(&f, &root);
    assert_ne!(candidate.connection_id(), sources[2].connection_id());
    assert!(f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [candidate.connection_id()],
        )
        .unwrap();
    assert!(!f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
}
