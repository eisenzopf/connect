use super::*;
use std::sync::Barrier;
struct Fixture {
    pool: Observers,
    path: PathBuf,
    _writer: Connection,
    _dir: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite3");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE marker(value); INSERT INTO marker VALUES(7)",
            )
            .unwrap();
        Self {
            pool: Observers::new(&path).unwrap(),
            path,
            _writer: writer,
            _dir: dir,
        }
    }
}
fn value(c: &Connection) -> i64 {
    c.query_row("SELECT value FROM marker", [], |r| r.get(0))
        .unwrap()
}
#[cfg(target_os = "linux")]
fn descriptors(path: &Path) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|v| std::fs::read_link(v.unwrap().path()).ok())
        .filter(|v| v == path)
        .count()
}
#[test]
fn two_original_scopes_reuse_four_slots_and_last_drop_only_queues_close() {
    let f = Fixture::new();
    let first = f.pool.scope().unwrap();
    let second = first.clone();
    let independent = f.pool.scope().unwrap();
    let read = f.pool.observe().unwrap();
    let mut guards = Vec::new();
    for _ in 0..OBSERVERS {
        let c = f.pool.connection(&read, None).unwrap();
        assert_eq!(value(&c), 7);
        guards.push(c);
    }
    assert!(matches!(f.pool.connection(&read,None),Err(e) if e.code=="authority-busy"));
    let ids: Vec<_> = guards.iter().map(Guard::id).collect();
    drop(guards);
    drop(read);
    drop(first);
    drop(second);
    let read = f.pool.observe().unwrap();
    for id in ids {
        assert_eq!(value(&f.pool.connection(&read, Some(id)).unwrap()), 7);
    }
    drop(read);
    drop(independent);
    assert!(matches!(
        f.pool.0.ownership.state.lock().unwrap().phase,
        Phase::Retiring { closing: false, .. }
    ));
    assert!(f.pool.reap().unwrap());
    assert!(matches!(
        f.pool.0.ownership.state.lock().unwrap().phase,
        Phase::Idle
    ));
}
#[test]
fn original_observation_survives_last_scope_drop_and_only_exact_reseal() {
    let f = Fixture::new();
    let physical = f.pool.scope().unwrap();
    let read = f.pool.observe().unwrap();
    let c = f.pool.connection(&read, None).unwrap();
    let id = c.id();
    assert_eq!(value(&c), 7);
    drop(c);
    drop(physical);
    assert!(!f.pool.reap().unwrap());
    assert!(f.pool.observe().is_err());
    assert_eq!(f.pool.connection(&read, Some(id)).unwrap().id(), id);
    let foreign = Fixture::new();
    let other_scope = foreign.pool.scope().unwrap();
    let other = foreign.pool.observe().unwrap();
    assert!(f.pool.connection(&other, Some(id)).is_err());
    drop(other);
    drop(other_scope);
    drop(read);
    assert!(f.pool.reap().unwrap());
}
#[test]
fn last_original_observation_fences_new_physical_generation_until_reseal() {
    let f = Fixture::new();
    let old = f.pool.scope().unwrap();
    let original = f.pool.observe().unwrap();
    let id = f.pool.connection(&original, None).unwrap().id();
    drop(old);
    assert!(matches!(f.pool.scope(), Err(e) if e.code == "authority-busy"));
    assert_eq!(f.pool.connection(&original, Some(id)).unwrap().id(), id);
    drop(original);
    let next = f.pool.scope().unwrap();
    let read = f.pool.observe().unwrap();
    assert_ne!(f.pool.connection(&read, None).unwrap().id(), id);
    assert!(f.pool.connection(&read, Some(id)).is_err());
    drop(read);
    drop(next);
    assert!(f.pool.reap().unwrap());
}
#[test]
fn closing_original_batch_fences_new_scope_without_holding_pool_or_slot_guards() {
    let f = Fixture::new();
    let old = f.pool.scope().unwrap();
    drop(old);
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let (a, b) = (entered.clone(), resume.clone());
    let pool = f.pool.clone();
    *f.pool.0.before_close.lock().unwrap() = Some(Box::new(move || {
        drop(
            pool.0
                .ownership
                .state
                .try_lock()
                .expect("no pool gate across close"),
        );
        for slot in &pool.0.slots {
            drop(slot.try_lock().expect("no SQL slot across close"));
        }
        a.wait();
        b.wait();
    }));
    let pool = f.pool.clone();
    let worker = std::thread::spawn(move || pool.reap());
    entered.wait();
    assert!(matches!(f.pool.scope(),Err(e) if e.code=="authority-busy"));
    assert!(f.pool.observe().is_err());
    resume.wait();
    assert!(worker.join().unwrap().unwrap());
    let fresh = f.pool.scope().unwrap();
    drop(fresh);
    assert!(f.pool.reap().unwrap());
}
#[test]
fn cold_open_has_no_pool_or_slot_guard_and_failure_closes_partial_readers() {
    let f = Fixture::new();
    let pool = f.pool.clone();
    *f.pool.0.before_open.lock().unwrap() = Some(Box::new(move || {
        drop(
            pool.0
                .ownership
                .state
                .try_lock()
                .expect("cold open outside pool gate"),
        );
        for slot in &pool.0.slots {
            drop(slot.try_lock().expect("cold open outside SQL slot"));
        }
        assert!(pool.observe().is_err());
    }));
    let scope = f.pool.scope().unwrap();
    drop(scope);
    assert!(f.pool.reap().unwrap());
    #[cfg(target_os = "linux")]
    let before = descriptors(&f.path);
    let mut actual_opened = 0;
    let failed = open_each(&f.path, |index| {
        if index == 2 {
            return Err(Error::storage());
        }
        actual_opened += 1;
        Ok(())
    });
    assert!(failed.is_err());
    assert_eq!(actual_opened, 2);
    #[cfg(target_os = "linux")]
    assert_eq!(
        descriptors(&f.path),
        before,
        "partial real SQLite opens must close"
    );
}
#[test]
fn generation_overflow_refuses_without_reopening_or_reusing_stamp() {
    let f = Fixture::new();
    f.pool.0.ownership.state.lock().unwrap().next_generation = u64::MAX;
    assert!(f.pool.scope().is_err());
    assert!(f.pool.observe().is_err());
    assert!(f.pool.0.slots.iter().all(|v| v.lock().unwrap().0.is_none()));
}
#[test]
fn last_resource_free_lease_cannot_retain_or_close_store_connections() {
    let f = Fixture::new();
    let scope = f.pool.scope().unwrap();
    let read = f.pool.observe().unwrap();
    assert_eq!(value(&f.pool.connection(&read, None).unwrap()), 7);
    let resources = Arc::downgrade(&f.pool.0);
    let ownership = Arc::downgrade(&f.pool.0.ownership);
    drop(f.pool);
    assert!(
        resources.upgrade().is_none(),
        "leases retain counters, not SQLite resources"
    );
    assert!(ownership.upgrade().is_some());
    // Both destructors have only resource-free ownership, even after the Store
    // owner is gone. They cannot become a last SQLite owner inside a caller lock.
    drop(scope);
    drop(read);
    assert!(ownership.upgrade().is_none());
}

fn actual_store() -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap());
    (dir, store)
}

#[test]
fn bounded_concurrent_admissions_share_the_original_cold_open_without_capacity_refusal() {
    use std::sync::atomic::AtomicUsize;
    let (_dir, store) = actual_store();
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let (a, b) = (entered.clone(), resume.clone());
    *store.packet_connections.0.before_open.lock().unwrap() = Some(Box::new(move || {
        a.wait();
        b.wait();
    }));
    let cutoff = std::time::Instant::now() + Duration::from_secs(2);
    let first_store = store.clone();
    let first =
        std::thread::spawn(move || first_store.retain_packet_observers_until(cutoff, || false));
    entered.wait();
    let polls = Arc::new(AtomicUsize::new(0));
    let seen = polls.clone();
    let second_store = store.clone();
    let second = std::thread::spawn(move || {
        second_store.retain_packet_observers_until(cutoff, || {
            seen.fetch_add(1, Ordering::SeqCst);
            false
        })
    });
    while polls.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < cutoff {
        std::thread::yield_now();
    }
    let actually_waited = polls.load(Ordering::SeqCst) >= 3 && !second.is_finished();
    resume.wait();
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert!(
        actually_waited,
        "second real admission did not reach the cold-open contention"
    );
    assert!(
        first.is_ok() && second.is_ok(),
        "normal concurrent admission became a capacity refusal"
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first._scope.generation, second._scope.generation);
    drop(first);
    assert!(store.packet_connections.observe().is_ok());
    drop(second);
    assert!(store.reap_idle_packet_observers().unwrap());
}

#[test]
fn bounded_admission_preserves_original_deadline_and_cancel_without_ending_another_opener() {
    let (_dir, store) = actual_store();
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let (a, b) = (entered.clone(), resume.clone());
    *store.packet_connections.0.before_open.lock().unwrap() = Some(Box::new(move || {
        a.wait();
        b.wait();
    }));
    let first_store = store.clone();
    let first = std::thread::spawn(move || {
        first_store.retain_packet_observers_until(
            std::time::Instant::now() + Duration::from_secs(2),
            || false,
        )
    });
    entered.wait();
    let cutoff = std::time::Instant::now() + Duration::from_millis(25);
    let expired = store.retain_packet_observers_until(cutoff, || false);
    let checks = std::cell::Cell::new(0);
    let cancelled = store.retain_packet_observers_until(
        std::time::Instant::now() + Duration::from_secs(1),
        || {
            checks.set(checks.get() + 1);
            checks.get() > 1
        },
    );
    let original_unaffected = !first.is_finished();
    resume.wait();
    let original = first.join().unwrap();
    assert!(matches!(expired, Err(e) if e.code == "cancelled"));
    assert!(matches!(cancelled, Err(e) if e.code == "cancelled"));
    assert!(
        original_unaffected && original.is_ok(),
        "another admission was cancelled or adopted"
    );
    drop(original);
    assert!(store.reap_idle_packet_observers().unwrap());
}

#[test]
fn bounded_admission_never_retries_poisoned_resource_ownership_as_contention() {
    let (_dir, store) = actual_store();
    let original = store.clone();
    let poison = std::thread::spawn(move || {
        let _held = original
            .packet_connections
            .0
            .ownership
            .state
            .lock()
            .unwrap();
        panic!("controlled original ownership poison");
    });
    assert!(poison.join().is_err());
    let checks = std::cell::Cell::new(0);
    let result = store.retain_packet_observers_until(
        std::time::Instant::now() + Duration::from_secs(1),
        || {
            checks.set(checks.get() + 1);
            false
        },
    );
    assert!(matches!(result, Err(e) if e.code == Error::storage().code));
    assert_eq!(checks.get(), 1, "permanent ownership failure was retried");
}
#[cfg(target_os = "linux")]
#[test]
fn actual_wal_handles_close_only_after_original_query_lease_and_custodian() {
    let f = Fixture::new();
    let wal = PathBuf::from(format!("{}-wal", f.path.display()));
    let before = descriptors(&wal);
    let scope = f.pool.scope().unwrap();
    let read = f.pool.observe().unwrap();
    let mut guards = Vec::new();
    for _ in 0..OBSERVERS {
        let c = f.pool.connection(&read, None).unwrap();
        assert_eq!(value(&c), 7);
        guards.push(c);
    }
    assert_eq!(descriptors(&wal), before + OBSERVERS);
    drop(scope);
    drop(guards);
    assert!(!f.pool.reap().unwrap());
    assert_eq!(descriptors(&wal), before + OBSERVERS);
    drop(read);
    assert_eq!(
        descriptors(&wal),
        before + OBSERVERS,
        "Drop queues; it cannot close under caller authority"
    );
    assert!(f.pool.reap().unwrap());
    assert_eq!(descriptors(&wal), before);
    assert_eq!(
        value(&f._writer),
        7,
        "the original primary connection remains live"
    );
}
