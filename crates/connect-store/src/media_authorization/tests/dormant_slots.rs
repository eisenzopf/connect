//! Cold job scheduling only; the real Manager/worker still owns all authority.
use super::*;

fn dormant(f: &Fixture, expires: Instant, allowed: Arc<AtomicBool>) -> Handle<Facts> {
    let manager = f.manager.clone();
    let scope = f.scope.clone();
    let owner = f.owner.clone();
    let calls = f.refreshes.clone();
    f.registry
        .try_register_dormant_with_hints(&[scope.clone()], move || {
            calls.fetch_add(1, Ordering::Relaxed);
            if !allowed.load(Ordering::Acquire) {
                return Err(Error::forbidden());
            }
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 0,
                }),
                guards,
                expires,
            )
        })
        .unwrap()
}

#[test]
fn dormant_slot_never_calls_or_caches_prephase_denial_and_activation_never_renews_stamps() {
    let f = Fixture::new();
    let allowed = Arc::new(AtomicBool::new(false));
    let expires = Instant::now() + Duration::from_secs(5);
    let handle = dormant(&f, expires, allowed.clone());
    let mut watcher = f.start();
    until(|| f.registry.original_idle_waits.load(Ordering::Relaxed) > 0);
    for _ in 0..16 {
        handle.slot.refresh_if_missing(&f.registry);
    }
    let before_calls = f.refreshes.load(Ordering::Relaxed);
    let before_attempt = {
        let state = f.registry.control.lock().unwrap();
        let ready = handle.slot.refresh_ready(&state);
        let attempt = handle.slot.attempt.lock().unwrap();
        (
            ready,
            attempt.sequence,
            attempt.transient,
            attempt.definitive,
        )
    };
    let refused_before_activation = handle.admit().is_err() && handle.observe().is_err();
    let no_cached_denial = handle.slot.refusal.load().is_none();
    allowed.store(true, Ordering::Release);
    f.registry.try_activate_dormant(&handle).unwrap();
    let lease = admit(&handle);
    let stamp = lease.stamp();
    let generation = lease.generation.number;
    let cutoff_unchanged = lease.generation.prepared.until == expires;
    f.registry.try_activate_dormant(&handle).unwrap();
    let still_same = handle.admit_original(stamp).unwrap();
    let same_generation = still_same.generation.number == generation;
    drop(still_same);
    // A normal real writer invalidates the exact old epoch; activation cannot
    // clear that invalidation or turn an old queued stamp into a new read.
    let notice = f.registry.before_write(&[f.scope.clone()]).unwrap();
    let old_revoked = !lease.current() && handle.admit_original(stamp).is_err();
    f.registry.try_activate_dormant(&handle).unwrap();
    let not_reactivated = handle.admit_original(stamp).is_err();
    drop(lease);
    drop(notice);
    let fresh = admit(&handle);
    let fresh_generation = fresh.generation.number > generation;
    let old_stamp_still_refused = handle.admit_original(stamp).is_err();
    drop(fresh);
    watcher.shutdown().unwrap();
    assert_eq!(before_calls, 0);
    assert_eq!(before_attempt, (false, None, false, false));
    assert!(refused_before_activation && no_cached_denial && cutoff_unchanged && same_generation);
    assert!(old_revoked && not_reactivated && fresh_generation && old_stamp_still_refused);
}

#[test]
fn dormant_activation_refuses_foreign_ordinary_control_busy_and_original_expiry() {
    let f = Fixture::new();
    let foreign = Fixture::new();
    let handle = dormant(
        &f,
        Instant::now() - Duration::from_millis(1),
        Arc::new(AtomicBool::new(true)),
    );
    let ordinary = f.register(Instant::now() + Duration::from_secs(5));
    let mut watcher = f.start();
    let foreign_refused = foreign
        .registry
        .try_activate_dormant(&handle)
        .unwrap_err()
        .code
        == "forbidden";
    let ordinary_refused =
        f.registry.try_activate_dormant(&ordinary).unwrap_err().code == "forbidden";
    let held = f.registry.control.lock().unwrap();
    let busy = f.registry.try_activate_dormant(&handle).unwrap_err().code == "authority-busy";
    let dormant_after_busy = !handle
        .slot
        .activation
        .as_ref()
        .unwrap()
        .load(Ordering::Acquire);
    drop(held);
    f.registry.try_activate_dormant(&handle).unwrap();
    // Run the exact unchanged expiry/factory path off media; no new lifetime.
    let expired = f.registry.try_prepare_current(&handle).is_err();
    let not_published = handle.admit().is_err() && handle.slot.publication.load().is_none();
    drop(ordinary);
    watcher.shutdown().unwrap();
    let stopped_refused = f.registry.try_activate_dormant(&handle).is_err();
    assert!(foreign_refused && ordinary_refused && busy && dormant_after_busy);
    assert!(expired && not_published && stopped_refused);
}

#[test]
fn dormant_activation_during_original_worker_turn_is_ready_without_sequence_or_idle_wait() {
    let f = Fixture::new();
    let expires = Instant::now() + Duration::from_secs(5);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let once = AtomicBool::new(false);
    let manager = f.manager.clone();
    let scope = f.scope.clone();
    let owner = f.owner.clone();
    let first = f
        .registry
        .register_with_hints(&[scope.clone()], move || {
            if !once.swap(true, Ordering::AcqRel) {
                entered_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| Error::storage())?;
            }
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 0,
                }),
                guards,
                expires,
            )
        })
        .unwrap();
    let others = (0..7).map(|_| f.register(expires)).collect::<Vec<_>>();
    let manager = f.manager.clone();
    let scope = f.scope.clone();
    let owner = f.owner.clone();
    let registry = Arc::downgrade(&f.registry);
    let (projected_tx, projected_rx) = mpsc::channel();
    let dormant = f
        .registry
        .try_register_dormant_with_hints(&[scope.clone()], move || {
            let waits = registry
                .upgrade()
                .ok_or_else(Error::forbidden)?
                .original_idle_waits
                .load(Ordering::Relaxed);
            projected_tx.send(waits).unwrap();
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 0,
                }),
                guards,
                expires,
            )
        })
        .unwrap();
    let mut watcher = f.start();
    let entered = entered_rx.recv_timeout(Duration::from_secs(1)).is_ok();
    let (sequence, idle_waits) = {
        let state = f.registry.control.lock().unwrap();
        (
            state.sequence,
            f.registry.original_idle_waits.load(Ordering::Relaxed),
        )
    };
    let activated = f.registry.try_activate_dormant(&dormant).is_ok();
    let same_sequence = f.registry.control.lock().unwrap().sequence == sequence;
    // Always release the original factory before asserting or joining worker.
    let _ = release_tx.send(());
    let projected = projected_rx.recv_timeout(Duration::from_secs(1)).ok();
    let until = Instant::now() + Duration::from_secs(1);
    let mut admitted = false;
    while !admitted && Instant::now() < until {
        admitted = dormant.admit().is_ok();
        if !admitted {
            std::thread::yield_now();
        }
    }
    drop((first, others, dormant));
    watcher.shutdown().unwrap();
    assert!(entered && activated && same_sequence && admitted);
    assert_eq!(
        projected,
        Some(idle_waits),
        "activation waited behind an unnecessary original idle turn"
    );
}
