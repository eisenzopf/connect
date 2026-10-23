#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::Limits;
use std::{
    io::{BufRead, Write},
    process::{Command, Stdio},
};
fn initialize() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    (dir, store)
}

#[test]
fn ordinary_handles_share_owner_but_exclusive_runtime_and_foreign_proofs_refuse() {
    let (_dir, store) = initialize();
    let owner = store.require_runtime_owner().unwrap();
    let peer = Store::open(store.path(), Limits::default()).unwrap();
    assert!(Arc::ptr_eq(&owner, &peer.require_runtime_owner().unwrap()));
    assert_eq!(
        InstallationOwnership::acquire(store.path())
            .unwrap_err()
            .code,
        "installation-busy"
    );
    assert_eq!(
        Store::open_for_recovery(store.path(), Limits::default())
            .err()
            .unwrap()
            .code,
        "installation-busy"
    );
    let (_other_dir, other) = initialize();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    assert_eq!(
        Store::open_with_ownership(
            store.path(),
            Limits::default(),
            &other.require_runtime_owner().unwrap()
        )
        .err()
        .unwrap()
        .code,
        "authority-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
    assert_eq!(format!("{owner:?}"), "InstallationOwnership(redacted)");
    let path = store.path().to_owned();
    drop(owner);
    drop(store);
    assert_eq!(
        InstallationOwnership::acquire(&path).unwrap_err().code,
        "installation-busy"
    );
    drop(peer);
    InstallationOwnership::acquire(&path).unwrap();
}

#[test]
fn retired_runtime_store_cannot_revive_and_active_sql_retains_kernel_ownership() {
    let (_dir, store) = initialize();
    let path = store.path().to_owned();
    drop(store);
    let owner = InstallationOwnership::acquire(&path).unwrap();
    let borrowed = Arc::new(Store::open_with_ownership(&path, Limits::default(), &owner).unwrap());
    let connection = borrowed.connection().unwrap();
    owner.retire();
    drop(owner);
    assert_eq!(borrowed.installation_id().unwrap_err().code, "forbidden");
    assert_eq!(
        Store::open(&path, Limits::default()).err().unwrap().code,
        "installation-busy"
    );
    // The already admitted SQL unit still owns the installation after retirement.
    connection
        .execute(
            "INSERT INTO identities(id,alias,capabilities) VALUES('settled','settled','[]')",
            [],
        )
        .unwrap();
    drop(connection);
    let next = InstallationOwnership::acquire(&path).unwrap();
    let current = Store::open_with_ownership(&path, Limits::default(), &next).unwrap();
    assert_eq!(
        current
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM identities WHERE id='settled'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(borrowed.installation_id().unwrap_err().code, "forbidden");
}

#[test]
fn ownership_namespace_unifies_aliases_and_different_extensions_do_not_collide() {
    let (dir, store) = initialize();
    let alias = dir.path().join("alias.sqlite3");
    #[cfg(unix)]
    std::os::unix::fs::symlink(store.path(), &alias).unwrap();
    let peer = Store::open(&alias, Limits::default()).unwrap();
    assert!(Arc::ptr_eq(
        &store.require_runtime_owner().unwrap(),
        &peer.require_runtime_owner().unwrap()
    ));
    let different = Store::initialize(&dir.path().join("state.db"), Limits::default()).unwrap();
    assert_ne!(
        store.require_runtime_owner().unwrap().lock_path,
        different.require_runtime_owner().unwrap().lock_path
    );
    let hardlink = dir.path().join("hard.sqlite3");
    fs::hard_link(store.path(), &hardlink).unwrap();
    assert_eq!(
        Store::open(&hardlink, Limits::default())
            .err()
            .unwrap()
            .code,
        "authority-unavailable"
    );
    fs::remove_file(&hardlink).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&store.require_runtime_owner().unwrap().lock_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn unsafe_or_replaced_lock_namespace_never_proves_ownership() {
    for unsafe_kind in ["symlink", "hardlink", "mode"] {
        let (dir, store) = initialize();
        let path = store.path().to_owned();
        let lock = store.require_runtime_owner().unwrap().lock_path.clone();
        drop(store);
        let saved = dir.path().join("saved-lock");
        fs::rename(&lock, &saved).unwrap();
        match unsafe_kind {
            "symlink" => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&saved, &lock).unwrap();
            }
            "hardlink" => fs::hard_link(&saved, &lock).unwrap(),
            _ => {
                fs::rename(&saved, &lock).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
        }
        assert_eq!(
            InstallationOwnership::acquire(&path).unwrap_err().code,
            "authority-unavailable"
        );
    }
    let (dir, store) = initialize();
    let owner = store.require_runtime_owner().unwrap();
    fs::rename(&owner.lock_path, dir.path().join("old-lock")).unwrap();
    File::create(&owner.lock_path).unwrap();
    assert_eq!(
        store.installation_id().unwrap_err().code,
        "authority-unavailable"
    );
}

fn child(path: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "installation_ownership::tests::process_worker",
            "--nocapture",
        ])
        .env("CONNECT_RUNTIME_OWNER_FIXTURE", path)
        .env("CONNECT_RUNTIME_OWNER_MODE", mode);
    command
}
#[test]
fn actual_child_cannot_adopt_live_parent_but_administration_remains_non_adopting() {
    let (_dir, store) = initialize();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let output = child(store.path(), "refuse").output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}
#[test]
fn actual_owner_process_death_releases_kernel_ownership_despite_stale_file() {
    let (_dir, store) = initialize();
    let path = store.path().to_owned();
    let lock = store.require_runtime_owner().unwrap().lock_path.clone();
    drop(store);
    let mut child = child(&path, "hold")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (ready, waited) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = std::io::BufReader::new(output);
        let mut line = String::new();
        while output.read_line(&mut line).unwrap_or(0) > 0 {
            if line.contains("RUNTIME_OWNER_READY") {
                let _ = ready.send(());
                return;
            }
            line.clear();
        }
    });
    if waited
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        panic!("owner fixture did not become ready within five seconds");
    }
    reader.join().unwrap();
    assert_eq!(
        InstallationOwnership::acquire(&path).unwrap_err().code,
        "installation-busy"
    );
    assert_eq!(
        Store::open(&path, Limits::default()).err().unwrap().code,
        "installation-busy"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(lock.exists());
    let owner = InstallationOwnership::acquire(&path).unwrap();
    Store::open_with_ownership(&path, Limits::default(), &owner).unwrap();
}
#[test]
#[ignore = "subprocess fixture explicitly invoked by ownership refusal and process-death parents"]
fn process_worker() {
    let path = PathBuf::from(std::env::var_os("CONNECT_RUNTIME_OWNER_FIXTURE").unwrap());
    if std::env::var("CONNECT_RUNTIME_OWNER_MODE").unwrap() == "refuse" {
        assert_eq!(
            Store::open(&path, Limits::default()).err().unwrap().code,
            "installation-busy"
        );
        assert_eq!(
            Store::open_for_recovery(&path, Limits::default())
                .err()
                .unwrap()
                .code,
            "installation-busy"
        );
        assert_eq!(
            InstallationOwnership::acquire(&path).unwrap_err().code,
            "installation-busy"
        );
        let admin = Store::open_administration(&path, Limits::default()).unwrap();
        admin.installation_id().unwrap();
        assert_eq!(
            admin.fence_transition_workers(1000).unwrap_err().code,
            "forbidden"
        );
        assert_eq!(
            admin
                .reconcile_interrupted_video_connections()
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(
            admin.fence_unobserved_connections(1000).unwrap_err().code,
            "forbidden"
        );
    } else {
        let _owner = InstallationOwnership::acquire(&path).unwrap();
        println!("RUNTIME_OWNER_READY");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }
}
