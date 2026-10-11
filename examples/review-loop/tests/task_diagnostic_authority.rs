//! macOS-owned FD observation proves revocation after authentication, before report.
#![cfg(all(feature = "test-support", target_os = "macos"))]
#![allow(clippy::unwrap_used)]
#[path = "support/task_fixture.rs"]
mod fixture;
use connect_store::Store;
use fixture::{exited, Fixture};
use rusqlite::Connection;
use std::{process::Stdio, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_revocation_during_local_snapshot_denies_all_report_output() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(Some("before-dispatch"), None);
    exited(&mut app, 86).await;
    let database = Connection::open(f.state()).unwrap();
    database
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    let intent: String = database
        .query_row(
            "SELECT command FROM runner_commands ORDER BY sequence LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    database.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect-example"))
        .arg("task-diagnostics")
        .arg("--config")
        .arg(f.directory.join("invocation.toml"))
        .arg("--task-id")
        .arg(&f.task.id)
        .arg("--state")
        .arg(f.state())
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap().to_string();
    let path = std::fs::canonicalize(f.state()).unwrap();
    // The production inspector opens this app DB only after authenticated task
    // lookup. Observe the owned child's exact FD, never another user's processes.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "diagnostic client exited before its snapshot could be paused"
            );
            let observation = tokio::process::Command::new("/usr/sbin/lsof")
                .args(["-a", "-p", &pid, "-Fn", "--"])
                .arg(&path)
                .env_clear()
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            let expected = format!("n{}", path.display());
            if observation.status.success()
                && String::from_utf8(observation.stdout)
                    .unwrap()
                    .lines()
                    .any(|line| line == expected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(child.try_wait().unwrap().is_none());
    f.server
        .as_ref()
        .unwrap()
        .service
        .revoke_task_grant(f.owner_config.credential_id.clone(), f.grant_id.clone())
        .await
        .unwrap();
    let revoked = Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap();
    database.execute_batch("ROLLBACK").unwrap();
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success() && output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("forbidden"));
    let retained: String = database
        .query_row(
            "SELECT command FROM runner_commands ORDER BY sequence LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, intent);
    assert_eq!(
        Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap(),
        revoked
    );
    f.finish().await;
}
