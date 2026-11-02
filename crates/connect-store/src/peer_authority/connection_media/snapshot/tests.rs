//! Controlled SQL races test capture invalidation, not physical media delivery.
use super::*;
use std::time::{Duration, Instant};
mod hints;
mod observer_cache;
mod observer_custody;
mod owned;

#[test]
fn erased_packet_projection_memo_skips_only_unchanged_facts_and_finishes_every_grant() {
    for _ in 0..8 {
        let f = Fixture::new();
        let memo = Store::new_packet_projection_metadata::<i64>();
        let first: MetadataToken = Arc::new(1u8);
        let other: MetadataToken = Arc::new(2u8);
        let calls = std::cell::Cell::new(0);
        let finishes = std::cell::RefCell::new(Vec::new());
        let read = |source| {
            f.store.try_cached_packet_projection(
                memo.as_ref(),
                Some(source),
                |c| {
                    calls.set(calls.get() + 1);
                    f.derive(c)
                },
                |_, facts: &i64| {
                    finishes.borrow_mut().push(*facts);
                    Ok(())
                },
            )
        };
        let second = crate::now();
        let (value, held) = read(&first).unwrap();
        assert_eq!(value, 1);
        drop(held);
        let command = f.store.connection().unwrap();
        let (value, held) = read(&first).unwrap();
        assert_eq!(value, 1);
        assert_eq!(
            f.store
                .try_acquire_authority(&[f.write.clone()])
                .unwrap_err()
                .code,
            "authority-busy"
        );
        drop(held);
        // Immutable memo facts retain no grant after the invocation ends.
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
        let (value, held) = read(&other).unwrap();
        assert_eq!(value, 1);
        drop(held);
        drop(command);
        assert_eq!(
            *finishes.borrow(),
            vec![1, 1, 1],
            "memo hits still finish each actual sealed grant"
        );
        if crate::now() == second {
            assert_eq!(
                calls.get(),
                2,
                "one unchanged source reuses facts; another original source cannot"
            );
            return;
        }
    }
    panic!("No unchanged clock interval for the controlled memo boundary");
}

#[test]
fn alternating_original_sources_reuse_only_their_own_current_metadata() {
    for _ in 0..8 {
        let f = Fixture::new();
        let memo = Store::new_packet_projection_metadata::<i64>();
        let sources: Vec<MetadataToken> = (0..4).map(|id| Arc::new(id) as MetadataToken).collect();
        let calls = std::cell::Cell::new(0);
        let finishes = std::cell::Cell::new(0);
        let read = |source: &MetadataToken, index: i64, expected: i64| {
            f.store.try_cached_packet_projection(
                memo.as_ref(),
                Some(source),
                |c| {
                    calls.set(calls.get() + 1);
                    let (value, scopes) = f.derive(c)?;
                    Ok((value * 100 + index, scopes))
                },
                |_, value| {
                    assert_eq!(
                        *value, expected,
                        "another original source cannot supply cached facts"
                    );
                    finishes.set(finishes.get() + 1);
                    Ok(())
                },
            )
        };
        let second = crate::now();
        for _ in 0..3 {
            for selected in 0..f.store.packet_connections.len() {
                let occupied: Vec<_> = f
                    .store
                    .packet_connections
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != selected)
                    .map(|(_, c)| c.lock().unwrap())
                    .collect();
                for (index, source) in sources.iter().enumerate() {
                    let (value, held) = read(source, index as i64, 100 + index as i64).unwrap();
                    assert_eq!(value, 100 + index as i64);
                    drop(held);
                }
                drop(occupied);
            }
        }
        if crate::now() != second {
            continue;
        }
        assert_eq!(
            calls.get(),
            4 * f.store.packet_connections.len(),
            "alternating current counterparties must not evict one another on every packet"
        );
        assert_eq!(
            finishes.get(),
            3 * 4 * f.store.packet_connections.len(),
            "every cached observation still finishes a fresh complete file grant"
        );
        let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
        for (index, source) in sources.iter().enumerate() {
            assert_eq!(
                read(source, index as i64, 100 + index as i64)
                    .unwrap_err()
                    .code,
                "authority-busy"
            );
        }
        drop(writer);
        let external = Connection::open(f.store.path()).unwrap();
        external
            .execute("UPDATE packet_snapshot_test SET value=2", [])
            .unwrap();
        for selected in 0..f.store.packet_connections.len() {
            let occupied: Vec<_> = f
                .store
                .packet_connections
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != selected)
                .map(|(_, c)| c.lock().unwrap())
                .collect();
            for (index, source) in sources.iter().enumerate() {
                let (value, held) = read(source, index as i64, 200 + index as i64).unwrap();
                assert_eq!(
                    value,
                    200 + index as i64,
                    "a commit invalidates every source/observer pair"
                );
                drop(held);
            }
            drop(occupied);
        }
        for _ in 0..32 {
            let source: MetadataToken = Arc::new(99u8);
            drop(read(&source, 17, 217).unwrap());
            let entries = memo.downcast_ref::<Memo<i64>>().unwrap().0.lock().unwrap();
            assert!(
                entries.len() <= 4 * crate::packet_observation::OBSERVERS,
                "counterparty churn cannot grow retained metadata without bound"
            );
        }
        return;
    }
    panic!("No unchanged clock interval for the controlled alternating-source boundary");
}

#[test]
fn current_packet_metadata_survives_unrelated_sql_custody_but_observes_commits_and_authority() {
    let f = Fixture::new();
    let memo = Memo::default();
    assert_eq!(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap().0, 1);

    // An unrelated control reader can retain the primary SQL mutex while
    // media remains authorized. Packet policy must still derive current facts
    // and enter a fresh complete authority interval without borrowing SQL.
    let primary = f.store.connection().unwrap();
    let (value, held) = cached(&f.store, &memo, None, |c| f.derive(c))
        .expect("unrelated primary SQL custody must not mute current packet reads");
    assert_eq!(value, 1);
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.write.clone()])
            .unwrap_err()
            .code,
        "authority-busy",
        "independent SQL observation still owns fresh file custody"
    );
    drop(held);

    // A committed same-Store update invalidates old metadata even though its
    // read-only observer did not execute the write or change total_changes.
    primary
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    assert_eq!(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap().0, 2);
    let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
    assert_eq!(
        cached(&f.store, &memo, None, |c| f.derive(c))
            .unwrap_err()
            .code,
        "authority-busy",
        "a metadata hit never bypasses current exclusive authority"
    );
    drop(writer);
    assert_eq!(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap().0, 2);
    drop(primary);
}

#[test]
fn fresh_projection_resolves_once_when_sql_and_clock_are_unchanged() {
    let f = Fixture::new();
    for _ in 0..8 {
        let second = crate::now();
        let mut resolutions = 0;
        let (value, held) = acquire(&f.store, |c| {
            resolutions += 1;
            f.derive(c)
        })
        .unwrap();
        assert_eq!(value, 1);
        assert!(f.store.try_connection().is_ok(), "SQL ends before delivery");
        assert_eq!(
            f.store
                .try_acquire_authority(&[f.write.clone()])
                .unwrap_err()
                .code,
            "authority-busy",
            "fresh current file custody survives SQL release"
        );
        drop(held);
        if crate::now() == second {
            assert_eq!(
                resolutions, 1,
                "unchanged complete projection is not repeated"
            );
            return;
        }
    }
    panic!("No unchanged policy-clock interval available for the controlled read test");
}

#[test]
fn unchanged_packet_projection_copy_does_not_hold_sql_against_other_current_reads() {
    struct Projection {
        store: Arc<Store>,
        copies: Arc<std::sync::Mutex<Vec<bool>>>,
    }
    impl Clone for Projection {
        fn clone(&self) -> Self {
            self.copies
                .lock()
                .unwrap()
                .push(self.store.packet_connections[0].try_lock().is_ok());
            Self {
                store: self.store.clone(),
                copies: self.copies.clone(),
            }
        }
    }
    let f = Fixture::new();
    let memo = Memo::default();
    let copies = Arc::new(std::sync::Mutex::new(Vec::new()));
    for _ in 0..8 {
        let second = crate::now();
        let derive = |_: &Connection| {
            Ok((
                Projection {
                    store: f.store.clone(),
                    copies: copies.clone(),
                },
                vec![f.read.clone()],
            ))
        };
        drop(cached(&f.store, &memo, None, derive).unwrap());
        copies.lock().unwrap().clear();
        drop(cached(&f.store, &memo, None, derive).unwrap());
        if crate::now() == second {
            let observed = copies.lock().unwrap();
            assert!(!observed.is_empty());
            assert!(observed.iter().all(|available| *available), "Unchanged projection copies retained SQL against a concurrent current read: {observed:?}");
            return;
        }
    }
    panic!("No unchanged policy-clock interval available for the controlled copy test");
}

#[test]
fn optimistic_metadata_uses_only_fresh_scope_coverage_after_a_version_change() {
    let f = Fixture::new();
    let memo = Memo::default();
    let scopes = [
        f.read.clone(),
        AuthorityScope::identity("changed-packet", AuthorityMode::Read).unwrap(),
        AuthorityScope::identity("latest-packet", AuthorityMode::Read).unwrap(),
    ];
    let derive = |c: &Connection| {
        let (value, _) = f.derive(c)?;
        Ok((value, vec![scopes[(value - 1) as usize].clone()]))
    };
    drop(cached(&f.store, &memo, None, derive).unwrap());
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    // Old metadata scope contention is not current authority. A changed SQL
    // version must derive and admit the new complete scope set instead.
    let old_writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
    let (value, current) = cached(&f.store, &memo, None, derive).unwrap();
    assert_eq!(value, 2);
    drop(current);
    drop(old_writer);
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE packet_snapshot_test SET value=3", [])
        .unwrap();
    let latest_writer = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::identity("latest-packet", AuthorityMode::Write).unwrap(),
        ])
        .unwrap();
    assert_eq!(
        cached(&f.store, &memo, None, derive).unwrap_err().code,
        "authority-busy"
    );
    // The optimistic old reader is disposable, and cannot lend its different
    // file custody to the refusing current scope or survive failed admission.
    drop(
        f.store
            .try_acquire_authority(&[AuthorityScope::identity(
                "changed-packet",
                AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    drop(latest_writer);
    assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 3);
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    read: AuthorityScope,
    write: AuthorityScope,
}

#[test]
fn unchanged_packet_metadata_is_retained_for_each_bounded_sql_observer() {
    let f = Fixture::new();
    for _ in 0..8 {
        let memo = Memo::default();
        let second = crate::now();
        let mut resolutions = 0;
        for _ in 0..3 {
            for selected in 0..f.store.packet_connections.len() {
                // Force real independent SQL custody, without swapping or
                // pretending equal data_version values identify a connection.
                let occupied: Vec<_> = f
                    .store
                    .packet_connections
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != selected)
                    .map(|(_, connection)| connection.lock().unwrap())
                    .collect();
                let (value, held) = cached(&f.store, &memo, None, |c| {
                    resolutions += 1;
                    f.derive(c)
                })
                .unwrap();
                assert_eq!(value, 1);
                drop(held);
                drop(occupied);
            }
        }
        if crate::now() != second {
            continue;
        }
        assert_eq!(
            resolutions,
            f.store.packet_connections.len(),
            "unchanged observers must not continually evict each other's validated metadata"
        );
        let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
        assert_eq!(
            cached(&f.store, &memo, None, |c| f.derive(c))
                .unwrap_err()
                .code,
            "authority-busy",
            "every metadata hit still needs a fresh complete file interval"
        );
        drop(writer);
        let external = Connection::open(f.store.path()).unwrap();
        for value in 2..=12 {
            external
                .execute("UPDATE packet_snapshot_test SET value=?1", [value])
                .unwrap();
            for selected in 0..f.store.packet_connections.len() {
                let occupied: Vec<_> = f
                    .store
                    .packet_connections
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != selected)
                    .map(|(_, connection)| connection.lock().unwrap())
                    .collect();
                assert_eq!(
                    cached(&f.store, &memo, None, |c| f.derive(c)).unwrap().0,
                    value,
                    "an external commit must invalidate every observer's old metadata"
                );
                drop(occupied);
            }
            assert!(memo.0.lock().unwrap().len()<=f.store.packet_connections.len(),
                "repeated external commits cannot grow retained metadata beyond the bounded observer pool");
        }
        return;
    }
    panic!("No unchanged clock interval for the controlled observer test");
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(
                &dir.path().join("state.sqlite3"),
                connect_protocol::Limits::default(),
            )
            .unwrap(),
        );
        store
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TABLE packet_snapshot_test(value INTEGER NOT NULL);
                 INSERT INTO packet_snapshot_test VALUES(1);",
            )
            .unwrap();
        Self {
            _dir: dir,
            store,
            read: AuthorityScope::identity("packet-test", AuthorityMode::Read).unwrap(),
            write: AuthorityScope::identity("packet-test", AuthorityMode::Write).unwrap(),
        }
    }
    fn derive(&self, c: &Connection) -> Result<(i64, Vec<AuthorityScope>)> {
        let value = c
            .query_row("SELECT value FROM packet_snapshot_test", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        Ok((value, vec![self.read.clone()]))
    }
}

#[test]
fn compound_packet_projection_clock_edge_revalidates_under_original_interval_and_expiry() {
    let f = Fixture::new();
    let calls = std::cell::Cell::new(0);
    let mut writer = None;
    let mut original_interval = None;
    let expiries = std::cell::RefCell::new(Vec::new());
    let result = f.store.try_current_packet_projection(
        |c| {
            let (facts, scopes) = f.derive(c)?;
            let expiry = crate::now() + 5;
            expiries.borrow_mut().push(expiry);
            Ok((facts, scopes, expiry))
        },
        |_, facts, held, expiry| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                original_interval = Some(held.clone());
                let store = f.store.clone();
                let scope = f.write.clone();
                writer = Some(std::thread::spawn(move || {
                    store.acquire_authority(
                        &[scope],
                        &crate::AuthorityWait::new(Duration::from_secs(2)),
                    )
                }));
                let until = Instant::now() + Duration::from_millis(500);
                while f.store.try_acquire_authority(&[f.read.clone()]).is_ok() {
                    assert!(Instant::now() < until, "writer never queued");
                    std::thread::yield_now();
                }
                let second = crate::now();
                while crate::now() == second {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            assert!(Arc::ptr_eq(original_interval.as_ref().unwrap(), &held));
            assert_eq!(expiry, expiries.borrow()[0], "original expiry cannot renew");
            Ok((*facts, held, expiry))
        },
    );
    // Release the extra test reference even when the old helper refuses, so
    // the negative test cannot strand its independently queued writer.
    drop(original_interval);
    let (value, held, expiry) = result.unwrap();
    assert_eq!(value, 1);
    assert!(calls.get() >= 2 && calls.get() <= 3);
    assert!(expiries.borrow().iter().any(|later| *later > expiry));
    assert!(!writer.as_ref().unwrap().is_finished());
    drop(held);
    drop(writer.unwrap().join().unwrap().unwrap());
}

#[test]
fn compound_packet_projection_releases_every_reader_when_final_construction_crosses_a_commit() {
    for refused in [false, true] {
        let f = Fixture::new();
        let external = Connection::open(f.store.path()).unwrap();
        let result = f.store.try_current_packet_projection(
            |c| {
                f.derive(c)
                    .map(|(facts, scopes)| (facts, scopes, crate::now() + 5))
            },
            |_, facts, held, _expiry| {
                assert_eq!(*facts, 1);
                assert!(f.store.authority_covers(&held, &[f.read.clone()]));
                external
                    .execute("UPDATE packet_snapshot_test SET value=2", [])
                    .unwrap();
                if refused {
                    Err(Error::forbidden())
                } else {
                    Ok((*facts, held))
                }
            },
        );
        assert_eq!(result.err().unwrap().code, "authority-busy");
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
        let command_sql = f.store.connection().unwrap();
        let (value, held) = f
            .store
            .try_current_packet_projection(
                |c| {
                    f.derive(c)
                        .map(|(facts, scopes)| (facts, scopes, crate::now() + 5))
                },
                |_, value, held, _expiry| Ok((*value, held)),
            )
            .unwrap();
        assert_eq!(value, 2);
        assert!(f.store.try_acquire_authority(&[f.write.clone()]).is_err());
        drop(command_sql);
        drop(held);
        drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    }
}

#[test]
fn packet_snapshot_external_commit_refuses_stale_facts_releases_reader_and_recovers() {
    let f = Fixture::new();
    let external = Connection::open(f.store.path()).unwrap();
    let result = acquire(&f.store, |c| {
        let facts = f.derive(c)?;
        // A different process/Store can commit despite this Store's SQL mutex.
        // Model a completed mutation between fact resolution and file admission.
        external
            .execute("UPDATE packet_snapshot_test SET value=2", [])
            .unwrap();
        Ok(facts)
    });
    assert_eq!(result.unwrap_err().code, "authority-busy");
    // No stale reader survives the failed capture to obstruct a current writer.
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    let (value, held) = acquire(&f.store, |c| f.derive(c)).unwrap();
    assert_eq!(value, 2);
    assert!(f.store.try_connection().is_ok(), "SQL ends before IO");
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.write.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(held);
    assert!(f.store.try_acquire_authority(&[f.write]).is_ok());
}

#[test]
fn packet_snapshot_refusal_after_a_commit_is_unavailable_not_definite_retirement() {
    for borrowed in [false, true] {
        let f = Fixture::new();
        let external = Connection::open(f.store.path()).unwrap();
        let held = borrowed.then(|| f.store.try_acquire_authority(&[f.read.clone()]).unwrap());
        let derive = |c: &Connection| -> Result<(i64, Vec<AuthorityScope>)> {
            assert_eq!(f.derive(c)?.0, 1);
            // A multi-query policy resolver can observe preparation before a
            // commit and terminal execution afterwards. That mixed refusal is
            // not proof that the physical source lost current authority.
            external
                .execute("UPDATE packet_snapshot_test SET value=2", [])
                .unwrap();
            Err(Error::forbidden())
        };
        let result = if let Some(held) = &held {
            current_under(&f.store, held, derive)
        } else {
            acquire(&f.store, derive).map(|(value, _)| value)
        };
        assert_eq!(
            result.unwrap_err().code,
            "authority-busy",
            "borrowed={borrowed}"
        );
        drop(held);
        assert!(f.store.try_acquire_authority(&[f.write.clone()]).is_ok());
        assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 2);
        let refusal = acquire::<i64>(&f.store, |_| Err(Error::forbidden()));
        assert_eq!(refusal.unwrap_err().code, "forbidden");
    }
}

#[test]
fn packet_snapshot_primary_connection_write_also_invalidates_capture() {
    let f = Fixture::new();
    let result = acquire(&f.store, |c| {
        let facts = f.derive(c)?;
        f.store
            .connection()
            .unwrap()
            .execute("UPDATE packet_snapshot_test SET value=3", [])
            .unwrap();
        Ok(facts)
    });
    assert_eq!(result.unwrap_err().code, "authority-busy");
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 3);
}

#[test]
fn packet_snapshot_refuses_queued_writer_without_waiting_under_sql_and_recovers() {
    let f = Fixture::new();
    let held = f.store.try_acquire_authority(&[f.read.clone()]).unwrap();
    let store = f.store.clone();
    let write = f.write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(3)))
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match f.store.try_acquire_authority(&[f.read.clone()]) {
            Err(error) => {
                assert_eq!(error.code, "authority-busy");
                break;
            }
            Ok(read) => drop(read),
        }
        assert!(Instant::now() < deadline, "writer never queued");
        std::thread::yield_now();
    }
    let start = Instant::now();
    assert_eq!(
        acquire(&f.store, |c| f.derive(c)).unwrap_err().code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(held);
    let exclusive = writer.join().unwrap().unwrap();
    assert_eq!(
        acquire(&f.store, |c| f.derive(c)).unwrap_err().code,
        "authority-busy"
    );
    drop(exclusive);
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 1);
}

#[test]
fn packet_metadata_memo_reuses_only_unchanged_facts_with_independent_fresh_readers() {
    // A real policy-clock edge must invalidate a memo. Require the reuse
    // assertion only after observing the complete case within one second;
    // every attempt still verifies facts and fresh file-authority custody.
    for _ in 0..8 {
        let f = Fixture::new();
        let memo = Memo::default();
        let count = std::cell::Cell::new(0);
        let derive = |c: &Connection| {
            count.set(count.get() + 1);
            f.derive(c)
        };
        let policy_second = crate::now();
        let (one, first) = cached(&f.store, &memo, None, derive).unwrap();
        let (two, second) = cached(&f.store, &memo, None, derive).unwrap();
        assert_eq!((one, two), (1, 1));
        assert!(!Arc::ptr_eq(&first, &second), "a memo never lends a guard");
        drop(first);
        drop(second);
        let writer = f.store.try_acquire_authority(&[f.write.clone()]).unwrap();
        assert_eq!(
            cached(&f.store, &memo, None, derive).unwrap_err().code,
            "authority-busy",
            "even a cache hit must enter fresh authority"
        );
        drop(writer);
        assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 1);
        if crate::now() == policy_second {
            assert_eq!(
                count.get(),
                1,
                "unchanged facts are reused through fresh independent readers"
            );
            return;
        }
    }
    panic!("No unchanged policy-clock interval available for the memo reuse test");
}

#[test]
fn packet_metadata_memo_invalidates_external_own_clock_connection_and_source_changes() {
    // Each forced SQL/schema/source/clock change must derive exactly once.
    // A second real clock change is independently valid, so establish one
    // unchanged wall-clock interval for the exact invocation-count assertions.
    for _ in 0..8 {
        let f = Fixture::new();
        let memo = Memo::default();
        let source = new_metadata();
        let other_source = new_metadata();
        let count = std::cell::Cell::new(0);
        let derive = |c: &Connection| {
            count.set(count.get() + 1);
            f.derive(c)
        };
        let second = crate::now();
        assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 1);
        let external = Connection::open(f.store.path()).unwrap();
        external
            .execute("UPDATE packet_snapshot_test SET value=2", [])
            .unwrap();
        assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 2);
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 2);
        f.store
            .connection()
            .unwrap()
            .execute("UPDATE packet_snapshot_test SET value=3", [])
            .unwrap();
        assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 3);
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 3);
        // Controlled memo timestamp models crossing an integer policy-clock edge.
        for entry in memo.0.lock().unwrap().iter_mut() {
            entry.version.second -= 1;
        }
        assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 3);
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 4);
        assert_eq!(
            cached(&f.store, &memo, Some(&other_source), derive)
                .unwrap()
                .0,
            3
        );
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 5);
        assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 3);
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 6);
        f.store
            .connection()
            .unwrap()
            .execute_batch("ALTER TABLE packet_snapshot_test ADD COLUMN marker INTEGER")
            .unwrap();
        assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 3);
        if crate::now() != second {
            continue;
        }
        assert_eq!(
            count.get(),
            7,
            "own DDL changes schema_version, not total_changes"
        );
        // SQLite data_version/total_changes counters are connection-local; equal
        // counters from a different Connection can never validate this entry.
        assert_eq!(
            cached(
                &Store::open_administration(f.store.path(), connect_protocol::Limits::default())
                    .unwrap(),
                &memo,
                None,
                derive
            )
            .unwrap()
            .0,
            3
        );
        if crate::now() != second {
            continue;
        }
        assert_eq!(count.get(), 8);
        return;
    }
    panic!("No unchanged policy-clock interval available for the memo invalidation test");
}

#[test]
fn packet_metadata_memo_contention_and_failed_capture_keep_no_cached_grant() {
    let f = Fixture::new();
    let memo = Memo::default();
    let locked = memo.0.lock().unwrap();
    let start = Instant::now();
    drop(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap());
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(locked);
    assert!(
        memo.0.lock().unwrap().is_empty(),
        "contended memo is optional metadata"
    );
    let external = Connection::open(f.store.path()).unwrap();
    assert_eq!(
        cached(&f.store, &memo, None, |c| {
            let facts = f.derive(c)?;
            external
                .execute("UPDATE packet_snapshot_test SET value=9", [])
                .unwrap();
            Ok(facts)
        })
        .unwrap_err()
        .code,
        "authority-busy"
    );
    assert!(
        memo.0.lock().unwrap().is_empty(),
        "failed admission cannot populate memo"
    );
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    assert_eq!(cached(&f.store, &memo, None, |c| f.derive(c)).unwrap().0, 9);
}

#[test]
fn packet_snapshot_clock_boundary_rechecks_with_owned_reader_behind_queued_writer() {
    let f = Fixture::new();
    let mut calls = 0;
    let mut writer = None;
    let (value, held) = acquire_at(
        &f.store,
        |c, stamp| {
            calls += 1;
            if calls == 1 {
                // Actual policy-clock crossing, without changing the system clock.
                while crate::now() == stamp.second {
                    std::thread::sleep(Duration::from_millis(2));
                }
            } else {
                let store = f.store.clone();
                let scope = f.write.clone();
                writer = Some(std::thread::spawn(move || {
                    store.acquire_authority(
                        &[scope],
                        &crate::AuthorityWait::new(Duration::from_secs(2)),
                    )
                }));
                let deadline = Instant::now() + Duration::from_millis(500);
                loop {
                    if f.store.try_acquire_authority(&[f.read.clone()]).is_err() {
                        break;
                    }
                    assert!(Instant::now() < deadline, "writer never queued");
                    std::thread::yield_now();
                }
            }
            f.derive(c)
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!((value, calls), (1, 2));
    assert!(!writer.as_ref().unwrap().is_finished());
    drop(held);
    drop(writer.unwrap().join().unwrap().unwrap());
}

fn cached<T: Clone>(
    store: &Store,
    memo: &Memo<T>,
    source: Option<&MetadataToken>,
    derive: impl FnMut(&Connection) -> Result<(T, Vec<AuthorityScope>)>,
) -> Result<(T, Arc<AuthorityGuardSet>)> {
    super::cached(store, memo, source, derive, |_| Ok(()))
}

#[test]
fn packet_snapshot_releases_sql_during_actual_admission_and_refuses_reacquire_contention() {
    let f = Fixture::new();
    let mut occupied = None;
    let result = acquire_with(
        &f.store,
        |c, _| f.derive(c),
        |_| Ok(()),
        |scopes| {
            // This is the actual admission boundary after capture, not a sleep.
            // A competing Store read can own SQL while every original file scope is
            // freshly admitted. Failure to reacquire SQL must drop that whole guard.
            occupied = Some(occupy_packet_sql(&f.store));
            f.store.try_acquire_authority(scopes)
        },
    );
    assert_eq!(result.unwrap_err().code, "authority-busy");
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    drop(occupied);
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 1);
}

#[test]
fn packet_snapshot_releases_sql_for_current_mutation_and_rejects_mixed_capture() {
    let f = Fixture::new();
    let result = acquire_with(
        &f.store,
        |c, _| f.derive(c),
        |_| Ok(()),
        |scopes| {
            let c = f.store.try_connection().unwrap();
            c.execute("UPDATE packet_snapshot_test SET value=7", [])
                .unwrap();
            drop(c);
            f.store.try_acquire_authority(scopes)
        },
    );
    assert_eq!(result.unwrap_err().code, "authority-busy");
    drop(f.store.try_acquire_authority(&[f.write.clone()]).unwrap());
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 7);
}

fn occupy_packet_sql(store: &Store) -> Vec<crate::installation_ownership::ConnectionGuard<'_>> {
    let mut held = Vec::new();
    while let Ok(reader) = store.try_packet_connection() {
        held.push(reader);
        assert!(held.len() <= 4, "packet SQL observer capacity exceeded");
    }
    assert_eq!(
        held.len(),
        4,
        "all bounded observers must be available before saturation"
    );
    held
}

#[test]
fn packet_sql_observers_are_read_only_and_release_after_saturation() {
    let f = Fixture::new();
    let held = occupy_packet_sql(&f.store);
    assert_eq!(
        acquire(&f.store, |c| f.derive(c)).unwrap_err().code,
        "authority-busy"
    );
    assert!(held[0]
        .execute("UPDATE packet_snapshot_test SET value=9", [])
        .is_err());
    assert!(f.store.try_acquire_authority(&[f.write.clone()]).is_ok());
    drop(held);
    assert_eq!(acquire(&f.store, |c| f.derive(c)).unwrap().0, 1);
}
