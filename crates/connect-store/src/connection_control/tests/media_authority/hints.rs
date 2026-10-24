use super::*;

#[test]
fn native_observation_hints_do_not_admit_media_and_current_mutation_invalidates_them() {
    let f = Fixture::new();
    let origin = f.origin(0, "hint-source");
    assert!(f
        .store
        .try_native_session_role_hint(&origin)
        .unwrap()
        .is_none());
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    let role = read.target().presence_role.clone();
    drop(read);
    assert!(f.store.try_native_connection_current(&origin).unwrap());
    let write = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
        ])
        .unwrap();
    assert_eq!(
        f.store.try_native_session_role_hint(&origin).unwrap(),
        Some(role)
    );
    assert!(f.store.try_native_connection_current(&origin).unwrap());
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='retired' WHERE id=?1",
            [origin.connection_id()],
        )
        .unwrap();
    assert!(f
        .store
        .try_native_session_role_hint(&origin)
        .unwrap()
        .is_none());
    assert!(f.store.try_native_connection_current(&origin).is_err());
    drop(write);
    assert!(f.store.try_native_media_read(&origin, None).is_err());
}

#[test]
fn native_observation_hints_cannot_extend_actual_original_source_expiry() {
    let f = Fixture::new();
    let request = f.native_request_for(0, 600, "expiring-hints");
    let mut proof = f.admit(&request);
    proof.limit_expiry(crate::now() + 2).unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    let lease = f.media_attempt(0, &proof);
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    drop(f.store.try_native_media_read(&origin, None).unwrap());
    assert!(f.store.try_native_connection_current(&origin).unwrap());
    assert!(f
        .store
        .try_native_session_role_hint(&origin)
        .unwrap()
        .is_some());
    std::thread::sleep(Duration::from_millis(2100));
    assert!(f.store.try_native_connection_current(&origin).is_err());
    assert!(f.store.try_native_session_role_hint(&origin).is_err());
    assert!(f.store.try_native_media_read(&origin, None).is_err());
}
