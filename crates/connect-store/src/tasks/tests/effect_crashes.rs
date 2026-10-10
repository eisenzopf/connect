//! Actual child termination proves the commit/ACK boundary, not a simulated error.
use super::{effect_helpers::*, fixture::Fixture, grants, replies};
use crate::{Limits, Store};
use connect_protocol::tasks::*;
use std::process::Command;

fn run(mut child: Command) -> std::process::Output {
    let mut process = child
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while process.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            process.kill().unwrap();
            process.wait().unwrap();
            panic!("Crash probe exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    process.wait_with_output().unwrap()
}

#[test]
fn child_operation() {
    let Ok(path) = std::env::var("CONNECT_TASK_CRASH_DATABASE") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&path), Limits::default()).unwrap();
    let request: TaskRequest =
        serde_json::from_str(&std::env::var("CONNECT_TASK_CRASH_INPUT").unwrap()).unwrap();
    let key = std::env::var("CONNECT_TASK_CRASH_CREDENTIAL").unwrap();
    if let Ok(origin) = std::env::var("CONNECT_TASK_CRASH_BINDING") {
        let binding: InvocationBinding = serde_json::from_str(&origin).unwrap();
        let definition: GrantDefinition =
            serde_json::from_str(&std::env::var("CONNECT_TASK_CRASH_GRANT").unwrap()).unwrap();
        let grant = store
            .issue_task_grant(&key, "crash-grant", &definition)
            .unwrap();
        let handle = store
            .authenticate_task_grant(grant.expose_secret(), &binding)
            .unwrap();
        store.execute_task_invocation(&handle, &request).unwrap();
    } else {
        store.execute_task(&key, &request).unwrap();
    }
    panic!("The selected commit hook did not terminate the child");
}
#[test]
fn terminated_owner_and_grant_processes_preserve_effect_identity_before_and_after_commit() {
    for grant_mode in [false, true] {
        for (stage, committed) in [("task-before-commit", false), ("task-after-commit", true)] {
            let f = Fixture::new();
            let task = f.task();
            let request = send(&f, &task, "crash-send", "Survive crash", false);
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "tasks::tests::effect_crashes::child_operation",
                    "--nocapture",
                ])
                .env("CONNECT_TASK_CRASH_DATABASE", f.store.path())
                .env("CONNECT_TASK_CRASH_CREDENTIAL", &f.key.id)
                .env(
                    "CONNECT_TASK_CRASH_INPUT",
                    serde_json::to_string(&request).unwrap(),
                )
                .env("CONNECT_TEST_FAULT_STAGE", stage)
                .env("CONNECT_TEST_FAULT_REQUEST", "crash-send");
            if grant_mode {
                let origin = grants::binding("crash-native", InvocationKind::Native);
                f.store
                    .register_task_binding(&f.key.id, &task.id, &origin)
                    .unwrap();
                child
                    .env(
                        "CONNECT_TASK_CRASH_BINDING",
                        serde_json::to_string(&origin).unwrap(),
                    )
                    .env(
                        "CONNECT_TASK_CRASH_GRANT",
                        serde_json::to_string(&grants::definition(&task, origin, f.now)).unwrap(),
                    );
            }
            let result = run(child);
            assert_eq!(
                result.status.code(),
                Some(86),
                "{stage}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(count(&f, "messages"), u64::from(committed));
            assert_eq!(count(&f, "task_effects"), u64::from(committed));
            // Owner inspection/replay uses its own current authority; the saved
            // original grant remains available for later protected dispatch.
            let replay = f.run(&request).unwrap();
            assert!(matches!(
                replay,
                TaskOutcome::Tool(TaskToolResult::Completed { .. })
            ));
            assert_eq!(count(&f, "messages"), 1);
            assert_eq!(count(&f, "task_effects"), 1);
            assert_eq!(count(&f, "task_effect_authority"), 1);
        }
    }
}

#[test]
fn terminated_recipient_preserves_reply_message_binding_and_wake_before_ack() {
    for (stage, committed) in [("task-before-commit", false), ("task-after-commit", true)] {
        let (f, human, identity) = replies::setup();
        let task = replies::task(&f, &identity, "task");
        let ask = send(&f, &task, "question", "Choose a draft", true);
        f.run(&ask).unwrap();
        let connect_protocol::Outcome::Accepted { message_id, .. } =
            effect(&f, &task).outcome.unwrap().as_ref().clone()
        else {
            panic!("question")
        };
        let request = TaskRequest::new(TaskCommand::Reply {
            request_id: "crash-reply".into(),
            conversation_id: f.cid.clone(),
            reply_to: message_id,
            body: "Choose B".into(),
        });
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "tasks::tests::effect_crashes::child_operation",
                "--nocapture",
            ])
            .env("CONNECT_TASK_CRASH_DATABASE", f.store.path())
            .env("CONNECT_TASK_CRASH_CREDENTIAL", &human.id)
            .env(
                "CONNECT_TASK_CRASH_INPUT",
                serde_json::to_string(&request).unwrap(),
            )
            .env("CONNECT_TEST_FAULT_STAGE", stage)
            .env("CONNECT_TEST_FAULT_REQUEST", "crash-reply");
        let result = run(child);
        assert_eq!(
            result.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(count(&f, "task_replies"), u64::from(committed));
        assert_eq!(count(&f, "messages"), 1 + u64::from(committed));
        assert_eq!(
            Fixture::record(f.run(&grants::get(&task)).unwrap()).state,
            if committed {
                TaskState::Ready
            } else {
                TaskState::Waiting
            }
        );
        let receipt = f.store.execute_task(&human.id, &request).unwrap();
        assert_eq!(f.store.execute_task(&human.id, &request).unwrap(), receipt);
        assert_eq!(count(&f, "task_replies"), 1);
        assert_eq!(count(&f, "messages"), 2);
        assert!(matches!(
            f.run(&ask).unwrap(),
            TaskOutcome::Tool(TaskToolResult::Completed { .. })
        ));
    }
}
