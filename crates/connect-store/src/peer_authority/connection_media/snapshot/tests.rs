//! Controlled SQL races test capture invalidation, not physical media delivery.
use super::*;
use std::time::{Duration, Instant};
mod hints;
mod owned;

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
fn packet_snapshot_own_connection_write_also_invalidates_capture() {
    let f = Fixture::new();
    let result = acquire(&f.store, |c| {
        let facts = f.derive(c)?;
        c.execute("UPDATE packet_snapshot_test SET value=3", [])
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
            occupied = Some(f.store.try_connection().unwrap());
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
