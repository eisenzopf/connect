use super::*;

#[test]
fn native_packet_pair_rederives_exact_target_under_owned_source_reader_and_queued_writer() {
    let f = Fixture::new();
    let target = f.origin(0, "target");
    let source = f.origin(1, "source");
    let held = f.store.try_native_media_read(&source, None).unwrap();
    let foreign = Fixture::new();
    let other_source = foreign.origin(1, "source");
    let other = foreign
        .store
        .try_native_media_read(&other_source, None)
        .unwrap();
    assert_eq!(
        f.store
            .try_native_media_read_under(&target, &source, &other)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap();
    let peer = Store::open_administration(f.store.path(), f.store.limits.clone()).unwrap();
    let write = scope.clone();
    let writer = std::thread::spawn(move || {
        peer.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(2)))
    });
    crate::authority_fences::test_wait_for_writer(&f.store, &scope);
    assert_eq!(
        f.store
            .try_native_media_read(&target, Some(&source))
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    let read = f
        .store
        .try_native_media_read_under(&target, &source, &held)
        .unwrap();
    assert!(read.current());
    assert_eq!(read.target().connection.participant_id, f.pids[0]);
    assert_eq!(read.source().unwrap().connection.participant_id, f.pids[1]);
    assert_eq!(
        read.source().unwrap().runtime_connection_id,
        source.runtime_connection_id()
    );
    assert!(read.expires_at() <= held.expires_at());
    drop(held);
    assert!(!writer.is_finished());
    drop(read);
    drop(writer.join().unwrap().unwrap());
    assert!(f
        .store
        .try_native_media_read(&target, Some(&source))
        .is_ok());
}

#[test]
fn native_packet_pair_owned_file_custody_cannot_supply_target_named_permission() {
    let mut f = Fixture::new();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("stream.subscribe".into());
    f.keys[0] = f
        .store
        .issue(&actor.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id;
    let target = f.origin(0, "receiver");
    let source = f.origin(1, "publisher");
    let held = f.store.try_native_media_read(&source, None).unwrap();
    assert!(f
        .store
        .try_native_media_read_under(&target, &source, &held)
        .unwrap()
        .permits_subscriptions());
    // Deliberate SQL bypass tests fresh policy, not supported writer behavior.
    // Real waiting-writer custody is verified by the separate test above.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![
                f.keys[0],
                serde_json::to_string(&Scope::collaboration(vec!["*".into()]).operations).unwrap()
            ],
        )
        .unwrap();
    let narrowed = f
        .store
        .try_native_media_read_under(&target, &source, &held)
        .unwrap();
    assert!(narrowed.current());
    assert!(!narrowed.permits_subscriptions());
    assert!(
        held.current(),
        "live source custody is not the target permission"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='retired' WHERE id=?1",
            [target.connection_id()],
        )
        .unwrap();
    assert!(f
        .store
        .try_native_media_read_under(&target, &source, &held)
        .is_err());
}

#[test]
fn native_packet_pair_owned_reader_cannot_extend_original_source_expiry() {
    let f = Fixture::new();
    let target = f.origin(0, "receiver");
    let request = f.native_request_for(1, 600, "expiring-publisher");
    let mut proof = f.admit(&request);
    proof.limit_expiry(crate::now() + 2).unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    f.store
        .prepare_connection_with_peer(&f.keys[1], intent.clone(), f.endpoints[1].clone(), &proof)
        .unwrap();
    let lease = f.media_attempt(1, &proof);
    let source = f
        .store
        .native_media_origin_with_peer(&f.keys[1], &proof, &lease)
        .unwrap();
    drop(proof);
    let held = f.store.try_native_media_read(&source, None).unwrap();
    let paired = f
        .store
        .try_native_media_read_under(&target, &source, &held)
        .unwrap();
    assert!(paired.current());
    assert!(paired.expires_at() <= held.expires_at());
    std::thread::sleep(Duration::from_millis(2100));
    assert!(!held.current());
    assert!(!paired.current());
    assert_eq!(
        f.store
            .try_native_media_read_under(&target, &source, &held)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}
