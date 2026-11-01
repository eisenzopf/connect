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
                .try_lock()
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
