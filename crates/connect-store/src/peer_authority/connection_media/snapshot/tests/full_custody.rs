//! Same compound projection/reseal path under an original complete interval.
use super::*;
#[test]
fn compound_projection_under_rejects_foreign_partial_expired_and_final_schema_change() {
    let f = Fixture::new();
    let foreign = Fixture::new();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let partial = f.store.try_acquire_authority(&[]).unwrap();
    let other = foreign
        .store
        .try_acquire_authority(&[foreign.read.clone()])
        .unwrap();
    for (proof, expiry) in [
        (&partial, crate::now() + 5),
        (&other, crate::now() + 5),
        (&held, crate::now() - 1),
    ] {
        let result = f.store.try_current_packet_projection_under(
            |c| {
                f.derive(c)
                    .map(|(facts, scopes)| (facts, scopes, crate::now() + 20))
            },
            |_, facts, _, expiry| Ok((*facts, expiry)),
            proof,
            expiry,
        );
        assert_eq!(result.err().unwrap().code, "forbidden");
    }
    let external = Connection::open(f.store.path()).unwrap();
    let result = f.store.try_current_packet_projection_under(
        |c| {
            f.derive(c)
                .map(|(facts, scopes)| (facts, scopes, crate::now() + 20))
        },
        |_, facts, _, expiry| {
            external
                .execute_batch("CREATE TABLE under_scene_boundary(id INTEGER)")
                .unwrap();
            Ok((*facts, expiry))
        },
        &held,
        crate::now() + 5,
    );
    assert_eq!(result.err().unwrap().code, "authority-busy");
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "failed projection keeps no borrowed interval"
    );
}
#[test]
fn compound_projection_under_revalidates_actual_clock_edge_and_keeps_exact_original_expiry() {
    let f = Fixture::new();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let original_expiry = crate::now() + 5;
    let calls = std::cell::Cell::new(0);
    let (value, expiry) = f
        .store
        .try_current_packet_projection_under(
            |c| {
                f.derive(c)
                    .map(|(facts, scopes)| (facts, scopes, crate::now() + 30))
            },
            |_, facts, interval, expiry| {
                assert!(Arc::ptr_eq(&held, &interval));
                assert!(expiry <= original_expiry);
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    let second = crate::now();
                    while crate::now() == second {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
                Ok((*facts, expiry))
            },
            &held,
            original_expiry,
        )
        .unwrap();
    assert_eq!(value, 1);
    assert!(calls.get() >= 2);
    assert_eq!(expiry, original_expiry);
    assert_eq!(Arc::strong_count(&held), 1);
}
