//! Local snapshot/ownership repair cases do not consume or dispatch app work.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::task_runner::{commands::Lane, TaskStore};
use connect_client::tasks::TaskCommand;

fn binding() -> Binding {
    Binding {
        task_id: "task".into(),
        owner_id: "owner".into(),
        conversation_id: "conversation".into(),
    }
}
#[test]
fn stopped_pending_work_and_owned_lock_are_distinct_and_inspection_never_claims_a_worker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite3");
    assert_eq!(
        local(None, &binding()).unwrap().state,
        WorkerState::Unconfigured
    );
    assert_eq!(
        local(Some(&path), &binding()).unwrap().state,
        WorkerState::NotStarted
    );
    assert!(!path.exists() && !path.with_extension("task.lock").exists());
    let store = TaskStore::open(&path, binding()).unwrap();
    store
        .prepare(
            "saved-command",
            Lane::Controller,
            TaskCommand::Get {
                task_id: "task".into(),
            },
        )
        .unwrap();
    store
        .connection
        .execute("INSERT INTO runner_metadata VALUES('sequence','7')", [])
        .unwrap();
    store
        .connection
        .execute("INSERT INTO runner_events VALUES('event',7)", [])
        .unwrap();
    let running = local(Some(&path), &binding()).unwrap();
    assert_eq!(running.state, WorkerState::LockHeld);
    assert_eq!(running.checkpoint_sequence, Some(7));
    assert_eq!(running.statistics.unwrap().pending_commands, 1);
    drop(store);
    let before = std::fs::read(&path).unwrap();
    let stopped = local(Some(&path), &binding()).unwrap();
    assert_eq!(stopped.state, WorkerState::StoppedWithPendingWork);
    assert_eq!(stopped.statistics.unwrap().business_effects, 0);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let store = TaskStore::open(&path, binding()).unwrap();
    assert_eq!(
        store.command("saved-command").unwrap().unwrap().command,
        TaskCommand::Get {
            task_id: "task".into()
        }
    );
    drop(store);
    let changed = Binding {
        task_id: "other".into(),
        ..binding()
    };
    assert!(local(Some(&path), &changed).is_err());
}
#[test]
fn incompatible_or_nonprivate_state_is_preserved_and_never_reported_as_not_started() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite3");
    let store = TaskStore::open(&path, binding()).unwrap();
    store
        .connection
        .pragma_update(None, "user_version", 2)
        .unwrap();
    drop(store);
    let before = std::fs::read(&path).unwrap();
    assert!(local(Some(&path), &binding()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 1).unwrap();
    for value in ["-1", "bad", "01", "0000000000000000000000001", "8"] {
        c.execute("INSERT INTO runner_metadata VALUES('sequence',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value]).unwrap();
        assert!(local(Some(&path), &binding()).is_err());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(local(Some(&path), &binding()).is_err());
        let link = directory.path().join("alias.sqlite3");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(local(Some(&link), &binding()).is_err());
    }
}
