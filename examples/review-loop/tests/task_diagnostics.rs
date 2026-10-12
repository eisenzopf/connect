//! Independent public-SDK views after a real app crash and missing notification.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/task_fixture.rs"]
mod fixture;
use connect_client::{
    task_diagnostics::{DiagnosticObservation, DiagnosticSection},
    tasks::*,
};
use connect_review_loop::task_runner::{
    diagnostics::{self, WorkerState},
    TaskStore,
};
use connect_store::Store;
use fixture::{exited, stop, Fixture};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crashed_worker_pending_intent_is_visible_without_restarting_or_consuming_it() {
    let mut f = Fixture::start().await;
    let config = f.directory.join("invocation.toml");
    let not_started = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(not_started.runner.state, WorkerState::NotStarted);
    assert!(!f.state().exists());
    let mut app = f.spawn(Some("before-dispatch"), None);
    exited(&mut app, 86).await;
    let before = Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap();
    let app_before = std::fs::read(f.state()).unwrap();
    let report = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(report.runner.state, WorkerState::StoppedWithPendingWork);
    assert_eq!(
        report.execution_readiness.state,
        connect_client::ReadinessState::Unavailable
    );
    assert_eq!(
        report.runner.readiness().reason,
        "original-worker-recovery-required"
    );
    assert_eq!(
        report.runner.statistics.as_ref().unwrap().pending_commands,
        1
    );
    assert_eq!(report.task.task.state, TaskState::Ready);
    let output = cli(&f).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["runner"]["state"], "stopped-with-pending-work");
    assert_eq!(value["execution_readiness"]["state"], "unavailable");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("Please review draft") && !text.contains(&f.owner_config.credential));
    assert_eq!(
        Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap(),
        before
    );
    assert_eq!(std::fs::read(f.state()).unwrap(), app_before);
    f.server
        .as_ref()
        .unwrap()
        .service
        .revoke_task_grant(f.owner_config.credential_id.clone(), f.grant_id.clone())
        .await
        .unwrap();
    assert!(
        diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(f.state()).unwrap(), app_before);
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_waiting_worker_catches_up_reply_after_server_restart_and_never_claims_completion()
{
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None, None);
    let question = f.question().await;
    f.settled(0).await;
    let config = f.directory.join("controller.toml");
    let running = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(running.runner.state, WorkerState::LockHeld);
    assert_eq!(
        running.execution_readiness.state,
        connect_client::ReadinessState::Uncertain
    );
    assert_eq!(running.task.task.state, TaskState::Waiting);
    stop(&mut app).await;
    // SIGINT may interrupt the next journal page after the earlier settled
    // observation. Inspect the now-stopped worker's actual durable state;
    // shutdown deliberately preserves pending work rather than discarding it.
    let stats = TaskStore::inspect(&f.state()).unwrap();
    let expected = if stats.pending_commands > 0 || stats.page_pending {
        WorkerState::StoppedWithPendingWork
    } else {
        WorkerState::Stopped
    };
    let app_before = std::fs::read(f.state()).unwrap();
    let stopped = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(stopped.runner.state, expected);
    assert_eq!(
        serde_json::to_value(stopped.runner.statistics.as_ref().unwrap()).unwrap(),
        serde_json::to_value(&stats).unwrap()
    );
    assert_eq!(
        stopped.execution_readiness.state,
        connect_client::ReadinessState::Unavailable
    );
    let checkpoint = stopped.runner.checkpoint_sequence.unwrap();
    f.restart().await;
    f.answer(&question).await;
    let before = Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap();
    let report = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(report.task.task.state, TaskState::Ready);
    assert_eq!(report.runner.state, expected);
    assert_eq!(report.runner.statistics.unwrap().business_effects, 0);
    let DiagnosticSection::Available(journal) = report.task.journal else {
        panic!("journal");
    };
    assert!(journal.items.iter().any(|item| item.sequence > checkpoint && matches!(&item.observation, DiagnosticObservation::Reply(reply) if !reply.consumed && reply.task_id.as_deref() == Some(&f.task.id))));
    assert_eq!(
        Store::inspect_snapshot(f.server.as_ref().unwrap().store.path()).unwrap(),
        before
    );
    assert_eq!(std::fs::read(f.state()).unwrap(), app_before);
    app = f.spawn(None, None);
    f.settled(1).await;
    stop(&mut app).await;
    let resumed = diagnostics::inspect(&config, &f.task.id, Some(&f.state()), 100)
        .await
        .unwrap();
    assert_eq!(resumed.runner.statistics.unwrap().business_effects, 1);
    f.finish().await;
}

async fn cli(f: &Fixture) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_connect-example"))
            .arg("task-diagnostics")
            .arg("--config")
            .arg(f.directory.join("invocation.toml"))
            .arg("--task-id")
            .arg(&f.task.id)
            .arg("--state")
            .arg(f.state())
            .env_clear()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}
