//! Controlled SQL races test capture invalidation, not physical media delivery.
use super::*;
use std::time::{Duration, Instant};
mod hints;
mod owned;

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
    let f = Fixture::new();
    let memo = Memo::default();
    let count = std::cell::Cell::new(0);
    let derive = |c: &Connection| {
        count.set(count.get() + 1);
        f.derive(c)
    };
    let (one, first) = cached(&f.store, &memo, None, derive).unwrap();
    let (two, second) = cached(&f.store, &memo, None, derive).unwrap();
    assert_eq!((one, two, count.get()), (1, 1, 1));
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
    assert_eq!(count.get(), 1);
}

#[test]
fn packet_metadata_memo_invalidates_external_own_clock_connection_and_source_changes() {
    let f = Fixture::new();
    let memo = Memo::default();
    let source = new_metadata();
    let other_source = new_metadata();
    let count = std::cell::Cell::new(0);
    let derive = |c: &Connection| {
        count.set(count.get() + 1);
        f.derive(c)
    };
    assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 1);
    let external = Connection::open(f.store.path()).unwrap();
    external
        .execute("UPDATE packet_snapshot_test SET value=2", [])
        .unwrap();
    assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 2);
    assert_eq!(count.get(), 2);
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE packet_snapshot_test SET value=3", [])
        .unwrap();
    assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 3);
    assert_eq!(count.get(), 3);
    // Controlled memo timestamp models crossing an integer policy-clock edge.
    memo.0.lock().unwrap().as_mut().unwrap().version.second -= 1;
    assert_eq!(cached(&f.store, &memo, Some(&source), derive).unwrap().0, 3);
    assert_eq!(count.get(), 4);
    assert_eq!(
        cached(&f.store, &memo, Some(&other_source), derive)
            .unwrap()
            .0,
        3
    );
    assert_eq!(count.get(), 5);
    assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 3);
    assert_eq!(count.get(), 6);
    f.store
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE packet_snapshot_test ADD COLUMN marker INTEGER")
        .unwrap();
    assert_eq!(cached(&f.store, &memo, None, derive).unwrap().0, 3);
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
    assert_eq!(count.get(), 8);
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
        memo.0.lock().unwrap().is_none(),
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
        memo.0.lock().unwrap().is_none(),
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
