//! Retained owner reads never revive the original continuation authority.
use super::{effect_helpers, fixture::Fixture, recall_helpers};
use crate::{IssuedCredential, Scope};
use connect_protocol::{tasks::*, Operation};

fn setup() -> (Fixture, TaskRecord, TaskRequest, IssuedCredential) {
    let f = Fixture::new();
    let task = f.task();
    let send = effect_helpers::send(&f, &task, "original-effect", "Retained fact", false);
    f.run(&send).unwrap();
    let inspector = f
        .store
        .issue(&f.owner, Scope::operator(), f.now + 7_200)
        .unwrap();
    (f, task, send, inspector)
}

fn inspect(f: &Fixture, task: &TaskRecord, credential: &str, timestamp: i64) {
    let commands = [
        TaskCommand::Inbox {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
            wait_ms: 0,
        },
        TaskCommand::Wait {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
            wait_ms: 0,
        },
    ];
    for command in commands {
        let TaskOutcome::Inbox { events, .. } = f
            .store
            .execute_task_at(credential, &TaskRequest::new(command), timestamp)
            .unwrap()
        else {
            panic!("retained journal")
        };
        let effect = events
            .into_iter()
            .find_map(|event| match event.data {
                TaskEventData::Effect(effect) => Some(effect),
                _ => None,
            })
            .unwrap();
        let TaskOutcome::Effect(effect) = f
            .store
            .execute_task_at(
                credential,
                &TaskRequest::new(TaskCommand::EffectGet {
                    task_id: task.id.clone(),
                    effect_id: effect.id,
                }),
                timestamp,
            )
            .unwrap()
        else {
            panic!("discovered retained effect")
        };
        assert_eq!(effect.state, EffectState::Succeeded);
    }
    let TaskOutcome::Recall { sources, .. } = f
        .store
        .execute_task_at(
            credential,
            &recall_helpers::request(task, None, 100),
            timestamp,
        )
        .unwrap()
    else {
        panic!("retained sources")
    };
    assert!(sources
        .iter()
        .any(|source| source.body.as_deref() == Some("Retained fact")));
}

#[test]
fn current_owner_discovers_retained_effects_after_backing_revoke_without_continuing() {
    let (f, task, send, inspector) = setup();
    let (_, invocation) = recall_helpers::grant(&f, &task);
    f.store
        .revoke(&f.store.authenticate(&inspector.token).unwrap(), &f.key.id)
        .unwrap();
    inspect(&f, &task, &inspector.id, f.now);
    assert_eq!(
        f.store
            .execute_task_at(&inspector.id, &send, f.now)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let fresh = effect_helpers::send(&f, &task, "new-effect", "Do not dispatch", false);
    assert_eq!(
        f.store
            .execute_task_at(&inspector.id, &fresh, f.now)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    assert!(f
        .store
        .execute_task_invocation_at(
            &invocation,
            &recall_helpers::request(&task, None, 100),
            f.now
        )
        .is_err());
    assert_eq!(
        f.store
            .execute_task_at(&f.key.id, &recall_helpers::request(&task, None, 100), f.now)
            .unwrap_err()
            .code,
        "unauthorized"
    );
}

#[test]
fn current_owner_inspects_expired_backing_and_task_without_replaying_an_effect() {
    let (f, task, send, inspector) = setup();
    let timestamp = f.now + 3_600;
    let expired = Fixture::record(
        f.store
            .execute_task_at(
                &inspector.id,
                &TaskRequest::new(TaskCommand::Get {
                    task_id: task.id.clone(),
                }),
                timestamp,
            )
            .unwrap(),
    );
    assert_eq!(expired.state, TaskState::Expired);
    inspect(&f, &task, &inspector.id, timestamp);
    assert_eq!(
        f.store
            .execute_task_at(&inspector.id, &send, timestamp)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn terminal_tasks_and_closed_conversations_retain_authorized_owner_journal_and_sources() {
    for terminal in [
        TaskState::Completed,
        TaskState::Cancelled,
        TaskState::Failed,
    ] {
        let (f, task, send, inspector) = setup();
        let running = Fixture::record(
            f.run(&TaskRequest::new(TaskCommand::Get {
                task_id: task.id.clone(),
            }))
            .unwrap(),
        );
        f.run(&Fixture::transition(&running, "terminal", terminal))
            .unwrap();
        f.store
            .execute(
                &inspector.id,
                &Operation::Close {
                    request_id: "close-conversation".into(),
                    conversation_id: f.cid.clone(),
                    options: Default::default(),
                },
            )
            .unwrap();
        inspect(&f, &task, &inspector.id, f.now);
        // Closed conversation/terminal inspection cannot produce another effect.
        let fresh = effect_helpers::send(&f, &task, "new-effect", "Do not dispatch", false);
        assert!(f
            .store
            .execute_task_at(&inspector.id, &fresh, f.now)
            .is_err());
        if terminal != TaskState::Completed {
            assert!(f
                .store
                .execute_task_at(&inspector.id, &send, f.now)
                .is_err());
        }
    }
}

#[test]
fn retained_inspection_still_requires_current_reader_scope_owner_membership_and_lifetime() {
    let (f, task, _, inspector) = setup();
    let source = recall_helpers::request(&task, None, 100);
    let no_read = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_recall".into()],
                recipients: vec![f.owner.clone()],
            },
            f.now + 3_600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_at(&no_read.id, &source, f.now)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let foreign = f.store.create_identity("foreign-inspector").unwrap();
    let foreign = f
        .store
        .issue(&foreign.id, Scope::operator(), f.now + 3_600)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_at(&foreign.id, &source, f.now)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_task_at(&inspector.id, &source, f.now + 7_200)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE conversation_id=?1",
            [&f.cid],
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_at(&inspector.id, &source, f.now)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
