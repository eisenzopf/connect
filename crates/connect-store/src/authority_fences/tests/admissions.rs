use super::*;
use std::os::unix::{
    fs::{OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};

fn name(f: &Fixture, scope: &AuthorityScope, kind: &str) -> String {
    format!(
        "{}.{}",
        f.store
            .authority
            .name(&(scope.kind, scope.id.clone()))
            .unwrap(),
        kind
    )
}
fn key(f: &Fixture) -> String {
    name(f, &f.scope(AuthorityMode::Read), "admission")
}
fn replace_private_file(path: &Path) {
    drop(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap(),
    );
}

#[test]
fn resource_description_remains_locked_until_the_final_owned_reader() {
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let name = name(&f, &scope, "resource");
    let first = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let second = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let (Held::Read { _resource: a }, Held::Read { _resource: b }) =
        (&first._held[0], &second._held[0])
    else {
        panic!("read guards expected");
    };
    assert!(
        Arc::ptr_eq(a, b),
        "overlapping readers lost live-resource coalescing"
    );
    drop(first);
    let independent = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&independent, None).is_err(),
        "last owned reader lost its lock"
    );
    drop(second);
    lock_with(&independent, None).unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-unavailable",
        "new resource bypassed an independent exclusive lock"
    );
    let another = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&another, None).is_err(),
        "failed open unlocked a foreign description"
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
fn fresh_admission_and_resource_recheck_privacy_inode_links_and_access() {
    for kind in ["admission", "resource"] {
        for changed in 0..7 {
            let f = Fixture::new();
            let scope = f.scope(AuthorityMode::Read);
            let name = name(&f, &scope, kind);
            drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
            let path = f.store.authority.directory.join(&name);
            let old = f.store.authority.directory.join("saved-original");
            match changed {
                0 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
                1 => {
                    fs::rename(&path, &old).unwrap();
                    replace_private_file(&path);
                }
                2 => fs::remove_file(&path).unwrap(),
                3 => {
                    fs::rename(&path, &old).unwrap();
                    std::os::unix::fs::symlink(&old, &path).unwrap();
                }
                4 => fs::hard_link(&path, &old).unwrap(),
                5 => fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap(),
                6 => fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap(),
                _ => unreachable!(),
            }
            for _ in 0..2 {
                assert_eq!(
                    f.store
                        .try_acquire_authority(&[scope.clone()])
                        .unwrap_err()
                        .code,
                    "authority-unavailable",
                    "unsafe {kind} case {changed} admitted"
                );
            }
            if changed == 2 {
                assert!(
                    !path.exists(),
                    "remembered namespace deletion was silently recreated"
                );
            }
        }
    }
}

#[test]
fn validation_rejects_path_replacement_after_the_original_descriptor_opened() {
    let f = Fixture::new();
    let name = key(&f);
    let opened = f.store.authority.file(&name).unwrap();
    let path = f.store.authority.directory.join(&name);
    let old = f.store.authority.directory.join("opened-original");
    fs::rename(&path, &old).unwrap();
    replace_private_file(&path);
    // Even a first open without remembered metadata must compare its own
    // descriptor with the current path after IO, before admission can proceed.
    assert!(
        matches!(f.store.authority.validate_lock_file(&name, &opened, None),
        Err(error) if error.code == "authority-unavailable")
    );
    drop(opened);
}

#[cfg(target_os = "macos")]
#[test]
fn fresh_admission_and_resource_respect_current_acl_with_unchanged_owner_and_mode() {
    for kind in ["admission", "resource"] {
        let f = Fixture::new();
        let scope = f.scope(AuthorityMode::Read);
        let name = name(&f, &scope, kind);
        drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
        let path = f.store.authority.directory.join(&name);
        assert!(Command::new("/bin/chmod")
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
        let refused = f.store.try_acquire_authority(&[scope.clone()]);
        // Restore the same inode before asserting so fixture cleanup keeps its
        // ordinary user authority even if the production assertion fails.
        assert!(Command::new("/bin/chmod")
            .arg("-N")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        assert_eq!(refused.unwrap_err().code, "authority-unavailable");
        drop(f.store.try_acquire_authority(&[scope]).unwrap());
    }
}

#[test]
fn independent_admission_descriptions_never_share_kernel_lock_ownership() {
    let f = Fixture::new();
    let name = key(&f);
    let first = f.store.authority.lock_file(&name).unwrap();
    lock_with(first.file(), None).unwrap();
    let second = f.store.authority.lock_file(&name).unwrap();
    assert_ne!(first.file().as_raw_fd(), second.file().as_raw_fd());
    assert!(
        lock_with(second.file(), None).is_err(),
        "two owners shared an open description"
    );
    drop(first);
    lock_with(second.file(), None).unwrap();
    let independent = f.store.authority.file(&name).unwrap();
    assert!(lock_with(&independent, None).is_err());
    drop(second);
    lock_with(&independent, None).unwrap();
    assert_eq!(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(independent);
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap(),
    );
}

#[test]
fn identity_metadata_contention_refuses_immediately_without_bypassing_validation() {
    let f = Fixture::new();
    let name = key(&f);
    drop(f.store.authority.lock_file(&name).unwrap());
    let held = f.store.authority.file_identities.write().unwrap();
    let store = f.store.clone();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(store.try_acquire_authority(&[
            AuthorityScope::identity("metadata_busy", AuthorityMode::Read).unwrap(),
        ]))
        .unwrap();
    });
    let result = received.recv_timeout(Duration::from_millis(500));
    drop(held);
    worker.join().unwrap();
    assert_eq!(result.unwrap().unwrap_err().code, "authority-busy");
    drop(
        f.store
            .try_acquire_authority(&[f.scope(AuthorityMode::Read)])
            .unwrap(),
    );
}

#[test]
fn bounded_identity_metadata_eviction_cannot_unlock_an_independent_owner() {
    let f = Fixture::new();
    let scope = f.scope(AuthorityMode::Read);
    let name = name(&f, &scope, "resource");
    drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
    let independent = f.store.authority.file(&name).unwrap();
    lock_with(&independent, None).unwrap();
    for index in 0..MAX_FILE_IDENTITIES + 8 {
        drop(
            f.store
                .authority
                .lock_file(&format!("metadata_{index}.admission"))
                .unwrap(),
        );
    }
    {
        let identities = f.store.authority.file_identities.read().unwrap();
        assert_eq!(identities.len(), MAX_FILE_IDENTITIES);
        assert!(!identities.contains_key(&name));
    }
    let another = f.store.authority.file(&name).unwrap();
    assert!(
        lock_with(&another, None).is_err(),
        "metadata eviction released another owner's lock"
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
fn warm_identity_open_refreshes_recency_without_growing_the_registry() {
    let f = Fixture::new();
    let manager = &f.store.authority;
    for index in 0..MAX_FILE_IDENTITIES {
        drop(manager.file(&format!("recency_{index}.admission")).unwrap());
    }
    drop(manager.file("recency_0.admission").unwrap());
    drop(manager.file("recency_new.admission").unwrap());

    let identities = manager.file_identities.read().unwrap();
    assert_eq!(identities.len(), MAX_FILE_IDENTITIES);
    assert!(identities.contains_key("recency_0.admission"));
    assert!(!identities.contains_key("recency_1.admission"));
}

// Count actual kernel descriptors in an isolated child, not Rust object counts.
// Both qualified Unix hosts expose /dev/fd. The iterator's own descriptor is
// present in every sample and closes before returning the count.
fn open_descriptors() -> usize {
    fs::read_dir("/dev/fd")
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .unwrap()
        })
        .count()
}
struct JoinedChild(Child);
impl Drop for JoinedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn final_owner_release_closes_actual_descriptors_with_store_alive() {
    let mut child = JoinedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "authority_fences::tests::admissions::idle_descriptor_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("CONNECT_AUTHORITY_IDLE_CHILD", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "isolated descriptor regression failed");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "isolated descriptor owner did not join"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "executed only by the isolated descriptor parent regression"]
fn idle_descriptor_process() {
    assert_eq!(std::env::var("CONNECT_AUTHORITY_IDLE_CHILD").unwrap(), "1");
    let f = Fixture::new();
    let baseline = open_descriptors();
    let scope = AuthorityScope::identity("descriptor_owner", AuthorityMode::Read).unwrap();
    let first = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    assert_eq!(
        open_descriptors(),
        baseline + 1,
        "read admission retained an idle descriptor"
    );
    let second = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    assert_eq!(
        open_descriptors(),
        baseline + 1,
        "overlapping readers did not share their live resource"
    );
    drop(first);
    assert_eq!(open_descriptors(), baseline + 1);
    drop(second);
    assert_eq!(
        open_descriptors(),
        baseline,
        "final reader kept an open description"
    );

    let a = AuthorityScope::identity("descriptor_a", AuthorityMode::Write).unwrap();
    let b = AuthorityScope::identity("descriptor_b", AuthorityMode::Write).unwrap();
    let mut read_b = b.clone();
    read_b.mode = AuthorityMode::Read;
    let held = f.store.try_acquire_authority(&[read_b]).unwrap();
    let held_count = open_descriptors();
    assert_eq!(held_count, baseline + 1);
    for _ in 0..3 {
        assert_eq!(
            f.store
                .try_acquire_authority(&[a.clone(), b.clone()])
                .unwrap_err()
                .code,
            "authority-busy"
        );
        assert_eq!(
            open_descriptors(),
            held_count,
            "partial acquisition retained a descriptor"
        );
    }
    assert_eq!(
        f.store
            .acquire_authority(
                &[a.clone(), b.clone()],
                &AuthorityWait::new(Duration::from_millis(15))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert_eq!(
        open_descriptors(),
        held_count,
        "deadline retained partial descriptors"
    );
    let wait = AuthorityWait::new(Duration::from_secs(2));
    let cancel = wait.cancel_on_drop();
    let store = f.store.clone();
    let pending_a = a.clone();
    let pending_b = b.clone();
    let worker =
        std::thread::spawn(move || store.acquire_authority(&[pending_a, pending_b], &wait));
    waiting(&f.store, &b);
    drop(cancel);
    assert_eq!(worker.join().unwrap().unwrap_err().code, "cancelled");
    assert_eq!(
        open_descriptors(),
        held_count,
        "cancelled original retained partial descriptors"
    );
    drop(held);
    assert_eq!(open_descriptors(), baseline);
    let writer = f.store.try_acquire_authority(&[a, b]).unwrap();
    assert_eq!(
        open_descriptors(),
        baseline + 4,
        "write guards must own both resource and admission"
    );
    drop(writer);
    assert_eq!(
        open_descriptors(),
        baseline,
        "writer retirement retained descriptors"
    );

    for index in 0..MAX_FILE_IDENTITIES + 8 {
        let scope =
            AuthorityScope::identity(&format!("descriptor_bulk_{index}"), AuthorityMode::Read)
                .unwrap();
        drop(f.store.try_acquire_authority(&[scope]).unwrap());
        assert_eq!(
            open_descriptors(),
            baseline,
            "inactive namespace accumulated descriptors"
        );
    }
    // Rejections also close their original opens, and an unchanged refusal may
    // not turn into a fresh namespace on its second attempt.
    let scope = AuthorityScope::identity("descriptor_refused", AuthorityMode::Read).unwrap();
    drop(f.store.try_acquire_authority(&[scope.clone()]).unwrap());
    let path = f
        .store
        .authority
        .directory
        .join(name(&f, &scope, "resource"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(
        f.store.try_acquire_authority(&[scope]).unwrap_err().code,
        "authority-unavailable"
    );
    assert_eq!(
        open_descriptors(),
        baseline,
        "refused descriptor remained open"
    );
    // The Store is deliberately still usable; no namespace or server restart
    // may supply the descriptor restoration observed above.
    assert!(f.store.credential_metadata(&f.user).is_ok());
    assert_eq!(open_descriptors(), baseline);
}

#[test]
fn fresh_admission_preserves_queued_writer_and_releases_partial_read_scopes() {
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

#[test]
fn cold_identity_publication_rechecks_a_concurrent_replacement() {
    let f = Fixture::new();
    let manager = f.store.authority.clone();
    let name = "cold_identity_race.admission";
    let (entered, entry) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let resumed = Mutex::new(resumed);
    let publications = std::sync::atomic::AtomicUsize::new(0);
    *manager.file_identity_probe.lock().unwrap() = Some(Arc::new(move |target, stage| {
        if target == name && stage == 1 && publications.fetch_add(1, Ordering::Relaxed) == 1 {
            // The original file passed fresh validation and the shared check
            // found no entry. Pause immediately before exclusive publication.
            entered.send(()).unwrap();
            resumed
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
    }));
    let worker_manager = manager.clone();
    let worker = std::thread::spawn(move || worker_manager.file(name));
    entry.recv_timeout(Duration::from_secs(1)).unwrap();
    let path = manager.directory.join(name);
    let old = manager.directory.join("cold_identity_race_original");
    fs::rename(&path, &old).unwrap();
    replace_private_file(&path);
    let current = manager.file(name).unwrap();
    resume.send(()).unwrap();
    let error = worker.join().unwrap().unwrap_err();
    *manager.file_identity_probe.lock().unwrap() = None;

    assert_eq!(error.code, "authority-unavailable");
    assert_eq!(error.message, "Authority file changed");
    let identity = manager
        .lock_file_identity(&current.metadata().unwrap())
        .unwrap();
    let original = manager
        .lock_file_identity(&fs::metadata(&old).unwrap())
        .unwrap();
    assert!(identity != original);
    assert!(
        manager
            .file_identities
            .read()
            .unwrap()
            .get(name)
            .unwrap()
            .identity
            == identity
    );
    drop(
        manager
            .file(name)
            .expect("stale open overwrote the newer namespace identity"),
    );
}
