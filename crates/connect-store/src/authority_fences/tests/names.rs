use super::*;

#[test]
fn name_metadata_is_bounded_namespace_specific_and_never_waits_on_its_map() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    let key = (Kind::Identity, "same_id".to_string());
    let expected = opaque(&manager.secret, b"identity", b"same_id").unwrap();
    assert_eq!(manager.name(&key).unwrap(), expected);
    assert_ne!(
        manager.name(&(Kind::Conversation, key.1.clone())).unwrap(),
        expected
    );
    assert_ne!(
        manager.name(&(Kind::Record, key.1.clone())).unwrap(),
        expected
    );
    let other = Fixture::new();
    assert_ne!(other.store.authority.name(&key).unwrap(), expected);
    let held = manager.names.lock().unwrap();
    let store = f.store.clone();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(store.authority.name(&key)).unwrap();
    });
    let result = received.recv_timeout(Duration::from_millis(500));
    drop(held);
    worker.join().unwrap();
    assert_eq!(result.unwrap().unwrap(), expected);
    for index in 0..MAX_SCOPES + 1 {
        let key = (Kind::Identity, format!("bounded_{index}"));
        assert_eq!(
            manager.name(&key).unwrap(),
            opaque(&manager.secret, b"identity", key.1.as_bytes()).unwrap()
        );
    }
    assert_eq!(manager.names.lock().unwrap().len(), MAX_SCOPES);
}

#[cfg(unix)]
#[test]
fn cached_names_still_check_fresh_file_privacy_and_reject_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let name = f
        .store
        .authority
        .name(&(scope.kind, scope.id.clone()))
        .unwrap();
    drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
    let path = f
        .store
        .authority
        .directory
        .join(format!("{name}.admission"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&path).unwrap();
    let target = f.dir.path().join("substitute");
    File::create(&target).unwrap();
    symlink(&target, &path).unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    fs::remove_file(path).unwrap();
    assert!(f.store.try_acquire_authority(&[scope]).is_ok());
}
