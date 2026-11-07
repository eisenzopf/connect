use super::*;

fn admission(f: &Fixture) -> (String, LockFile) {
    let scope = f.scope(AuthorityMode::Read);
    let name = f.store.authority.name(&(scope.kind, scope.id)).unwrap();
    let file = f
        .store
        .authority
        .lock_file(&format!("{name}.admission"))
        .unwrap();
    lock_with(file.file(), None).unwrap();
    (name, file)
}

#[test]
fn preparing_one_resource_keeps_unrelated_scope_admission_live_and_own_writer_excluded() {
    let f = Fixture::new();
    let other = f.store.create_identity("independent").unwrap().id;
    let other = AuthorityScope::identity(&other, AuthorityMode::Read).unwrap();
    let manager = &f.store.authority;
    let (name, reserved) = admission(&f);
    let resource = manager
        .read_resource(&name, None, || {
            assert_eq!(
                f.store
                    .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
                    .unwrap_err()
                    .code,
                "authority-busy"
            );
            let independent = f
                .store
                .try_acquire_authority(&[other.clone()])
                .expect("resource preparation must not stop an unrelated scope");
            drop(independent);
            manager.lock_file(&format!("{name}.resource"))
        })
        .unwrap();
    drop(reserved);
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(resource);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap(),
    );
}

#[test]
fn busy_registry_after_resource_preparation_refuses_and_releases_its_original_file_lock() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    let (name, reserved) = admission(&f);
    let mut blocked = None;
    let result = manager.read_resource(&name, None, || {
        let file = manager.lock_file(&format!("{name}.resource"))?;
        blocked = Some(
            manager
                .readers
                .try_write()
                .expect("resource factory must run outside the registry"),
        );
        Ok(file)
    });
    drop(blocked);
    drop(reserved);
    assert!(matches!(result, Err(ref error) if error.code == "authority-busy"));
    let independent = manager.file(&format!("{name}.resource")).unwrap();
    lock_with(&independent, None).expect("refused original retained a resource lock");
    drop(independent);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap(),
    );
}

#[test]
fn an_existing_owned_resource_keeps_one_shared_lock_without_reentering_preparation() {
    let f = Fixture::new();
    let first = f
        .store
        .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
        .unwrap();
    let (name, reserved) = admission(&f);
    let second = f
        .store
        .authority
        .read_resource(&name, None, || {
            panic!("existing owned resource was reopened")
        })
        .unwrap();
    drop(reserved);
    drop(first);
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(second);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap(),
    );
}

#[test]
fn same_scope_read_admission_coexists_and_keeps_reserved_writer_exclusive() {
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let name = f
        .store
        .authority
        .name(&(scope.kind, scope.id.clone()))
        .unwrap();
    let preparing = f
        .store
        .authority
        .lock_file(&format!("{name}.admission"))
        .unwrap();
    lock_admission(preparing.file(), AuthorityMode::Read, None).unwrap();
    // Pause one actual reader at admission while a second enters the public
    // Store path. Read/read exclusion must not manufacture a packet deferral.
    let second = f
        .store
        .try_acquire_authority(&[scope.clone()])
        .expect("a concurrent reader must not defer another reader");
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(preparing);
    let store = f.store.clone();
    let write = f.scope(AuthorityMode::Write);
    let writer_scope = write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[writer_scope], &AuthorityWait::new(Duration::from_secs(2)))
    });
    waiting(&f.store, &write);
    let refused = f.store.try_acquire_authority(&[scope.clone()]);
    let writer_waited = !writer.is_finished();
    drop(second);
    let exclusive = writer.join().unwrap().unwrap();
    assert!(writer_waited, "writer overlapped an active resource reader");
    assert_eq!(
        refused.unwrap_err().code,
        "authority-busy",
        "cached reader bypassed a reserved writer"
    );
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(exclusive);
    drop(f.store.try_acquire_authority(&[scope]).unwrap());
}

#[test]
fn concurrent_cold_readers_coalesce_only_the_current_owned_resource() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    let scope = f.scope(AuthorityMode::Read);
    let name = manager.name(&(scope.kind, scope.id.clone())).unwrap();
    let admission = manager.lock_file(&format!("{name}.admission")).unwrap();
    lock_admission(admission.file(), AuthorityMode::Read, None).unwrap();
    let mut sibling = None;
    let resource = manager
        .read_resource(&name, None, || {
            // A second reader wins publication while our first cold resource is
            // being opened. Both own shared admission; neither may admit a writer.
            sibling = Some(f.store.try_acquire_authority(&[scope.clone()])?);
            manager.lock_file(&format!("{name}.resource"))
        })
        .unwrap();
    let sibling = sibling.unwrap();
    let Held::Read { _resource: other } = &sibling._held[0] else {
        panic!("expected reader")
    };
    assert!(
        Arc::ptr_eq(&resource, other),
        "concurrent cold readers published separate resources"
    );
    drop(admission);
    drop(sibling);
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(resource);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
            .unwrap(),
    );
}

#[test]
fn blocked_shared_reader_keeps_original_deadline_and_releases_partial_scopes() {
    let f = Fixture::new();
    let a = AuthorityScope::identity("reader_wait_a", AuthorityMode::Read).unwrap();
    let b = AuthorityScope::identity("reader_wait_b", AuthorityMode::Read).unwrap();
    let mut write_b = b.clone();
    write_b.mode = AuthorityMode::Write;
    let held = f.store.try_acquire_authority(&[write_b]).unwrap();
    let started = Instant::now();
    let wait = AuthorityWait::new(Duration::from_millis(15));
    assert_eq!(
        f.store
            .acquire_authority(&[a.clone(), b.clone()], &wait)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "reader renewed its original wait"
    );
    let mut write_a = a.clone();
    write_a.mode = AuthorityMode::Write;
    drop(
        f.store
            .try_acquire_authority(&[write_a])
            .expect("failed read retained its first scope"),
    );
    let cancelled = AuthorityWait::new(Duration::from_secs(1));
    cancelled.cancel();
    assert_eq!(
        f.store
            .acquire_authority(&[a.clone(), b.clone()], &cancelled)
            .unwrap_err()
            .code,
        "cancelled"
    );
    drop(held);
    drop(f.store.try_acquire_authority(&[a, b]).unwrap());
}

#[test]
fn warm_readers_share_registry_and_original_resource_without_bypassing_writer() {
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let first = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let Held::Read {
        _resource: original,
    } = &first._held[0]
    else {
        panic!("expected original reader");
    };
    let weak = Arc::downgrade(original);
    // Hold a real shared metadata guard throughout the second complete Store
    // admission. An exclusive warm lookup deterministically refuses here.
    let registry = f.store.authority.readers.read().unwrap();
    let store = f.store.clone();
    let other_scope = scope.clone();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(store.try_acquire_authority(&[other_scope]))
            .unwrap();
    });
    let result = received.recv_timeout(Duration::from_secs(1));
    drop(registry);
    worker.join().unwrap();
    let second = result
        .expect("warm admission blocked behind another shared registry guard")
        .expect("warm registry readers must coexist");
    let Held::Read { _resource: current } = &second._held[0] else {
        panic!("expected concurrent reader");
    };
    assert!(Arc::ptr_eq(original, current));
    assert!(first.covers(&[scope.clone()]));
    assert!(second.covers(&[scope]));
    let write = f.scope(AuthorityMode::Write);
    assert_eq!(
        f.store
            .try_acquire_authority(&[write.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(first);
    assert_eq!(
        f.store
            .try_acquire_authority(&[write.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(second);
    assert!(
        weak.upgrade().is_none(),
        "registry pinned an inactive reader"
    );
    drop(f.store.try_acquire_authority(&[write]).unwrap());
}

#[test]
fn cold_publication_prunes_expired_weak_metadata_without_unlocking_a_foreign_owner() {
    let f = Fixture::new();
    let scope = AuthorityScope::identity("expired_reader", AuthorityMode::Read).unwrap();
    let manager = &f.store.authority;
    let name = manager.name(&(scope.kind, scope.id.clone())).unwrap();
    let first = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let Held::Read {
        _resource: original,
    } = &first._held[0]
    else {
        panic!("expected original reader");
    };
    let weak = Arc::downgrade(original);
    drop(first);
    assert!(weak.upgrade().is_none());
    // The manager remains alive, but its stale weak entry holds no kernel lock.
    let foreign = manager.file(&format!("{name}.resource")).unwrap();
    lock_with(&foreign, None).expect("expired reader kept its resource locked");
    assert!(manager.readers.read().unwrap().contains_key(&name));
    let other = AuthorityScope::identity("cold_reader", AuthorityMode::Read).unwrap();
    let fresh = f.store.try_acquire_authority(&[other]).unwrap();
    assert!(
        !manager.readers.read().unwrap().contains_key(&name),
        "cold publication did not sweep expired weak metadata"
    );
    let probe = manager.file(&format!("{name}.resource")).unwrap();
    assert!(
        lock_with(&probe, None).is_err(),
        "sweep unlocked a foreign owner"
    );
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    drop(probe);
    drop(foreign);
    drop(fresh);
    drop(f.store.try_acquire_authority(&[scope]).unwrap());
}
