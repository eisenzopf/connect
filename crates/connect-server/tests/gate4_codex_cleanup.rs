//! Owned process-group cleanup preserves task intent and unrelated processes.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_fixture.rs"]
#[allow(dead_code)]
mod host;
use connect_protocol::tasks::TaskPermission;
use connect_server::cli::task_access::codex_host;
use connect_store::Store;
use serde::Deserialize;
use std::{path::Path, time::Duration};
use tokio::{net::TcpStream, process::Command};

// Leave room for cold interpreter startup while retaining one absolute,
// explicitly configured deadline. Outer observation includes CLI startup.
const TURN_SECONDS: u64 = 8;
const STARTUP_SECONDS: u64 = 20;
const CLEANUP_SECONDS: u64 = 3;

#[derive(Deserialize)]
struct Descendant {
    pid: u32,
    group: u32,
    parent: u32,
    port: u16,
}
async fn observe(path: &Path) -> Descendant {
    tokio::time::timeout(Duration::from_secs(STARTUP_SECONDS), async {
        loop {
            if let Ok(bytes) = std::fs::read(path) {
                return serde_json::from_slice(&bytes).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let parent = path.parent().unwrap();
        let phase = std::fs::read_to_string(parent.join("fixture-phase"))
            .unwrap_or_else(|_| "not-entered".into());
        panic!(
            "Fixture marker {} unavailable; host phase={phase}",
            path.file_name().unwrap().to_string_lossy()
        )
    })
}
async fn listener_alive(child: &Descendant) -> bool {
    TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, child.port))
        .await
        .is_ok()
}
async fn gone(child: &Descendant) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            // A closed listening socket proves the owned resource is gone;
            // checking the PID too distinguishes shutdown from a living worker.
            let mut processes_gone = true;
            for pid in [child.pid, child.parent] {
                let status = Command::new("/bin/ps")
                    .args(["-p", &pid.to_string(), "-o", "stat="])
                    .output()
                    .await
                    .unwrap();
                processes_gone &= !status.status.success() || status.stdout.is_empty();
            }
            if !listener_alive(child).await && processes_gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

async fn exercise(mode: &str, abort: bool) {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::Inbox],
    )
    .await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, mode);
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    // A separate owned group is a live negative control for overbroad cleanup.
    let neighbor_path = f.directory().join("neighbor.json");
    let mut neighbor = Command::new("python3")
        .arg(f.directory().join("codex-descendant.py"))
        .arg(&neighbor_path)
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let control = observe(&neighbor_path).await;
    assert_eq!(control.pid, neighbor.id().unwrap());
    assert_eq!(control.group, control.pid);
    let workdir = dir.clone();
    let started = tokio::time::Instant::now();
    let running = tokio::spawn(async move {
        codex_host::run(
            &workdir,
            &binary,
            Some(&workdir.join("prompt.txt")),
            TURN_SECONDS,
        )
        .await
    });
    let child = observe(&dir.join("descendant.json")).await;
    assert_eq!(child.group, child.parent);
    assert_ne!(child.pid, child.group);
    assert_ne!(child.group, control.group);
    // The invalid stream may already have been cleaned before observation.
    if mode != "descendant-invalid" {
        assert!(listener_alive(&child).await);
    }
    let saved = host::state(&dir);
    assert_eq!(saved["completed"], false);
    assert!(saved["session_id"].is_string());
    if abort {
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
    } else {
        let error = running.await.unwrap().unwrap_err().to_string();
        assert!(
            error.contains(if mode == "descendant-invalid" {
                "Invalid Codex JSONL"
            } else {
                "deadline"
            }),
            "{error}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(TURN_SECONDS + CLEANUP_SECONDS),
        "Host deadline restarted or cleanup exceeded its bound"
    );
    gone(&child).await;
    assert!(listener_alive(&control).await);
    assert!(neighbor.try_wait().unwrap().is_none());
    assert_eq!(host::state(&dir), saved);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    // The installation lock is released. Original explicit resume, not a new
    // session/task/run, is usable after all owned resources have stopped.
    host::mode(&dir, "complete");
    let binary = f.directory().join("fake-codex");
    codex_host::run(&dir, &binary, None, 15).await.unwrap();
    let resumed = host::state(&dir);
    for key in ["session_id", "task_id", "run_id", "prompt"] {
        assert_eq!(resumed[key], saved[key]);
    }
    neighbor.kill().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn timeout_stops_inherited_group_and_preserves_another_worker() {
    exercise("descendant-stall", false).await;
}
#[tokio::test]
async fn aborting_caller_signals_owned_group_before_releasing_lock() {
    exercise("descendant-stall", true).await;
}
#[tokio::test]
async fn invalid_stream_reaps_parent_and_stops_inherited_worker() {
    exercise("descendant-invalid", false).await;
}
#[tokio::test]
async fn exited_parent_with_inherited_stdout_cannot_escape_deadline() {
    exercise("descendant-exit", false).await;
}

#[tokio::test]
async fn fresh_launcher_reaps_group_before_returning_deadline_failure() {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::Inbox],
    )
    .await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "descendant-stall");
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let launcher = Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(["task-access", "run-codex", "--installation"])
        .arg(&dir)
        .arg("--codex-binary")
        .arg(&binary)
        .args([
            "--timeout-seconds",
            &TURN_SECONDS.to_string(),
            "--prompt-file",
        ])
        .arg(dir.join("prompt.txt"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let child = observe(&dir.join("descendant.json")).await;
    let saved = host::state(&dir);
    let result = tokio::time::timeout(
        Duration::from_secs(TURN_SECONDS + CLEANUP_SECONDS),
        launcher.wait_with_output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("deadline"));
    gone(&child).await;
    assert_eq!(host::state(&dir), saved);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    host::mode(&dir, "complete");
    assert!(host::cli(&dir, &binary, false).await.status.success());
    for key in ["session_id", "task_id", "run_id", "prompt"] {
        assert_eq!(host::state(&dir)[key], saved[key]);
    }
    f.close().await;
}

#[tokio::test]
async fn confirmed_normal_exit_also_stops_a_worker_that_closed_its_output_pipe() {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::Inbox],
    )
    .await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "descendant-complete");
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), TURN_SECONDS)
        .await
        .unwrap();
    let child = observe(&dir.join("descendant.json")).await;
    gone(&child).await;
    let state = host::state(&dir);
    assert_eq!(state["completed"], true);
    assert!(state["session_id"].is_string());
    assert_eq!(state["task_id"], f.task.id);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
