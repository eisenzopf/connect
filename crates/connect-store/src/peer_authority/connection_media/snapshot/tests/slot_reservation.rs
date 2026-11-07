//! Actual compound projection boundary: SQL unlock, competing reader, same reseal.
use super::*;
fn all_four_sql_slots_available(store: &Store) {
    let observation = store.packet_connections.observe().unwrap();
    let guards = (0..crate::packet_observation::OBSERVERS)
        .map(|_| store.try_packet_connection(&observation).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(guards.len(), 4);
}
#[test]
fn compound_projection_reserves_original_slot_while_actual_competing_reader_owns_another() {
    let f = Fixture::new();
    let competitor = f.store.packet_connections.observe().unwrap();
    let original = std::cell::Cell::new(None);
    let mut competing_sql = None;
    let (facts, grant) = acquire_with(
        &f.store,
        |c, stamp| {
            original.set(Some(stamp.connection));
            f.derive(c)
        },
        |c| {
            assert_eq!(f.derive(c)?.0, 1);
            Ok(())
        },
        |scopes| {
            let id = original.get().expect("actual original capture happened");
            drop(
                f.store.packet_connections[id.slot_index()]
                    .try_lock()
                    .expect("initial SQL really ends before file admission"),
            );
            let sql = f.store.try_packet_connection(&competitor)?;
            assert_ne!(
                sql.id(),
                id,
                "independent original cannot steal the reserved reseal slot"
            );
            competing_sql = Some(sql);
            f.store.try_acquire_authority(scopes)
        },
    )
    .unwrap();
    assert_eq!(facts, 1);
    assert!(competing_sql.is_some());
    drop(competing_sql);
    drop(competitor);
    // Keep the actual returned authority grant live. It must own neither the
    // original slot reservation nor SQL, even for its whole later media use.
    all_four_sql_slots_available(&f.store);
    drop(grant);
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
}
#[test]
fn compound_slot_claim_releases_on_derive_admission_finish_and_runtime_cancellation_errors() {
    for stage in ["derive", "admission", "finish"] {
        let f = Fixture::new();
        let result = acquire_with(
            &f.store,
            |c, _| {
                if stage == "derive" {
                    Err(Error::new("controlled", "derive"))
                } else {
                    f.derive(c)
                }
            },
            |_| {
                if stage == "finish" {
                    Err(Error::new("controlled", "finish"))
                } else {
                    Ok(())
                }
            },
            |scopes| {
                if stage == "admission" {
                    Err(Error::new("controlled", "admission"))
                } else {
                    f.store.try_acquire_authority(scopes)
                }
            },
        );
        assert_eq!(result.err().unwrap().code, "controlled", "{stage}");
        all_four_sql_slots_available(&f.store);
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    }
    let f = Fixture::new();
    let owner = f.store.require_runtime_owner().unwrap();
    let result = acquire_with(
        &f.store,
        |c, _| f.derive(c),
        |_| Ok(()),
        |scopes| {
            let held = f.store.try_acquire_authority(scopes)?;
            owner.retire();
            Ok(held)
        },
    );
    assert!(
        result.is_err(),
        "fresh runtime authority must still refuse reseal"
    );
    // Direct resource-only observers establish claim release after the actual
    // runtime was retired; these reads are not executable media authority.
    let observation = f.store.packet_connections.observe().unwrap();
    let guards = (0..crate::packet_observation::OBSERVERS)
        .map(|_| {
            f.store
                .packet_connections
                .connection(&observation, None)
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(guards.len(), 4);
}
