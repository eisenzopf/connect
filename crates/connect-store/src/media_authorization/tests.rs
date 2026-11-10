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
        self.register_for(&self.scope, until)
    }
    fn register_for(&self, scope: &AuthorityScope, until: Instant) -> Handle<Facts> {
        self.register_hinted_for(scope, &[], until)
    }
    fn register_hinted_for(
        &self,
        scope: &AuthorityScope,
        hints: &[AuthorityScope],
        until: Instant,
    ) -> Handle<Facts> {
        let manager = self.manager.clone();
        let owner = self.owner.clone();
        let revision = self.revision.clone();
        let refreshes = self.refreshes.clone();
        let scope = scope.clone();
        self.registry
            .register_with_hints(hints, move || {
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
    f.registry.availability.enable().unwrap();
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
    f.registry.availability.disable();
    f.registry.invalidate_all();
    assert_eq!(
        raced_status, "authority-busy",
        "old failed derivation must not become a definitive refusal after writer invalidation"
    );
    assert!(new_current);
}

#[test]
fn packet_availability_drops_without_pinning_registry_and_never_resurrects() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(5));
    // Drive the same production publication method under the module's explicit
    // verified-boundary model, without manufacturing another watcher.
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let lease = handle.admit().unwrap();
    let epoch = lease.packet_epoch();
    let weak_registry = Arc::downgrade(&f.registry);
    drop(f.registry);
    let registry_not_pinned = weak_registry.upgrade().is_none();
    let refused = handle.admit().err().unwrap().code;
    let invalidated = !lease.current() && !epoch.current();
    // Actual in-flight IO still drains its original Read before a writer. A
    // queued epoch has no fence of its own and cannot revive a new Registry.
    let before_drain = f.manager.try_acquire(&[f.scope.write_scope()]).is_err();
    drop(lease);
    let after_drain = f.manager.try_acquire(&[f.scope.write_scope()]).is_ok();
    let successor = f.manager.media_snapshot_registry().unwrap();
    successor.availability.enable().unwrap();
    let old_stays_invalid = !epoch.current() && handle.admit().is_err();
    successor.invalidate_all();
    assert!(registry_not_pinned && invalidated && before_drain && after_drain && old_stays_invalid);
    assert_eq!(refused, "forbidden");
}
#[test]
fn scope_probe_has_one_healthy_call_and_deduplicated_busy_fallback() {
    let a = AuthorityScope::identity("first", AuthorityMode::Read).unwrap();
    let b = AuthorityScope::identity("second", AuthorityMode::Read).unwrap();
    let selected = b.clone();
    let busy_scope = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let busy = busy_scope.clone();
    let registry = Registry::new(move |scopes| {
        seen.lock().unwrap().push(scopes.to_vec());
        if busy.load(Ordering::Acquire) && scopes.contains(&selected) {
            Err(super::busy())
        } else {
            Ok(())
        }
    });
    let input = [a.clone(), b.clone(), a.clone()];
    assert!(registry.blocked_external_scopes(&input).unwrap().is_empty());
    assert_eq!(calls.lock().unwrap().len(), 1);
    calls.lock().unwrap().clear();
    busy_scope.store(true, Ordering::Release);
    assert!(registry.blocked_external_scopes(&input).unwrap() == vec![b]);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3); // One aggregate + exactly two unique scopes.
    assert_eq!(calls[0].len(), 2);
    assert!(calls[1..].iter().all(|scopes| scopes.len() == 1));
}
#[test]
fn hard_external_probe_failure_still_revokes_every_scope_and_old_epoch() {
    let mut f = Fixture::new();
    let fail = Arc::new(AtomicBool::new(false));
    let flag = fail.clone();
    let manager = f.manager.clone();
    f.registry = Registry::new(move |scopes| {
        if flag.load(Ordering::Acquire) {
            return Err(Error::storage());
        }
        drop(manager.try_acquire(scopes)?);
        Ok(())
    });
    let first = f.register(Instant::now() + Duration::from_secs(5));
    let other = AuthorityScope::identity("unrelated", AuthorityMode::Read).unwrap();
    let second = f.register_for(&other, Instant::now() + Duration::from_secs(5));
    let mut watcher = f.start();
    let a = admit(&first);
    let b = admit(&second);
    let old_a = a.stamp();
    let old_b = b.stamp();
    fail.store(true, Ordering::Release);
    until(|| !a.current() && !b.current());
    let both_refused = first.admit().is_err() && second.admit().is_err();
    drop(a);
    drop(b);
    fail.store(false, Ordering::Release);
    let restored_a = admit(&first);
    let restored_b = admit(&second);
    let old_not_reused =
        first.admit_original(old_a).is_err() && second.admit_original(old_b).is_err();
    watcher.shutdown().unwrap();
    assert!(both_refused && old_not_reused);
    assert!(!restored_a.current() && !restored_b.current());
}

#[test]
fn immutable_scope_probe_names_reuse_the_published_vector_without_holding_a_fence() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(5));
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let first = handle.slot.published_scopes().unwrap();
    let second = handle.slot.published_scopes().unwrap();
    let unchanged_names_reused = Arc::ptr_eq(&first, &second);
    drop(handle);
    // Retaining these off-media names is not a retained IO/authority read.
    let writer = f.manager.try_acquire(&[f.scope.write_scope()]);
    let names_hold_no_fence = writer.is_ok();
    drop(writer);
    f.registry.invalidate_all();
    assert!(unchanged_names_reused && names_hold_no_fence);
}

#[test]
fn missing_publication_ignores_disjoint_cid_writer_but_matching_writer_still_drains_io() {
    for hinted in [false, true] {
        let f = Fixture::new();
        f.registry.availability.enable().unwrap();
        let a = AuthorityScope::conversation("unrelated-room", AuthorityMode::Read).unwrap();
        let b = AuthorityScope::conversation("original-room", AuthorityMode::Read).unwrap();
        let writer_a = f.manager.try_acquire(&[a.write_scope()]).unwrap();
        let hints = if hinted { vec![b.clone()] } else { Vec::new() };
        let handle = f.register_hinted_for(&b, &hints, Instant::now() + Duration::from_secs(5));
        handle.slot.refresh_if_missing(&f.registry);
        let original = handle.admit();
        drop(writer_a);
        let original = original.expect("unrelated CID writer must not pause a missing route");
        let queued = original.stamp();
        assert!(original.current());
        assert_eq!(f.refreshes.load(Ordering::Acquire), 1);
        // A matching state change still revokes before waiting, and the actual
        // original IO Read must drain before its file Writer can succeed.
        let blocked = f.manager.try_acquire(&[b.write_scope()]);
        let waiting_for_original_io = blocked.is_err();
        drop(blocked);
        assert!(!original.current() && handle.admit().is_err());
        drop(original);
        let writer_b = f.manager.try_acquire(&[b.write_scope()]).unwrap();
        let before = f.refreshes.load(Ordering::Acquire);
        handle.slot.refresh_if_missing(&f.registry);
        let matching_still_refused = handle.admit().is_err();
        let matching_not_rederived = f.refreshes.load(Ordering::Acquire) == before;
        drop(writer_b);
        handle.slot.refresh_if_missing(&f.registry);
        let renewed = handle.admit().unwrap();
        let old_stamp_refused = handle.admit_original(queued).is_err();
        f.registry.invalidate_all();
        assert!(waiting_for_original_io && matching_still_refused && matching_not_rederived);
        assert!(old_stamp_refused && !renewed.current());
    }
}

#[test]
fn successful_fenced_refresh_publishes_across_disjoint_writer_sequence_change() {
    let f = Fixture::new();
    let scope = AuthorityScope::conversation("prepared-room", AuthorityMode::Read).unwrap();
    let other = AuthorityScope::conversation("changed-room", AuthorityMode::Write).unwrap();
    let manager = f.manager.clone();
    let owner = f.owner.clone();
    let captured_scope = scope.clone();
    let (prepared_tx, prepared_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Mutex::new(release_rx);
    let handle = f
        .registry
        .register_with_hints(&[scope], move || {
            let guards = manager.try_acquire(std::slice::from_ref(&captured_scope))?;
            let prepared = Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 7,
                }),
                guards,
                Instant::now() + Duration::from_secs(5),
            )?;
            prepared_tx.send(()).unwrap();
            release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            Ok(prepared)
        })
        .unwrap();
    f.registry.availability.enable().unwrap();
    let slot = handle.slot.clone();
    let registry = f.registry.clone();
    let worker = std::thread::spawn(move || slot.refresh_if_missing(&registry));
    let prepared = prepared_rx.recv_timeout(Duration::from_secs(1));
    let before = f.registry.control.lock().unwrap().sequence;
    let writer = f.manager.try_acquire(&[other]).unwrap();
    drop(writer);
    let after = f.registry.control.lock().unwrap().sequence;
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    let published = handle.admit();
    f.registry.invalidate_all();
    assert!(prepared.is_ok() && after > before);
    assert_eq!(published.unwrap().facts().revision, 7);
}

#[test]
fn discovered_dependency_writer_prevents_publication_and_drains_prepared_read() {
    let f = Fixture::new();
    let old_hint = AuthorityScope::conversation("previous-room", AuthorityMode::Read).unwrap();
    let dependency = AuthorityScope::conversation("discovered-room", AuthorityMode::Read).unwrap();
    let manager = f.manager.clone();
    let owner = f.owner.clone();
    let actual = dependency.clone();
    let first = AtomicBool::new(true);
    let (prepared_tx, prepared_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Mutex::new(release_rx);
    let handle = f
        .registry
        .register_with_hints(&[old_hint], move || {
            let guards = manager.try_acquire(std::slice::from_ref(&actual))?;
            let prepared = Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 1,
                }),
                guards,
                Instant::now() + Duration::from_secs(5),
            )?;
            if first.swap(false, Ordering::AcqRel) {
                prepared_tx.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
            }
            Ok(prepared)
        })
        .unwrap();
    f.registry.availability.enable().unwrap();
    let slot = handle.slot.clone();
    let registry = f.registry.clone();
    let refresh = std::thread::spawn(move || slot.refresh_if_missing(&registry));
    let prepared = prepared_rx.recv_timeout(Duration::from_secs(1));
    let (blocked_tx, blocked_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let manager = f.manager.clone();
    let write = dependency.write_scope();
    let writer = std::thread::spawn(move || {
        let guards = crate::authority_fences::with_waited_lock_probe(
            move || {
                let _ = blocked_tx.send(());
            },
            || manager.acquire(&[write], &AuthorityWait::new(Duration::from_secs(2))),
        );
        acquired_tx.send(guards.is_ok()).unwrap();
        let _ = finish_rx.recv_timeout(Duration::from_secs(2));
        drop(guards);
    });
    let waited_for_exact_read = blocked_rx.recv_timeout(Duration::from_secs(1));
    let still_waiting = acquired_rx.try_recv().is_err();
    release_tx.send(()).unwrap();
    refresh.join().unwrap();
    let refused = handle.admit().is_err();
    let acquired_after_read_drop = acquired_rx.recv_timeout(Duration::from_secs(1));
    // Even an unsafe counterfactual publication must release its original
    // Read before joining the same actual writer; retain the failed criteria.
    handle.slot.revoke();
    let _ = finish_tx.send(());
    writer.join().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let current = handle.admit().is_ok();
    let exact_scope_published = handle
        .slot
        .published_scopes()
        .is_some_and(|scopes| scopes.as_slice() == std::slice::from_ref(&dependency));
    f.registry.invalidate_all();
    assert!(prepared.is_ok() && waited_for_exact_read.is_ok() && still_waiting);
    assert!(refused && matches!(acquired_after_read_drop, Ok(true)));
    assert!(current && exact_scope_published);
}

#[test]
fn external_scope_intent_allows_disjoint_missing_publication_without_local_notice() {
    let mut f = Fixture::new();
    let manager = f.manager.clone();
    // An independent registry receives no Manager before_write notification;
    // only its real kernel scope probe can observe this writer's exclusion.
    f.registry = Registry::new(move |scopes| {
        drop(manager.try_acquire(scopes)?);
        Ok(())
    });
    let denial_hint =
        AuthorityScope::conversation("unknown-denial-room", AuthorityMode::Read).unwrap();
    let denied: Handle<Facts> = f
        .registry
        .register_with_hints(&[denial_hint], || Err(Error::forbidden()))
        .unwrap();
    f.registry.availability.enable().unwrap();
    denied.slot.refresh_if_missing(&f.registry);
    let original_denial = denied
        .observe()
        .err()
        .is_some_and(|error| error.code == "forbidden");
    let a = AuthorityScope::conversation("externally-changed-room", AuthorityMode::Read).unwrap();
    let b = AuthorityScope::conversation("unrelated-new-room", AuthorityMode::Read).unwrap();
    let writer = f.manager.try_acquire(&[a.write_scope()]).unwrap();
    let affected = f.register_hinted_for(
        &a,
        std::slice::from_ref(&a),
        Instant::now() + Duration::from_secs(5),
    );
    let mut watcher = f.start();
    until(|| {
        f.registry
            .control
            .lock()
            .unwrap()
            .external_scopes
            .contains(&a)
    });
    let old_denial_invalidated = denied
        .observe()
        .err()
        .is_some_and(|error| error.code == "authority-busy");
    let no_local_notice = f.registry.control.lock().unwrap().writers.is_empty();
    let other = f.register_hinted_for(&b, &[b.clone()], Instant::now() + Duration::from_secs(5));
    let end = Instant::now() + Duration::from_secs(1);
    let mut current = None;
    while current.is_none() && Instant::now() < end {
        current = other.admit().ok();
        if current.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let disjoint_published_during_writer = current.as_ref().is_some_and(Lease::current);
    let matching_refused = affected.admit().is_err();
    drop(current);
    drop(writer);
    watcher.shutdown().unwrap();
    assert!(no_local_notice && disjoint_published_during_writer && matching_refused);
    assert!(original_denial && old_denial_invalidated);
}

#[test]
fn definitive_failure_with_unknown_footprint_stays_uncached_across_unrelated_writer() {
    let f = Fixture::new();
    let scope = AuthorityScope::conversation("hint-room", AuthorityMode::Read).unwrap();
    let other = AuthorityScope::conversation("unproven-room", AuthorityMode::Write).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let manager = f.manager.clone();
    let owner = f.owner.clone();
    let actual = scope.clone();
    let handle = f
        .registry
        .register_with_hints(&[scope], move || {
            if first.swap(false, Ordering::AcqRel) {
                entered_tx.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                return Err(Error::forbidden());
            }
            let guards = manager.try_acquire(std::slice::from_ref(&actual))?;
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
    f.registry.availability.enable().unwrap();
    let slot = handle.slot.clone();
    let registry = f.registry.clone();
    let worker = std::thread::spawn(move || slot.refresh_if_missing(&registry));
    let entered = entered_rx.recv_timeout(Duration::from_secs(1));
    let writer = f.manager.try_acquire(&[other]).unwrap();
    drop(writer);
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    let no_stale_denial = handle
        .observe()
        .err()
        .is_some_and(|e| e.code == "authority-busy")
        && handle.slot.refusal.load().is_none();
    handle.slot.refresh_if_missing(&f.registry);
    let fresh = handle.admit().is_ok();
    f.registry.invalidate_all();
    assert!(entered.is_ok() && no_stale_denial && fresh);
}

#[test]
fn definitive_missing_route_failure_is_not_cached_while_disjoint_writer_active() {
    let f = Fixture::new();
    let scope = AuthorityScope::conversation("missing-room", AuthorityMode::Read).unwrap();
    let other = AuthorityScope::conversation("writing-room", AuthorityMode::Write).unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    let handle: Handle<Facts> = f
        .registry
        .register_with_hints(&[scope], move || {
            calls.fetch_add(1, Ordering::AcqRel);
            Err(Error::forbidden())
        })
        .unwrap();
    f.registry.availability.enable().unwrap();
    let writer = f.manager.try_acquire(&[other]).unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let attempted_during_writer = attempts.load(Ordering::Acquire) == 1;
    let not_cached = handle.slot.refusal.load().is_none()
        && handle
            .observe()
            .err()
            .is_some_and(|error| error.code == "authority-busy");
    drop(writer);
    handle.slot.refresh_if_missing(&f.registry);
    let original_outcome_refreshed = attempts.load(Ordering::Acquire) == 2
        && handle
            .observe()
            .err()
            .is_some_and(|error| error.code == "forbidden");
    handle.slot.refresh_if_missing(&f.registry);
    let no_unchanged_sql_poll = attempts.load(Ordering::Acquire) == 2;
    f.registry.invalidate_all();
    assert!(
        attempted_during_writer
            && not_cached
            && original_outcome_refreshed
            && no_unchanged_sql_poll
    );
}

#[test]
fn cached_unknown_footprint_denial_is_cleared_by_disjoint_writer_then_refetched() {
    let f = Fixture::new();
    let hint = AuthorityScope::conversation("old-hint-room", AuthorityMode::Read).unwrap();
    let dependency =
        AuthorityScope::conversation("newly-allowed-room", AuthorityMode::Read).unwrap();
    let allowed = Arc::new(AtomicBool::new(false));
    let grant = allowed.clone();
    let manager = f.manager.clone();
    let owner = f.owner.clone();
    let scopes = vec![hint.clone(), dependency.clone()];
    let handle = f
        .registry
        .register_with_hints(&[hint], move || {
            if !grant.load(Ordering::Acquire) {
                return Err(Error::forbidden());
            }
            let guards = manager.try_acquire(&scopes)?;
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
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let cached = handle
        .observe()
        .err()
        .is_some_and(|error| error.code == "forbidden");
    let writer = f.manager.try_acquire(&[dependency.write_scope()]).unwrap();
    let stale_denial_cleared = handle.slot.refusal.load().is_none()
        && handle
            .observe()
            .err()
            .is_some_and(|error| error.code == "authority-busy");
    // A failed projection did not prove the old hint was complete. The actual
    // writer changes this dependency; a later positive grant fences BOTH scopes.
    allowed.store(true, Ordering::Release);
    drop(writer);
    handle.slot.refresh_if_missing(&f.registry);
    let fresh = handle.admit().is_ok();
    f.registry.invalidate_all();
    assert!(cached && stale_denial_cleared && fresh);
}

#[test]
fn known_definitive_failure_waits_for_disjoint_writer_outcome_before_reprojection() {
    let f = Fixture::new();
    let hint = AuthorityScope::conversation("denied-room", AuthorityMode::Read).unwrap();
    let other = AuthorityScope::conversation("pending-room", AuthorityMode::Write).unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    let handle: Handle<Facts> = f
        .registry
        .register_with_hints(&[hint], move || {
            calls.fetch_add(1, Ordering::AcqRel);
            Err(Error::forbidden())
        })
        .unwrap();
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let original_denial = handle
        .observe()
        .err()
        .is_some_and(|error| error.code == "forbidden");
    let writer = f.manager.try_acquire(&[other]).unwrap();
    for _ in 0..16 {
        handle.slot.refresh_if_missing(&f.registry);
    }
    let only_first_projection = attempts.load(Ordering::Acquire) == 1;
    let denial_withdrawn = handle
        .observe()
        .err()
        .is_some_and(|error| error.code == "authority-busy");
    drop(writer);
    handle.slot.refresh_if_missing(&f.registry);
    let one_after_original_outcome = attempts.load(Ordering::Acquire) == 2
        && handle
            .observe()
            .err()
            .is_some_and(|error| error.code == "forbidden");
    for _ in 0..16 {
        handle.slot.refresh_if_missing(&f.registry);
    }
    let no_unchanged_reprojection = attempts.load(Ordering::Acquire) == 2;
    f.registry.invalidate_all();
    assert!(original_denial && denial_withdrawn && only_first_projection);
    assert!(one_after_original_outcome && no_unchanged_reprojection);
}

#[test]
fn native_custody_erasure_keeps_same_allocation_and_last_io_reader() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(5));
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let typed = handle.admit().unwrap();
    let allocation = Arc::as_ptr(&typed.generation).cast::<()>();
    let weak = Arc::downgrade(&typed.generation);
    let stamp = typed.stamp();
    let original_epoch = typed.packet_epoch();
    let retained = typed.into_custody();
    assert_eq!(allocation, Arc::as_ptr(&retained.generation).cast::<()>());
    assert!(retained.stamp() == stamp && retained.current());
    let last = retained.clone();
    assert_eq!(allocation, Arc::as_ptr(&last.generation).cast::<()>());
    drop(handle);
    assert!(!retained.current() && !original_epoch.current());
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_err());
    drop(retained);
    assert!(weak.upgrade().is_some());
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_err());
    drop(last);
    assert!(weak.upgrade().is_none());
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_ok());
}

#[test]
fn native_custody_erasure_checks_original_facts_and_cannot_borrow_successor() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(5));
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let original = handle.admit().unwrap().into_custody();
    let stamp = original.stamp();
    let epoch = original.packet_epoch();
    f.owner.store(false, Ordering::Release);
    assert!(
        !original.current(),
        "custody must check facts, not only epoch"
    );
    assert!(
        epoch.current(),
        "this refusal is the original facts predicate"
    );
    f.owner.store(true, Ordering::Release);
    assert!(original.current());
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_err());
    assert!(!original.current() && !epoch.current());
    drop(original);
    let writer = f.manager.try_acquire(&[f.scope.write_scope()]).unwrap();
    f.revision.store(1, Ordering::Release);
    drop(writer);
    handle.slot.refresh_if_missing(&f.registry);
    let typed = handle.admit().unwrap();
    assert_eq!(typed.facts().revision, 1);
    let successor = typed.into_custody();
    assert!(successor.current() && successor.stamp() != stamp);
    assert!(!epoch.current());
    assert!(handle.admit_original(stamp).is_err());
    drop(successor);
    drop(handle);
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_ok());
}

#[test]
fn native_custody_erasure_preserves_original_expiry_and_runtime_drop() {
    let f = Fixture::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    let handle = f.register(deadline);
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let original = handle.admit().unwrap().into_custody();
    let epoch = original.packet_epoch();
    until(|| Instant::now() >= deadline);
    assert!(!original.current() && !epoch.current());
    assert!(handle.admit_original(original.stamp()).is_err());
    drop(original);
    drop(handle);
    let handle = f.register(Instant::now() + Duration::from_secs(5));
    handle.slot.refresh_if_missing(&f.registry);
    let original = handle.admit().unwrap().into_custody();
    let epoch = original.packet_epoch();
    let registry = Arc::downgrade(&f.registry);
    drop(f.registry);
    assert!(registry.upgrade().is_none());
    assert!(!original.current() && !epoch.current());
    drop(handle);
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_err());
    drop(original);
    assert!(f.manager.try_acquire(&[f.scope.write_scope()]).is_ok());
}

#[test]
fn missing_publication_backlog_remains_ready_after_one_bounded_refresh_turn() {
    let f = Fixture::new();
    let handles = (0..MAX_REFRESH_PER_TURN + 1)
        .map(|_| f.register(Instant::now() + Duration::from_secs(10)))
        .collect::<Vec<_>>();
    // Module-local model of the independently verified boundary; factories
    // below still acquire the actual Manager read fences for every generation.
    f.registry.availability.enable().unwrap();
    for handle in handles.iter().take(MAX_REFRESH_PER_TURN) {
        handle.slot.refresh_if_missing(&f.registry);
    }
    assert_eq!(f.refreshes.load(Ordering::Relaxed), MAX_REFRESH_PER_TURN);
    assert!(handles[..MAX_REFRESH_PER_TURN]
        .iter()
        .all(|h| h.admit().is_ok()));
    assert!(handles[MAX_REFRESH_PER_TURN].admit().is_err());
    {
        let state = f.registry.control.lock().unwrap();
        assert!(
            state
                .slots
                .iter()
                .filter_map(Weak::upgrade)
                .any(|slot| slot.refresh_ready(&state)),
            "a real pending factory must prevent the unconditional idle sleep"
        );
    }
    handles[MAX_REFRESH_PER_TURN]
        .slot
        .refresh_if_missing(&f.registry);
    let state = f.registry.control.lock().unwrap();
    assert!(handles.iter().all(|h| h.admit().is_ok()));
    assert!(
        !state
            .slots
            .iter()
            .filter_map(Weak::upgrade)
            .any(|slot| slot.refresh_ready(&state)),
        "healthy publications must not make the worker spin"
    );
}

#[test]
fn registration_during_refresh_is_ready_even_without_a_writer_sequence_change() {
    let f = Fixture::new();
    let handles = (0..MAX_REFRESH_PER_TURN)
        .map(|_| f.register(Instant::now() + Duration::from_secs(10)))
        .collect::<Vec<_>>();
    f.registry.availability.enable().unwrap();
    let sequence = f.registry.control.lock().unwrap().sequence;
    for h in &handles {
        h.slot.refresh_if_missing(&f.registry);
    }
    // This is the same registry call that may run while the original worker is
    // outside control, in a bounded factory. Its notify need not be retained by
    // a Condvar, so the pending-slot predicate must carry the wake condition.
    let new = f.register(Instant::now() + Duration::from_secs(10));
    let state = f.registry.control.lock().unwrap();
    assert_eq!(state.sequence, sequence);
    assert!(new.admit().is_err());
    assert!(new.slot.refresh_ready(&state));
    assert!(handles.iter().all(|h| !h.slot.refresh_ready(&state)));
    drop(state);
    new.slot.refresh_if_missing(&f.registry);
    assert!(new.admit().is_ok());
}

#[test]
fn withdrawn_publication_does_not_spin_or_borrow_old_epoch_during_original_writer() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    f.registry.availability.enable().unwrap();
    handle.slot.refresh_if_missing(&f.registry);
    let old = handle.admit().unwrap().stamp();
    let write = f.scope.write_scope();
    let writer = f.manager.try_acquire(&[write]).unwrap();
    assert!(handle.admit().is_err());
    assert!(handle.admit_original(old).is_err());
    let before = f.refreshes.load(Ordering::Relaxed);
    {
        let state = f.registry.control.lock().unwrap();
        assert!(
            !handle.slot.refresh_ready(&state),
            "live writer is not eligible work"
        );
    }
    handle.slot.refresh_if_missing(&f.registry);
    assert_eq!(f.refreshes.load(Ordering::Relaxed), before);
    drop(writer);
    {
        let state = f.registry.control.lock().unwrap();
        assert!(handle.slot.refresh_ready(&state));
    }
    handle.slot.refresh_if_missing(&f.registry);
    let fresh = handle.admit().unwrap();
    assert_ne!(fresh.stamp().generation, old.generation);
    assert!(handle.admit_original(old).is_err());
}

#[test]
fn deferred_or_definitive_refusal_keeps_original_wait_instead_of_busy_spinning() {
    let f = Fixture::new();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    f.registry.availability.enable().unwrap();
    let sequence = f.registry.control.lock().unwrap().sequence;
    {
        let mut attempt = handle.slot.attempt.lock().unwrap();
        attempt.sequence = Some(sequence);
        attempt.transient = true;
        // Deterministic scheduling state only, never a grant/packet deadline.
        attempt.retry_at = Instant::now() + Duration::from_secs(1);
    }
    let state = f.registry.control.lock().unwrap();
    assert!(!handle.slot.refresh_ready(&state));
    drop(state);
    {
        let mut attempt = handle.slot.attempt.lock().unwrap();
        attempt.transient = false;
        attempt.definitive = true;
    }
    let state = f.registry.control.lock().unwrap();
    assert!(!handle.slot.refresh_ready(&state));
    drop(state);
    f.owner.store(false, Ordering::Release);
    handle.slot.refresh_if_missing(&f.registry);
    assert!(handle.admit().is_err());
}

#[test]
fn original_worker_probes_and_refreshes_next_batch_without_an_intervening_idle_wait() {
    let f = Fixture::new();
    let waits = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..MAX_REFRESH_PER_TURN + 1 {
        let manager = f.manager.clone();
        let scope = f.scope.clone();
        let owner = f.owner.clone();
        let registry = Arc::downgrade(&f.registry);
        let waits = waits.clone();
        handles.push(
            f.registry
                .register(move || {
                    let registry = registry.upgrade().unwrap();
                    let observed = registry.original_idle_waits.load(Ordering::Relaxed);
                    let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
                    let prepared = Prepared::new(
                        Arc::new(Facts {
                            owner: owner.clone(),
                            revision: 0,
                        }),
                        guards,
                        Instant::now() + Duration::from_secs(10),
                    )?;
                    waits.lock().unwrap().push(observed);
                    Ok(prepared)
                })
                .unwrap(),
        );
    }
    let mut watcher = f.start();
    until(|| handles.iter().all(|h| h.admit().is_ok()));
    watcher.shutdown().unwrap();
    let waits = waits.lock().unwrap();
    assert_eq!(waits.len(), MAX_REFRESH_PER_TURN + 1);
    assert!(
        waits.iter().all(|n| *n == waits[0]),
        "actual original worker inserted an idle wait between eligible batches: {waits:?}"
    );
}
