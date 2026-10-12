//! Actual child-process exits at SDK/consume/app commit boundaries.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/task_fixture.rs"]
mod fixture;
use connect_client::tasks::*;
use connect_review_loop::task_runner::TaskStore;
use fixture::{exited, stop, Fixture};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_crash_boundary_restarts_original_intents_and_records_one_business_action() {
    for stage in [
        "before-dispatch",
        "after-invocation-ack",
        "after-consume-ack",
        "after-consume",
        "before-app-commit",
        "after-app-commit",
    ] {
        let mut f = Fixture::start().await;
        let mut child;
        if stage.starts_with("before-app")
            || stage.starts_with("after-app")
            || stage.starts_with("after-consume")
        {
            child = f.spawn(None, None);
            let question = f.question().await;
            f.settled(0).await;
            stop(&mut child).await;
            child = f.spawn(Some(stage), None);
            f.answer(&question).await;
        } else {
            child = f.spawn(Some(stage), None);
        }
        exited(&mut child, 86).await;
        let before = TaskStore::inspect(&f.state()).unwrap();
        assert_eq!(
            before.business_effects,
            u64::from(stage == "after-app-commit"),
            "{stage}"
        );
        f.restart().await;
        child = f.spawn(None, None);
        let question = f.question().await;
        f.answer(&question).await; // exact reply replay when the fault follows consumption
        f.settled(1).await;
        stop(&mut child).await;
        child = f.spawn(None, None);
        f.settled(1).await;
        stop(&mut child).await;
        let history = f
            .peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap();
        assert_eq!(
            history
                .messages
                .iter()
                .filter(|m| m.body == "Please review draft-002 and reply to this question.")
                .count(),
            1,
            "{stage}"
        );
        assert_eq!(
            history
                .messages
                .iter()
                .filter(|m| m.body == "Your draft response is recorded.")
                .count(),
            1,
            "{stage}"
        );
        f.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_running_worker_recovers_server_restart_and_catches_up_an_explicit_reply() {
    let mut f = Fixture::start().await;
    let mut child = f.spawn(None, None);
    let question = f.question().await;
    f.settled(0).await;
    f.restart().await;
    f.answer(&question).await;
    f.settled(1).await;
    stop(&mut child).await;
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        3
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn competing_worker_and_unassigned_message_cannot_repeat_or_select_the_business_action() {
    let mut f = Fixture::start().await;
    let mut first = f.spawn(None, None);
    let question = f.question().await;
    f.settled(0).await;
    let mut same_state = f.spawn(None, None);
    exited(&mut same_state, 1).await;
    let mut other_state = f.spawn(None, Some(f.directory.join("competing.sqlite3")));
    exited(&mut other_state, 1).await;
    f.peer
        .as_mut()
        .unwrap()
        .send("unassigned", &f.task.conversation_id, "Approved draft-002.")
        .await
        .unwrap();
    f.settled(0).await;
    f.answer(&question).await;
    f.settled(1).await;
    stop(&mut first).await;
    assert_eq!(
        TaskStore::inspect(&f.directory.join("competing.sqlite3"))
            .unwrap()
            .business_effects,
        0
    );
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        4
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_revocation_denies_restarting_a_saved_workflow_without_owner_fallback() {
    let mut f = Fixture::start().await;
    let mut first = f.spawn(None, None);
    let question = f.question().await;
    f.settled(0).await;
    stop(&mut first).await;
    f.server
        .as_ref()
        .unwrap()
        .service
        .revoke_task_grant(f.owner_config.credential_id.clone(), f.grant_id.clone())
        .await
        .unwrap();
    f.answer(&question).await;
    let mut restarted = f.spawn(None, None);
    exited(&mut restarted, 1).await;
    assert_eq!(TaskStore::inspect(&f.state()).unwrap().business_effects, 0);
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        2
    );
    f.reissue().await;
    let mut renewed = f.spawn(None, None);
    f.settled(1).await;
    stop(&mut renewed).await;
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        3
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_running_task_stops_the_runner_without_a_follow_up() {
    let mut f = Fixture::start().await;
    let mut child = f.spawn(None, None);
    f.question().await;
    f.settled(0).await;
    f.owner
        .as_mut()
        .unwrap()
        .task(TaskCommand::Cancel {
            request_id: "cancel-runner-task".into(),
            task_id: f.task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    exited(&mut child, 1).await;
    assert_eq!(TaskStore::inspect(&f.state()).unwrap().business_effects, 0);
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_controller_token_cannot_be_used_as_the_invocation_configuration() {
    let mut f = Fixture::start().await;
    std::fs::copy(
        f.directory.join("controller.toml"),
        f.directory.join("invocation.toml"),
    )
    .unwrap();
    let mut child = f.spawn(None, None);
    exited(&mut child, 1).await;
    assert!(!f.state().exists());
    assert!(f
        .peer
        .as_mut()
        .unwrap()
        .history(&f.task.conversation_id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    f.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_known_stale_consume_cas_reconciles_with_a_new_preserved_control_intent() {
    let mut f = Fixture::start().await;
    let mut child = f.spawn(None, None);
    let question = f.question().await;
    f.settled(0).await;
    stop(&mut child).await;
    child = f.spawn(Some("before-consume-dispatch"), None);
    f.answer(&question).await;
    exited(&mut child, 86).await;
    let TaskOutcome::Record(current) = f
        .owner
        .as_mut()
        .unwrap()
        .task(TaskCommand::Get {
            task_id: f.task.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("record")
    };
    f.owner
        .as_mut()
        .unwrap()
        .task(TaskCommand::Transition {
            request_id: "concurrent-task-change".into(),
            task_id: f.task.id.clone(),
            expected_revision: current.revision,
            state: TaskState::Running,
            reason: None,
        })
        .await
        .unwrap();
    f.restart().await;
    child = f.spawn(None, None);
    let stats = f.settled(1).await;
    assert_eq!(stats.rejected_commands, 1);
    stop(&mut child).await;
    assert_eq!(
        f.peer
            .as_mut()
            .unwrap()
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        3
    );
    f.finish().await;
}
