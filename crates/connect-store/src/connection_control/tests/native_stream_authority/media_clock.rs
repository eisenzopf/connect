//! Actual Store admissions with controlled policy seconds; no media IO claim.
use super::*;
use std::sync::{Arc, Barrier};

fn origins() -> (Fixture, Vec<Arc<crate::NativeMediaOrigin>>) {
    let mut f = Fixture::new();
    for index in 0..3 {
        let identity = f.store.revalidate(&f.keys[index]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[index] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let origins = (0..3)
        .map(|index| consultation_reads::ready(&f, index))
        .collect();
    (f, origins)
}
fn occupy_except(
    f: &Fixture,
    selected: usize,
) -> Vec<std::sync::MutexGuard<'_, crate::packet_observation::Slot>> {
    f.store
        .packet_connections
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected)
        .map(|(_, slot)| slot.lock().unwrap())
        .collect()
}
fn counts(f: &Fixture, origin: &crate::NativeMediaOrigin, paired: bool) -> [u64; 3] {
    f.store.native_media_memo_counts_for_test(origin, paired)
}
fn fingerprint(read: &crate::NativeMediaRead) -> serde_json::Value {
    json!({"target":read.target().connection,"source":read.source().map(|value| &value.connection),
        "expiry":read.expires_at(),"subscribe":read.permits_subscriptions()})
}

#[test]
fn native_media_positive_memo_crosses_clock_seconds_with_fresh_authority() {
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
    let (f, origins) = origins();
    let writer = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    for paired in [false, true] {
        let source = paired.then_some(origins[1].as_ref());
        for selected in 0..crate::packet_observation::OBSERVERS {
            clock.set(base + 1);
            let _occupied = occupy_except(&f, selected);
            let cold = f.store.try_native_media_read(&origins[0], source).unwrap();
            let expected = fingerprint(&cold);
            assert!(cold.expires_at() > base + 3);
            drop(cold);
            let before = counts(&f, &origins[0], paired);
            // The backwards step stays inside the ORIGINAL valid interval.
            for second in [base + 3, base + 2] {
                clock.set(second);
                let admissions = f.store.authority.test_admission_attempts();
                let read = f.store.try_native_media_read(&origins[0], source).unwrap();
                assert_eq!(fingerprint(&read), expected);
                assert_eq!(f.store.authority.test_admission_attempts(), admissions + 1);
                assert_eq!(
                    f.store
                        .try_acquire_authority(&[writer.clone()])
                        .unwrap_err()
                        .code,
                    "authority-busy"
                );
                drop(read);
            }
            let after = counts(&f, &origins[0], paired);
            assert_eq!(
                after[0], before[0],
                "clock-only steps rederived positive facts"
            );
            assert_eq!(after[1], before[1] + 2);
            if paired {
                // Publishing another source at a later clock must not evict
                // this still-valid original source merely because seconds differ.
                drop(
                    f.store
                        .try_native_media_read(&origins[0], Some(&origins[2]))
                        .unwrap(),
                );
                let after_other_source = counts(&f, &origins[0], true);
                drop(f.store.try_native_media_read(&origins[0], source).unwrap());
                assert_eq!(counts(&f, &origins[0], true)[0], after_other_source[0]);
            }
            let before_rollback = counts(&f, &origins[0], paired);
            // Earlier than capture requires fresh facts, but is not itself a
            // refusal: this fixture's actual keys were already valid at base.
            clock.set(base);
            let fresh = f.store.try_native_media_read(&origins[0], source).unwrap();
            assert!(fresh.current());
            assert!(counts(&f, &origins[0], paired)[0] > before_rollback[0]);
            drop(fresh);
            let held = f.store.try_acquire_authority(&[writer.clone()]).unwrap();
            assert_eq!(
                f.store
                    .try_native_media_read(&origins[0], source)
                    .err()
                    .unwrap()
                    .code,
                "authority-busy"
            );
            drop(held);
        }
    }
}

#[test]
fn native_media_positive_memo_rederives_at_original_expiry_without_renewing_hits() {
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
    let (f, origins) = origins();
    let _occupied = occupy_except(&f, 0);
    f.change_fixture()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.keys[0], base + 3],
        )
        .unwrap();
    let read = f.store.try_native_media_read(&origins[0], None).unwrap();
    assert_eq!(read.expires_at(), base + 3);
    drop(read);
    let before = counts(&f, &origins[0], false);
    for second in [base + 1, base + 2] {
        clock.set(second);
        let read = f.store.try_native_media_read(&origins[0], None).unwrap();
        assert_eq!(
            read.expires_at(),
            base + 3,
            "a hit extended original expiry"
        );
    }
    assert_eq!(counts(&f, &origins[0], false)[0], before[0]);
    clock.set(base + 3);
    assert!(f.store.try_native_media_read(&origins[0], None).is_err());
    assert!(counts(&f, &origins[0], false)[0] > before[0]);
    // Only a real updated credential and fresh derivation may extend validity.
    f.change_fixture()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.keys[0], base + 60],
        )
        .unwrap();
    assert!(
        f.store
            .try_native_media_read(&origins[0], None)
            .unwrap()
            .expires_at()
            > base + 3
    );
}

#[test]
fn native_media_positive_memo_refuses_actual_credential_expiry_and_revocation() {
    for actor in [0, 1] {
        // Revocation is irreversible. Each scenario starts with its own live
        // credential, rather than reviving a credential for the expiry check.
        for revoke in [false, true] {
            let base = crate::now();
            let clock = crate::test_clock::Clock::at(base);
            let (f, origins) = origins();
            let _occupied = occupy_except(&f, 0);
            drop(
                f.store
                    .try_native_media_read(&origins[0], Some(&origins[1]))
                    .unwrap(),
            );
            clock.set(base + 1);
            drop(
                f.store
                    .try_native_media_read(&origins[0], Some(&origins[1]))
                    .unwrap(),
            );
            let before = counts(&f, &origins[0], true);
            let external = rusqlite::Connection::open(f.store.path()).unwrap();
            assert!(external
                .execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&f.keys[actor]]
                )
                .is_err());
            drop(external);
            if revoke {
                f.change_fixture()
                    .execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&f.keys[actor]],
                    )
                    .unwrap();
                assert!(f
                    .store
                    .try_native_media_read(&origins[0], Some(&origins[1]))
                    .is_err());
            } else {
                f.change_fixture()
                    .execute(
                        "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                        params![f.keys[actor], base + 2],
                    )
                    .unwrap();
                assert_eq!(
                    f.store
                        .try_native_media_read(&origins[0], Some(&origins[1]))
                        .unwrap()
                        .expires_at(),
                    base + 2
                );
                clock.set(base + 2);
                assert!(f
                    .store
                    .try_native_media_read(&origins[0], Some(&origins[1]))
                    .is_err());
            }
            assert!(counts(&f, &origins[0], true)[0] > before[0]);
        }
    }
}

#[test]
fn native_media_positive_memo_rechecks_external_state_schema_and_current_recipient() {
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
    let (f, origins) = origins();
    let _occupied = occupy_except(&f, 0);
    let read = || {
        f.store
            .try_native_media_read(&origins[0], Some(&origins[1]))
    };
    drop(read().unwrap());
    clock.set(base + 1);
    let held = read().unwrap();
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    let before = counts(&f, &origins[0], true);
    external
        .execute_batch("CREATE TABLE media_clock_external_marker(value INTEGER)")
        .unwrap();
    let under = f
        .store
        .try_native_media_read_under(&origins[0], &origins[1], &held)
        .unwrap();
    assert!(under.current());
    assert!(counts(&f, &origins[0], true)[0] > before[0]);
    drop(under);
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut limited = actor.scope.clone();
    limited
        .operations
        .retain(|operation| operation != "stream.subscribe");
    assert!(external
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .is_err());
    assert!(held.current());
    drop(held);
    f.change_fixture()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![
                f.keys[0],
                serde_json::to_string(&limited.operations).unwrap()
            ],
        )
        .unwrap();
    let current = read().unwrap();
    assert!(
        !current.permits_subscriptions(),
        "old positive receiver permission survived SQL change"
    );
    drop(current);
    let source_before = counts(&f, &origins[0], true);
    let different = f
        .store
        .try_native_media_read(&origins[0], Some(&origins[2]))
        .unwrap();
    assert_eq!(
        different.source().unwrap().runtime_connection_id,
        origins[2].runtime_connection_id()
    );
    assert!(counts(&f, &origins[0], true)[0] > source_before[0]);
    drop(different);
    // The raw external writer still cannot bypass coordinated authority.
    assert!(external
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .is_err());
    f.change_fixture()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    assert!(read().is_err());
}

#[test]
fn six_directed_media_reads_cross_clock_steps_with_current_background_reads() {
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
    let (f, origins) = origins();
    let pairs: Vec<_> = (0..3)
        .flat_map(|target| {
            (0..3)
                .filter(move |source| *source != target)
                .map(move |source| (target, source))
        })
        .collect();
    assert_eq!(pairs.len(), 6);
    for selected in 0..crate::packet_observation::OBSERVERS {
        let _occupied = occupy_except(&f, selected);
        for &(target, source) in &pairs {
            drop(
                f.store
                    .try_native_media_read(&origins[target], Some(&origins[source]))
                    .unwrap(),
            );
        }
    }
    let before: Vec<_> = origins
        .iter()
        .map(|origin| counts(&f, origin, true))
        .collect();
    let barrier = Barrier::new(7);
    let failures = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for &(target, source) in &pairs {
            let (f, origins, barrier) = (&f, &origins, &barrier);
            workers.push(scope.spawn(move || {
                let mut failures = Vec::new();
                let clock = crate::test_clock::Clock::at(base);
                for second in [base + 1, base + 2, base + 3] {
                    clock.set(second);
                    barrier.wait();
                    for _ in 0..8 {
                        let until = std::time::Instant::now() + Duration::from_secs(2);
                        loop {
                            match f
                                .store
                                .try_native_media_read(&origins[target], Some(&origins[source]))
                            {
                                Ok(read) => {
                                    if !read.current()
                                        || !read.permits_subscriptions()
                                        || read
                                            .source()
                                            .map(|value| value.runtime_connection_id.as_str())
                                            != Some(origins[source].runtime_connection_id())
                                    {
                                        failures
                                            .push(format!("wrong current route {target}/{source}"));
                                    }
                                    break;
                                }
                                Err(error)
                                    if error.code == "authority-busy"
                                        && std::time::Instant::now() < until =>
                                {
                                    std::thread::yield_now()
                                }
                                Err(error) => {
                                    failures.push(format!(
                                        "bounded current route read failed: {error:?}"
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                    barrier.wait();
                }
                failures
            }));
        }
        let mut failures = Vec::new();
        for second in [base + 1, base + 2, base + 3] {
            clock.set(second);
            // Deterministic counterexample before releasing concurrent work:
            // a clock-only step must not cold-start every observer/pair. During
            // contention, an unavailable memo lock may legitimately rederive.
            for selected in 0..crate::packet_observation::OBSERVERS {
                let _occupied = occupy_except(&f, selected);
                for &(target, source) in &pairs {
                    let derived = counts(&f, &origins[target], true)[0];
                    match f
                        .store
                        .try_native_media_read(&origins[target], Some(&origins[source]))
                    {
                        Ok(read) => {
                            drop(read);
                            if counts(&f, &origins[target], true)[0] != derived {
                                failures.push(format!("clock-only route derived at {second}/{target}/{source}/{selected}"));
                            }
                        }
                        Err(error) => {
                            failures.push(format!("serial current route read: {error:?}"))
                        }
                    }
                }
            }
            barrier.wait();
            // These are real background read paths, with their original
            // separate readers. This test does not claim physical media timing.
            if let Err(error) = f.store.session_lifecycle_hint_page(0) {
                failures.push(format!("background lifecycle read: {error:?}"));
            }
            if let Err(error) = f.store.session_reconnect_page("") {
                failures.push(format!("background reconnect read: {error:?}"));
            }
            barrier.wait();
        }
        for worker in workers {
            failures.extend(worker.join().unwrap());
        }
        failures
    });
    assert!(failures.is_empty(), "{failures:?}");
    for (index, origin) in origins.iter().enumerate() {
        let after = counts(&f, origin, true);
        assert!(
            after[1] >= before[index][1] + 24,
            "serial observer/pair checks did not exercise positive memo reuse"
        );
    }
    let external = rusqlite::Connection::open(f.store.path()).unwrap();
    assert!(external
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[1]])
        .is_err());
    f.change_fixture()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[1]])
        .unwrap();
    assert!(f
        .store
        .try_native_media_read(&origins[0], Some(&origins[1]))
        .is_err());
}
