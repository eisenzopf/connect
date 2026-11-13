//! Control-only snapshot preparation under real Manager Read/Write fences.
//! Narrow Registry/real Manager fence controls, not a Service/Root/media proof.
//! No background refresher is scheduled: this deterministically isolates whether
//! the control call executes the real guarded factory before first admission.
use super::*;

fn cold_control_prepare(eager: bool) -> bool {
    let f = Fixture::new();
    // Test-only fixture enables the private boundary without a background
    // refresher. Production start/enable requirements remain unchanged.
    f.registry.availability.enable().unwrap();
    let caller = std::thread::current().id();
    let threads = Arc::new(Mutex::new(Vec::new()));
    let captured_threads = threads.clone();
    let manager = f.manager.clone();
    let owner = f.owner.clone();
    let refreshes = f.refreshes.clone();
    let scope = f.scope.clone();
    let cutoff = Instant::now() + Duration::from_secs(10);
    let handle = f
        .registry
        .register_with_hints(&[f.scope.clone()], move || {
            captured_threads
                .lock()
                .unwrap()
                .push(std::thread::current().id());
            let guards = manager.try_acquire(std::slice::from_ref(&scope))?;
            refreshes.fetch_add(1, Ordering::Relaxed);
            Prepared::new(
                Arc::new(Facts {
                    owner: owner.clone(),
                    revision: 0,
                }),
                guards,
                cutoff,
            )
        })
        .unwrap();
    assert_eq!(f.refreshes.load(Ordering::Relaxed), 0);
    assert!(
        handle.admit().is_err(),
        "cold handle unexpectedly had a grant"
    );
    // The production factory itself is unchanged and acquires real Manager
    // Read fences. The proposed control API is the only invocation difference.
    if eager {
        f.registry.try_prepare_current(&handle).unwrap();
    }
    let first = handle.admit();
    let result = first.as_ref().is_ok_and(Lease::current);
    if eager {
        assert_eq!(f.refreshes.load(Ordering::Relaxed), 1);
        let original_stamp = first.as_ref().unwrap().stamp();
        drop(first);
        for _ in 0..2 {
            f.registry.try_prepare_current(&handle).unwrap();
            drop(handle.admit_original(original_stamp).unwrap());
        }
        assert_eq!(
            f.refreshes.load(Ordering::Relaxed),
            1,
            "repeated control preparation replaced a healthy publication"
        );
        for _ in 0..100 {
            drop(handle.admit().unwrap());
        }
        assert_eq!(
            f.refreshes.load(Ordering::Relaxed),
            1,
            "packet admission invoked the authorization factory"
        );
        assert_eq!(
            *threads.lock().unwrap(),
            vec![caller],
            "guarded factory did not run only on the explicit control caller"
        );
    } else {
        drop(first);
    }
    drop(handle);
    result
}

#[test]
fn explicit_control_refresh_prepares_first_actual_admission_without_packet_sql() {
    assert!(cold_control_prepare(true));
}

#[test]
fn control_refresh_is_store_scoped_and_does_not_refresh_healthy_disjoint_slot() {
    let f = Fixture::new();
    let other = Fixture::new();
    f.registry.availability.enable().unwrap();
    other.registry.availability.enable().unwrap();
    let selected = f.register(Instant::now() + Duration::from_secs(10));
    let unrelated = f.register_for(
        &AuthorityScope::identity("unrelated-route-owner", AuthorityMode::Read).unwrap(),
        Instant::now() + Duration::from_secs(10),
    );
    f.registry.try_prepare_current(&unrelated).unwrap();
    let original = unrelated.admit().unwrap();
    let stamp = original.stamp();
    let before = f.refreshes.load(Ordering::Relaxed);
    assert_eq!(
        other
            .registry
            .try_prepare_current(&selected)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.refreshes.load(Ordering::Relaxed), before);
    f.registry.try_prepare_current(&selected).unwrap();
    assert!(unrelated.admit_original(stamp).is_ok());
    assert!(original.current());
    drop(original);
    drop((selected, unrelated));
}

#[test]
fn writer_and_regrant_never_renew_original_queued_epoch() {
    let f = Fixture::new();
    f.registry.availability.enable().unwrap();
    let handle = f.register(Instant::now() + Duration::from_secs(10));
    f.registry.try_prepare_current(&handle).unwrap();
    let original = handle.admit().unwrap();
    let epoch = original.packet_epoch();
    let stamp = original.stamp();
    drop(original);
    let writer = f
        .manager
        .try_acquire(&[AuthorityScope::identity("snapshot-owner", AuthorityMode::Write).unwrap()])
        .unwrap();
    f.owner.store(false, Ordering::Release);
    assert!(!epoch.current());
    assert!(f.registry.try_prepare_current(&handle).is_err());
    drop(writer);
    assert!(f.registry.try_prepare_current(&handle).is_err());
    let writer = f
        .manager
        .try_acquire(&[AuthorityScope::identity("snapshot-owner", AuthorityMode::Write).unwrap()])
        .unwrap();
    f.owner.store(true, Ordering::Release);
    f.revision.fetch_add(1, Ordering::AcqRel);
    drop(writer);
    f.registry.try_prepare_current(&handle).unwrap();
    assert!(handle.admit().unwrap().current());
    assert!(!epoch.current());
    assert!(handle.admit_original(stamp).is_err());
    drop(handle);
}

#[test]
fn expired_or_cancelled_owner_cannot_be_prepared() {
    let f = Fixture::new();
    f.registry.availability.enable().unwrap();
    let expired = f.register(Instant::now() - Duration::from_millis(1));
    assert!(f.registry.try_prepare_current(&expired).is_err());
    let cancelled = f.register(Instant::now() + Duration::from_secs(10));
    f.owner.store(false, Ordering::Release);
    assert!(f.registry.try_prepare_current(&cancelled).is_err());
    assert!(cancelled.admit().is_err());
    drop((expired, cancelled));
}
