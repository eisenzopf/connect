//! Actual-observer-shaped metadata hints; no SIP/Core runtime qualification.
use super::*;

#[test]
fn sip_ready_hint_tracks_original_connected_generation_runtime_and_captured_retirement() {
    for fault in ["runtime", "generation", "retirement"] {
        let (f, root, _, initial, media, observed, signal) = consent();
        assert!(!f
            .store
            .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
            .unwrap());
        f.store
            .record_private_initial_telephone_connected(media, &observed)
            .unwrap();
        let before = counts(&f);
        assert!(f
            .store
            .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
            .unwrap());
        drop(
            f.store
                .try_acquire_authority(&[AuthorityScope::conversation(
                    &f.cid,
                    AuthorityMode::Write,
                )
                .unwrap()])
                .unwrap(),
        );
        assert_eq!(counts(&f), before);
        match fault {
            "runtime" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE connections SET observed_runtime_id='other-runtime' WHERE id=?1",
                        [initial.connection_id()],
                    )
                    .unwrap();
            }
            "generation" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [initial.connection_id()],
                    )
                    .unwrap();
            }
            _ => f
                .store
                .settle_initial_telephone_cleanup(&signal, &ControlledAbsent(true))
                .unwrap(),
        }
        assert!(
            !f.store
                .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
                .unwrap(),
            "{fault}"
        );
    }
}

#[test]
fn sip_ready_hint_needs_current_named_consult_authority_and_owns_no_idle_reader() {
    let (f, root, _, _, media, observed, _) = consent();
    f.store
        .record_private_initial_telephone_connected(media, &observed)
        .unwrap();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = actor.scope.clone();
    scope.operations.retain(|op| op != "session.consult");
    let key = f
        .store
        .issue(&actor.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id;
    assert!(f
        .store
        .native_consultation_has_ready_leg(&key, root.record_id())
        .is_err());
    assert!(f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .unwrap());
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    assert!(f
        .store
        .native_consultation_has_ready_leg(&f.keys[0], root.record_id())
        .is_err());
    drop(
        f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
}
