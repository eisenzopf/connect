//! Metadata snapshots keep exact original attempt/authority and never perform IO.
use super::*;

#[test]
fn native_stream_metadata_snapshot_matches_individual_reads_and_excludes_foreign_origin() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let (_other, _, _, foreign) = parent_permissions_origin(true, true, false, false, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let (target, sources) = f
        .store
        .native_stream_media_reads_with_peer(
            &origin,
            &[origin.as_ref(), foreign.as_ref(), origin.as_ref()],
            &proof,
            &lease,
        )
        .unwrap();
    let single = f
        .store
        .native_stream_media_read_with_peer(&origin, Some(&origin), &proof, &lease)
        .unwrap();
    assert!(target.current() && target.permits_subscriptions());
    assert_eq!(sources.len(), 3);
    assert!(
        sources[1].is_none(),
        "foreign installation cannot lend a source"
    );
    for index in [0, 2] {
        let read = sources[index].as_ref().unwrap();
        assert!(read.current());
        assert_eq!(
            read.target().runtime_connection_id,
            single.target().runtime_connection_id
        );
        assert_eq!(
            read.source().unwrap().runtime_connection_id,
            single.source().unwrap().runtime_connection_id
        );
        assert!(std::sync::Arc::ptr_eq(read.interval(), single.interval()));
    }
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    assert_eq!(
        f.store
            .native_stream_media_reads_with_peer(&foreign, &[origin.as_ref()], &proof, &lease)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

#[test]
fn native_stream_metadata_snapshot_contention_and_cancel_return_no_partial_grants_or_effects() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let held = f.store.connection().unwrap();
    let unavailable =
        f.store
            .native_stream_media_reads_with_peer(&origin, &[origin.as_ref()], &proof, &lease);
    drop(held);
    assert_eq!(unavailable.err().unwrap().code, "authority-busy");
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    assert!(f
        .store
        .native_stream_media_reads_with_peer(&origin, &[origin.as_ref()], &proof, &lease)
        .is_ok());
    // Controlled metadata fault: a cancelled original attempt cannot supply a
    // snapshot even though its original physical/source metadata still exists.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
            [lease.record_id()],
        )
        .unwrap();
    assert_eq!(
        f.store
            .native_stream_media_reads_with_peer(&origin, &[origin.as_ref()], &proof, &lease)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
}

#[test]
fn accepted_stream_source_preflight_is_read_only_and_cannot_borrow_an_attempt() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let (_other, _, _, foreign) = parent_permissions_origin(true, true, false, false, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let counts = || {
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal"),
        )
    };
    let before = counts();
    let (target, sources) = f
        .store
        .native_stream_preflight_media_reads_with_peer(
            &origin,
            &[origin.as_ref(), foreign.as_ref(), origin.as_ref()],
            &proof,
        )
        .unwrap();
    assert!(target.current() && target.permits_subscriptions());
    assert!(sources[0].as_ref().unwrap().current());
    assert!(sources[1].is_none());
    assert!(sources[2].as_ref().unwrap().current());
    assert!(std::sync::Arc::ptr_eq(
        target.interval(),
        sources[0].as_ref().unwrap().interval()
    ));
    assert_eq!(
        before,
        counts(),
        "preflight creates no ledger/effect/journal writes"
    );
    assert_eq!(
        f.store
            .native_stream_preflight_media_reads_with_peer(&foreign, &[origin.as_ref()], &proof,)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let started = counts();
    assert_eq!(
        f.store
            .native_stream_preflight_media_reads_with_peer(&origin, &[], &proof,)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(started, counts());
    assert!(f
        .store
        .native_stream_media_read_with_peer(&origin, None, &proof, &lease)
        .unwrap()
        .current());
}
