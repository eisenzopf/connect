use super::*;
use crate::authority_fences::{AuthorityMode, AuthorityWait, Manager};
use std::sync::{atomic::AtomicUsize, mpsc};

struct Facts {
    owner: Arc<AtomicBool>,
    revision: u64,
}
impl SnapshotFacts for Facts {
    fn current(&self) -> bool {
        self.owner.load(Ordering::Acquire)
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    manager: Arc<Manager>,
    registry: Arc<Registry>,
    owner: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    refreshes: Arc<AtomicUsize>,
    scope: AuthorityScope,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("fences.sqlite3");
        std::fs::write(&database, b"snapshot fence test").unwrap();
        let manager = Manager::open(&database, "snapshot-fence-test").unwrap();
        let registry = manager.media_snapshot_registry().unwrap();
        Self {
            _dir: dir,
            manager,
            registry,
            owner: Arc::new(AtomicBool::new(true)),
            revision: Arc::new(AtomicU64::new(0)),
            refreshes: Arc::new(AtomicUsize::new(0)),
            scope: AuthorityScope::identity("snapshot-owner", AuthorityMode::Read).unwrap(),
        }
    }
    fn register(&self, until: Instant) -> Handle<Facts> {
        let manager = self.manager.clone();
        let owner = self.owner.clone();
        let revision = self.revision.clone();
        let refreshes = self.refreshes.clone();
        let scope = self.scope.clone();
        self.registry
            .register(move || {
                let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
                refreshes.fetch_add(1, Ordering::Relaxed);
                Prepared::new(
                    Arc::new(Facts {
                        owner: owner.clone(),
                        revision: revision.load(Ordering::Acquire),
                    }),
                    guards,
                    until,
                )
            })
            .unwrap()
    }
    fn start(&self) -> OwnedWatcher {
        let watcher = self
            .registry
            .start_watcher(Duration::from_millis(1))
            .unwrap();
        // These module tests explicitly model a verified writer boundary; the
        // production composition must prove that boundary before enabling.
        self.registry.enable_after_writer_boundary().unwrap();
        watcher
    }
}
fn until(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(1);
    while !predicate() {
        assert!(Instant::now() < end, "snapshot transition did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn admit(handle: &Handle<Facts>) -> Lease<Facts> {
    let mut admitted = None;
    until(|| {
        admitted = handle.admit().ok();
        admitted.is_some()
    });
    admitted.unwrap()
}

#[test]
fn publication_requires_boundary_worker_and_original_owner() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    assert!(f.registry.enable_after_writer_boundary().is_err());
    assert!(handle.admit().is_err());
    let mut watcher = f.registry.start_watcher(Duration::from_millis(1)).unwrap();
    assert!(handle.admit().is_err());
    f.registry.enable_after_writer_boundary().unwrap();
    let lease = admit(&handle);
    f.owner.store(false, Ordering::Release);
    assert!(!lease.current());
    assert!(handle.admit().is_err());
    watcher.shutdown().unwrap();
    assert!(watcher.thread.is_none());
    assert!(handle.admit().is_err());
}

#[test]
fn media_admission_does_not_enter_control_mutex_or_refresh_projection() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    let mut watcher = f.start();
    drop(admit(&handle));
    // Holding this actual control mutex blocks the watcher and every local
    // writer notification. Media delivery must still proceed without entering
    // that mutex or executing its projection closure.
    let held = f.registry.control.lock().unwrap();
    let before = f.refreshes.load(Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    let delivery = std::thread::spawn(move || {
        for _ in 0..1000 {
            let lease = handle.admit().unwrap();
            assert!(lease.current());
            assert_eq!(lease.facts().revision, 0);
        }
        tx.send(()).unwrap();
    });
    let completed_without_control = rx.recv_timeout(Duration::from_secs(1)).is_ok();
    drop(held);
    delivery.join().unwrap();
    watcher.shutdown().unwrap();
    assert!(completed_without_control, "media entered the control mutex");
    assert_eq!(f.refreshes.load(Ordering::Relaxed), before);
}

#[test]
fn local_writer_revokes_before_wait_and_drains_actual_io_without_renewing_queued_stamp() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    let mut watcher = f.start();
    let lease = admit(&handle);
    let queued = lease.stamp();
    let manager = f.manager.clone();
    let revision = f.revision.clone();
    let write = AuthorityScope::identity("snapshot-owner", AuthorityMode::Write).unwrap();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let guards = manager
            .acquire(&[write], &AuthorityWait::new(Duration::from_secs(2)))
            .unwrap();
        revision.store(1, Ordering::Release);
        acquired_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(guards);
    });
    until(|| !lease.current());
    let refused_new = handle.admit().is_err();
    let old_io_still_fences_writer = acquired_rx.try_recv().is_err();
    let unrelated = AuthorityScope::identity("other-owner", AuthorityMode::Read).unwrap();
    drop(f.manager.try_acquire(&[unrelated]).unwrap());
    drop(lease);
    acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let refused_during_writer = handle.admit().is_err();
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    let refreshed = admit(&handle);
    assert_eq!(refreshed.facts().revision, 1);
    assert_ne!(refreshed.stamp().generation, queued.generation);
    assert!(handle.admit_original(queued).is_err());
    assert!(handle.admit_original(refreshed.stamp()).is_ok());
    watcher.shutdown().unwrap();
    assert!(!refreshed.current());
    assert!(refused_new && old_io_still_fences_writer && refused_during_writer);
}

#[test]
fn monotonic_original_expiry_refuses_even_before_watcher_reaps_publication() {
    let f = Fixture::new();
    let deadline = Instant::now() + Duration::from_millis(100);
    let handle = f.register(deadline);
    let mut watcher = f.start();
    let lease = admit(&handle);
    // Prevent background removal, so this checks the media-side original
    // deadline rather than a later worker refresh or an invented new budget.
    let held = f.registry.control.lock().unwrap();
    until(|| Instant::now() >= deadline);
    let expired = !lease.current() && handle.admit().is_err();
    drop(held);
    watcher.shutdown().unwrap();
    assert!(expired);
}

#[test]
fn queued_epoch_owns_no_fence_and_last_handle_withdrawal_revokes_it() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    let mut watcher = f.start();
    let lease = admit(&handle);
    let queued = lease.packet_epoch();
    assert!(queued.current());
    drop(lease);
    drop(handle);
    let revoked = !queued.current();
    let write = AuthorityScope::identity("snapshot-owner", AuthorityMode::Write).unwrap();
    let result = f
        .manager
        .acquire(&[write], &AuthorityWait::new(Duration::from_secs(1)));
    let queued_metadata_does_not_pin_fence = result.is_ok();
    drop(result);
    watcher.shutdown().unwrap();
    assert!(revoked && queued_metadata_does_not_pin_fence);
}

#[test]
fn unchanged_definitive_denial_waits_for_next_write_outcome_instead_of_polling_sql() {
    let f = Fixture::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let handle: Handle<Facts> = f
        .registry
        .register(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            Err(Error::forbidden())
        })
        .unwrap();
    let mut watcher = f.start();
    until(|| attempts.load(Ordering::Relaxed) == 1);
    std::thread::sleep(Duration::from_millis(20));
    let unchanged_attempts = attempts.load(Ordering::Relaxed);
    let write = AuthorityScope::identity("snapshot-owner", AuthorityMode::Write).unwrap();
    let held = f.manager.try_acquire(&[write]).unwrap();
    drop(held);
    until(|| attempts.load(Ordering::Relaxed) == 2);
    std::thread::sleep(Duration::from_millis(20));
    let changed_attempts = attempts.load(Ordering::Relaxed);
    watcher.shutdown().unwrap();
    assert!(handle.admit().is_err());
    assert_eq!(unchanged_attempts, 1);
    assert_eq!(changed_attempts, 2);
}

#[test]
fn route_miss_registration_refuses_control_contention_without_running_factory() {
    let f = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let held = f.registry.control.lock().unwrap();
    let refused: Result<Handle<Facts>> =
        f.registry
            .try_register_with_hints(std::slice::from_ref(&f.scope), move || {
                callback_calls.fetch_add(1, Ordering::Relaxed);
                Err(Error::forbidden())
            });
    drop(held);
    assert_eq!(refused.err().unwrap().code, "authority-busy");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn lifetime_observation_distinguishes_refresh_from_refusal_and_never_revives_original_epoch() {
    let f = Fixture::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let selected = mode.clone();
    let manager = f.manager.clone();
    let scope = f.scope.clone();
    let owner = f.owner.clone();
    let handle = f
        .registry
        .register_with_hints(std::slice::from_ref(&f.scope), move || {
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            match selected.load(Ordering::Acquire) {
                1 => {
                    return Err(Error::new(
                        "authority-busy",
                        "same original refresh is pending",
                    ))
                }
                2 => {
                    return Err(Error::new(
                        "handoff-pending",
                        "same original projection is pending",
                    ))
                }
                3 => return Err(Error::forbidden()),
                _ => {}
            }
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: selected.load(Ordering::Acquire) as u64,
                }),
                guards,
                Instant::now() + Duration::from_secs(5),
            )
        })
        .unwrap();
    let mut watcher = f.start();
    let original = admit(&handle);
    let queued = original.stamp();
    let epoch = original.packet_epoch();
    let manager = f.manager.clone();
    let selected = mode.clone();
    let write = f.scope.write_scope();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let held = manager
            .acquire(&[write], &AuthorityWait::new(Duration::from_secs(2)))
            .unwrap();
        selected.store(1, Ordering::Release);
        acquired_tx.send(()).unwrap();
        drop(held);
    });
    until(|| !epoch.current());
    let pending = handle.observe().err().unwrap().code;
    let original_io_drained_first = acquired_rx.try_recv().is_err();
    drop(original);
    acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    writer.join().unwrap();
    until(|| mode.load(Ordering::Acquire) == 1 && handle.observe().is_err());
    let transient = handle.observe().err().unwrap().code;
    let mut phases = Vec::new();
    for (next, expected) in [(2, "handoff-pending"), (3, "forbidden")] {
        let held = f
            .manager
            .acquire(
                &[f.scope.write_scope()],
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        mode.store(next, Ordering::Release);
        drop(held);
        until(|| handle.observe().is_err_and(|error| error.code == expected));
        phases.push(handle.observe().err().unwrap().code);
    }
    let held = f
        .manager
        .acquire(
            &[f.scope.write_scope()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    mode.store(4, Ordering::Release);
    drop(held);
    let restored = admit(&handle);
    let restored_current = handle.observe().is_ok();
    let old_never_revived = !epoch.current() && handle.admit_original(queued).is_err();
    drop(restored);
    watcher.shutdown().unwrap();
    assert!(original_io_drained_first);
    assert_eq!(pending, "authority-busy");
    assert_eq!(transient, "authority-busy");
    assert_eq!(phases, ["handoff-pending", "forbidden"]);
    assert!(restored_current && old_never_revived);
}

#[test]
fn original_failed_lifetime_refresh_cannot_publish_across_a_racing_writer_sequence() {
    let f = Fixture::new();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let manager = f.manager.clone();
    let scope = f.scope.clone();
    let owner = f.owner.clone();
    let handle = f
        .registry
        .register_with_hints(std::slice::from_ref(&f.scope), move || {
            if first.swap(false, Ordering::AcqRel) {
                entered_tx.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                return Err(Error::forbidden());
            }
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 1,
                }),
                guards,
                Instant::now() + Duration::from_secs(5),
            )
        })
        .unwrap();
    // Drive the exact production refresh method once, so its return/join proves
    // the stale refusal was processed before inspecting the observed status.
    // Module-local publication model: no extra watcher is manufactured. The
    // owned thread below drives the same refresh function as the Runtime worker.
    f.registry.enabled.store(true, Ordering::Release);
    let slot = handle.slot.clone();
    let registry = f.registry.clone();
    let worker = std::thread::spawn(move || slot.refresh_if_missing(&registry));
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let writer = f
        .manager
        .acquire(
            &[f.scope.write_scope()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    let raced_status = handle.observe().err().unwrap().code;
    drop(writer);
    handle.slot.refresh_if_missing(&f.registry);
    let new_current = handle.observe().is_ok();
    f.registry.enabled.store(false, Ordering::Release);
    f.registry.invalidate_all();
    assert_eq!(
        raced_status, "authority-busy",
        "old failed derivation must not become a definitive refusal after writer invalidation"
    );
    assert!(new_current);
}
