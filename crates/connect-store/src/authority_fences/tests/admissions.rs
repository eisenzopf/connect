use super::*;
use std::os::unix::{fs::PermissionsExt, io::AsRawFd};

fn key(f: &Fixture) -> String {
    let scope = f.scope(AuthorityMode::Read);
    format!(
        "{}.admission",
        f.store.authority.name(&(scope.kind, scope.id)).unwrap()
    )
}
fn clear(f: &Fixture) {
    *f.store.authority.idle_files.lock().unwrap() = IdleLockFiles::default();
}
#[test]
fn resource_description_returns_unlocked_only_after_the_final_owned_reader() {
    let f = Fixture::new();
    clear(&f);
    let scope = f.scope(AuthorityMode::Read);
    let name = format!(
        "{}.resource",
        f.store
            .authority
            .name(&(scope.kind, scope.id.clone()))
            .unwrap()
    );
    let first = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let second = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    assert!(!f
        .store
        .authority
        .idle_files
        .lock()
        .unwrap()
        .files
        .contains_key(&name));
    drop(first);
    assert!(!f
        .store
        .authority
        .idle_files
        .lock()
        .unwrap()
        .files
        .contains_key(&name));
    let independent = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&independent, None).is_err(),
        "last owned reader lost its lock"
    );
    drop(second);
    assert!(
        f.store
            .authority
            .idle_files
            .lock()
            .unwrap()
            .files
            .contains_key(&name),
        "resource description was closed instead of returned unlocked"
    );
    lock_with(&independent, None).unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable",
        "idle resource description bypassed an independent exclusive lock"
    );
    let another = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&another, None).is_err(),
        "failed checkout unlocked a foreign description"
    );
    drop(independent);
    drop(another);
    let recovered = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let mut write = scope;
    write.mode = AuthorityMode::Write;
    assert_eq!(
        f.store
            .try_acquire_authority(&[write.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(recovered);
    drop(f.store.try_acquire_authority(&[write]).unwrap());
}
#[test]
fn cached_resource_rechecks_current_privacy_inode_links_and_effective_access() {
    let cases = if cfg!(target_os = "macos") { 8 } else { 7 };
    for changed in 0..cases {
        let f = Fixture::new();
        clear(&f);
        let scope = f.scope(AuthorityMode::Read);
        let name = format!(
            "{}.resource",
            f.store
                .authority
                .name(&(scope.kind, scope.id.clone()))
                .unwrap()
        );
        drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
        let path = f.store.authority.directory.join(&name);
        let old = f.store.authority.directory.join("saved-resource");
        match changed {
            0 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            1 => {
                fs::rename(&path, &old).unwrap();
                f.store.authority.file(&name).unwrap();
            }
            2 => fs::remove_file(&path).unwrap(),
            3 => {
                fs::rename(&path, &old).unwrap();
                std::os::unix::fs::symlink(&old, &path).unwrap();
            }
            4 => fs::hard_link(&path, &old).unwrap(),
            5 => fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap(),
            6 => fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap(),
            #[cfg(target_os = "macos")]
            7 => assert!(std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone deny read,write"])
                .arg(&path)
                .status()
                .unwrap()
                .success()),
            _ => unreachable!(),
        }
        assert_eq!(
            f.store.try_acquire_authority(&[scope]).unwrap_err().code,
            "authority-unavailable",
            "unsafe resource {changed} admitted"
        );
        assert!(
            !f.store
                .authority
                .idle_files
                .lock()
                .unwrap()
                .files
                .contains_key(&name),
            "unsafe resource was recycled"
        );
        #[cfg(target_os = "macos")]
        if changed == 7 {
            assert!(std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(&path)
                .status()
                .unwrap()
                .success());
        }
    }
}
#[test]
fn evicting_an_idle_resource_cannot_unlock_an_independent_active_description() {
    let f = Fixture::new();
    clear(&f);
    let scope = f.scope(AuthorityMode::Read);
    let name = format!(
        "{}.resource",
        f.store
            .authority
            .name(&(scope.kind, scope.id.clone()))
            .unwrap()
    );
    drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
    let independent = f.store.authority.file(&name).unwrap();
    lock_with(&independent, None).unwrap();
    for index in 0..MAX_IDLE_LOCK_FILES + 2 {
        drop(
            f.store
                .authority
                .lock_file(&format!("evict_{index}.admission"))
                .unwrap(),
        );
    }
    assert!(!f
        .store
        .authority
        .idle_files
        .lock()
        .unwrap()
        .files
        .contains_key(&name));
    let another = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&another, None).is_err(),
        "evicting an idle FD released another owner's lock"
    );
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    drop(independent);
    drop(another);
    drop(f.store.try_acquire_authority(&[scope]).unwrap());
}
#[test]
fn idle_admission_descriptions_hold_no_lock_and_checkout_is_exclusive() {
    let f = Fixture::new();
    clear(&f);
    let name = key(&f);
    let first = f.store.authority.lock_file(&name).unwrap();
    let fd = first.file().as_raw_fd();
    lock_with(first.file(), None).unwrap();
    drop(first);
    let first = f.store.authority.lock_file(&name).unwrap();
    assert_eq!(
        first.file().as_raw_fd(),
        fd,
        "warm descriptor was not reused"
    );
    // An independent open must succeed while the checkout is still unlocked.
    let independent = f.store.authority.file(&name).unwrap();
    lock_with(&independent, None).unwrap();
    assert!(lock_with(first.file(), None).is_err());
    drop(independent);
    lock_with(first.file(), None).unwrap();
    let second = f.store.authority.lock_file(&name).unwrap();
    assert_ne!(first.file().as_raw_fd(), second.file().as_raw_fd());
    assert!(
        lock_with(second.file(), None).is_err(),
        "two checkouts shared an open description"
    );
    drop(first);
    lock_with(second.file(), None).unwrap();
    drop(second);
    let writer = f
        .store
        .try_acquire_authority(&[f.scope(AuthorityMode::Write)])
        .unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(writer);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap(),
    );
}
#[test]
fn admission_pool_never_waits_on_cache_and_bounds_idle_descriptors() {
    let f = Fixture::new();
    clear(&f);
    let name = key(&f);
    drop(f.store.authority.lock_file(&name).unwrap());
    let cache = f.store.authority.idle_files.lock().unwrap();
    let store = f.store.clone();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = store.try_acquire_authority(&[AuthorityScope::identity(
            "cache_busy",
            AuthorityMode::Read,
        )
        .unwrap()]);
        sent.send(result).unwrap();
    });
    let current = received
        .recv_timeout(Duration::from_millis(500))
        .unwrap()
        .unwrap();
    drop(current);
    drop(cache);
    worker.join().unwrap();
    for index in 0..MAX_IDLE_LOCK_FILES + 8 {
        let scope = AuthorityScope::identity(&format!("idle_bounded_{index}"), AuthorityMode::Read)
            .unwrap();
        drop(f.store.try_acquire_authority(&[scope]).unwrap());
    }
    let cache = f.store.authority.idle_files.lock().unwrap();
    assert!(cache.count <= MAX_IDLE_LOCK_FILES);
    assert_eq!(
        cache.count,
        cache
            .files
            .values()
            .map(|entry| entry.files.len())
            .sum::<usize>()
    );
    assert!(cache
        .files
        .values()
        .all(|entry| !entry.files.is_empty() && entry.files.len() <= MAX_IDLE_PER_NAME));
}
#[test]
fn active_media_working_set_can_replace_idle_cold_namespace_descriptions() {
    let f = Fixture::new();
    clear(&f);
    for index in 0..MAX_IDLE_LOCK_FILES {
        drop(
            f.store
                .authority
                .lock_file(&format!("cold_{index}.admission"))
                .unwrap(),
        );
    }
    assert_eq!(
        f.store.authority.idle_files.lock().unwrap().count,
        MAX_IDLE_LOCK_FILES
    );
    let names = (0..51)
        .map(|i| format!("media_{i}.admission"))
        .collect::<Vec<_>>();
    for name in &names {
        drop(f.store.authority.lock_file(name).unwrap());
    }
    {
        let pool = f.store.authority.idle_files.lock().unwrap();
        assert_eq!(pool.count, MAX_IDLE_LOCK_FILES);
        for name in &names {
            assert!(
                pool.files.contains_key(name),
                "cold namespace pinned out {name}"
            );
        }
    }
    for _ in 0..3 {
        for name in &names {
            let admission = f.store.authority.lock_file(name).unwrap();
            // Replacement keeps only idle, unlocked descriptions. A separate
            // process/open description must remain able to take this lock.
            let independent = f.store.authority.file(name).unwrap();
            lock_with(&independent, None).unwrap();
            assert!(lock_with(admission.file(), None).is_err());
            drop(independent);
            lock_with(admission.file(), None).unwrap();
        }
    }
    let pool = f.store.authority.idle_files.lock().unwrap();
    assert_eq!(pool.count, MAX_IDLE_LOCK_FILES);
    assert!(names.iter().all(|name| pool.files.contains_key(name)));
}
#[test]
fn cached_admission_revalidates_privacy_links_and_current_inode_before_locking() {
    for changed in 0..7 {
        let f = Fixture::new();
        clear(&f);
        let name = key(&f);
        drop(f.store.authority.lock_file(&name).unwrap());
        let path = f.store.authority.directory.join(&name);
        let old = f.store.authority.directory.join("saved-admission");
        match changed {
            0 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            1 => {
                fs::rename(&path, &old).unwrap();
                f.store.authority.file(&name).unwrap();
            }
            2 => {
                fs::remove_file(&path).unwrap();
            }
            3 => {
                fs::rename(&path, &old).unwrap();
                std::os::unix::fs::symlink(&old, &path).unwrap();
            }
            4 => {
                fs::hard_link(&path, &old).unwrap();
            }
            5 => fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap(),
            6 => fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap(),
            _ => unreachable!(),
        }
        let error = f
            .store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap_err();
        assert_eq!(
            error.code, "authority-unavailable",
            "unsafe cached file {changed} admitted"
        );
        assert_eq!(
            f.store.authority.idle_files.lock().unwrap().count,
            0,
            "unsafe descriptor was recycled"
        );
    }
}
#[cfg(target_os = "macos")]
#[test]
fn cached_admission_respects_current_acl_even_when_owner_and_mode_are_unchanged() {
    let f = Fixture::new();
    clear(&f);
    let name = key(&f);
    drop(f.store.authority.lock_file(&name).unwrap());
    let path = f.store.authority.directory.join(&name);
    assert!(std::process::Command::new("/bin/chmod")
        .args(["+a", "everyone deny read,write"])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .is_err());
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap_err()
            .code,
        "authority-unavailable"
    );
    assert_eq!(f.store.authority.idle_files.lock().unwrap().count, 0);
    assert!(std::process::Command::new("/bin/chmod")
        .arg("-N")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap(),
    );
}
#[test]
fn cached_admission_preserves_queued_writer_and_releases_partial_read_scopes() {
    let f = Fixture::new();
    let a = AuthorityScope::identity("idle_a", AuthorityMode::Read).unwrap();
    let b = AuthorityScope::identity("idle_b", AuthorityMode::Read).unwrap();
    drop(
        f.store
            .try_acquire_authority(&[a.clone(), b.clone()])
            .unwrap(),
    );
    let held = f.store.try_acquire_authority(&[b.clone()]).unwrap();
    let store = f.store.clone();
    let mut write = b.clone();
    write.mode = AuthorityMode::Write;
    let wait_for = write.clone();
    let writer = std::thread::spawn(move || {
        store.acquire_authority(&[write], &AuthorityWait::new(Duration::from_secs(2)))
    });
    waiting(&f.store, &wait_for);
    assert_eq!(
        f.store
            .try_acquire_authority(&[a.clone(), b.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    let mut write_a = a;
    write_a.mode = AuthorityMode::Write;
    drop(f.store.try_acquire_authority(&[write_a]).unwrap());
    assert!(!writer.is_finished());
    drop(held);
    drop(writer.join().unwrap().unwrap());
    drop(f.store.try_acquire_authority(&[b]).unwrap());
}
