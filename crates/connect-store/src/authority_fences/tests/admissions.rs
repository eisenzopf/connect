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
    *f.store.authority.admissions.lock().unwrap() = IdleAdmissions::default();
}
#[test]
fn idle_admission_descriptions_hold_no_lock_and_checkout_is_exclusive() {
    let f = Fixture::new();
    clear(&f);
    let name = key(&f);
    let first = f.store.authority.admission(&name).unwrap();
    let fd = first.file().as_raw_fd();
    lock_with(first.file(), None).unwrap();
    drop(first);
    let first = f.store.authority.admission(&name).unwrap();
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
    let second = f.store.authority.admission(&name).unwrap();
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
    drop(f.store.authority.admission(&name).unwrap());
    let cache = f.store.authority.admissions.lock().unwrap();
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
    for index in 0..MAX_IDLE_ADMISSIONS + 8 {
        let scope = AuthorityScope::identity(&format!("idle_bounded_{index}"), AuthorityMode::Read)
            .unwrap();
        drop(f.store.try_acquire_authority(&[scope]).unwrap());
    }
    let cache = f.store.authority.admissions.lock().unwrap();
    assert!(cache.count <= MAX_IDLE_ADMISSIONS);
    assert_eq!(
        cache.count,
        cache.files.values().map(Vec::len).sum::<usize>()
    );
    assert!(cache
        .files
        .values()
        .all(|files| !files.is_empty() && files.len() <= MAX_IDLE_PER_NAME));
}
#[test]
fn cached_admission_revalidates_privacy_links_and_current_inode_before_locking() {
    for changed in 0..7 {
        let f = Fixture::new();
        clear(&f);
        let name = key(&f);
        drop(f.store.authority.admission(&name).unwrap());
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
            f.store.authority.admissions.lock().unwrap().count,
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
    drop(f.store.authority.admission(&name).unwrap());
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
    assert_eq!(f.store.authority.admissions.lock().unwrap().count, 0);
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
