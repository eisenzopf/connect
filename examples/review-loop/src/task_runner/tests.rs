//! Real SQLite recovery boundaries and exact persisted intent invariants.
#![allow(clippy::unwrap_used)]
use super::{
    commands::Lane,
    journal::{consume_key, Page},
    Binding, TaskStore,
};
use connect_client::tasks::*;

#[test]
fn the_public_runner_future_can_be_supervised_on_a_tokio_worker() {
    fn send<T: Send>(_: T) {}
    send(super::run(
        super::RunOptions {
            controller_config: "unused-controller".into(),
            invocation_config: "unused-invocation".into(),
            task_id: "unused-task".into(),
            state: "unused-state".into(),
        },
        tokio_util::sync::CancellationToken::new(),
    ));
}

fn binding() -> Binding {
    Binding {
        task_id: "task".into(),
        owner_id: "owner".into(),
        conversation_id: "conversation".into(),
    }
}
fn reply(sequence: u64) -> TaskEvent {
    TaskEvent {
        id: format!("event-{sequence}"),
        task_id: "task".into(),
        actor_id: "peer".into(),
        sequence,
        created_at: 1,
        data: TaskEventData::Reply(TaskReply {
            event_id: "reply".into(),
            conversation_id: "conversation".into(),
            task_id: Some("task".into()),
            reply_to: Some("question".into()),
            assigned_by: Some("peer".into()),
            consumed: false,
        }),
    }
}
fn receipt() -> TaskOutcome {
    TaskOutcome::Record(Box::new(TaskRecord {
        id: "task".into(),
        owner_id: "owner".into(),
        conversation_id: "conversation".into(),
        scope: TaskScope {
            operations: vec![TaskPermission::Send],
            recipients: vec!["peer".into()],
            sms_enrollments: vec![],
        },
        state: TaskState::Ready,
        revision: 2,
        expires_at: 100,
        initiating_event_id: None,
        created_at: 1,
        updated_at: 2,
        reason: None,
    }))
}
fn consume(store: &TaskStore) {
    let key = consume_key("reply");
    store
        .prepare(
            &key,
            Lane::Controller,
            TaskCommand::ConsumeReply {
                request_id: "exact-consume".into(),
                task_id: "task".into(),
                event_id: "reply".into(),
                expected_revision: 1,
            },
        )
        .unwrap();
    store.accepted(&key, &receipt()).unwrap();
}

#[test]
fn page_cannot_advance_business_or_checkpoint_without_its_consume_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TaskStore::open(&dir.path().join("app.sqlite3"), binding()).unwrap();
    let page = Page {
        events: vec![reply(1)],
        next: Some("cursor-1".into()),
    };
    store.stage_page(&page).unwrap();
    assert!(store.commit_page(&page).is_err());
    let stats = store.statistics().unwrap();
    assert_eq!(stats.business_effects, 0);
    assert_eq!(stats.processed_events, 0);
    assert!(!stats.cursor_saved && stats.page_pending);
    consume(&store);
    store.commit_page(&page).unwrap();
    let stats = store.statistics().unwrap();
    assert_eq!(stats.business_effects, 1);
    assert_eq!(stats.processed_events, 1);
    assert_eq!(stats.pending_commands, 1);
    assert!(stats.cursor_saved && !stats.page_pending);
}

#[test]
fn reopening_duplicates_do_not_repeat_business_or_regress_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite3");
    let mut store = TaskStore::open(&path, binding()).unwrap();
    consume(&store);
    let page = Page {
        events: vec![reply(1), reply(2)],
        next: Some("cursor-2".into()),
    };
    store.stage_page(&page).unwrap();
    store.commit_page(&page).unwrap();
    let original = store.pending_invocations().unwrap().remove(0).command;
    drop(store);
    let mut store = TaskStore::open(&path, binding()).unwrap();
    let old = Page {
        events: vec![reply(1)],
        next: Some("old-cursor".into()),
    };
    store.stage_page(&old).unwrap();
    store.commit_page(&old).unwrap();
    assert_eq!(
        store.metadata("cursor").unwrap().as_deref(),
        Some("cursor-2")
    );
    assert_eq!(store.statistics().unwrap().business_effects, 1);
    assert_eq!(
        store.pending_invocations().unwrap().remove(0).command,
        original
    );
}

#[test]
fn changed_binding_concurrent_process_and_unrelated_database_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite3");
    let store = TaskStore::open(&path, binding()).unwrap();
    assert!(TaskStore::open(&path, binding()).is_err());
    drop(store);
    for changed in [
        Binding {
            task_id: "other".into(),
            ..binding()
        },
        Binding {
            owner_id: "other".into(),
            ..binding()
        },
        Binding {
            conversation_id: "other".into(),
            ..binding()
        },
    ] {
        assert!(TaskStore::open(&path, changed).is_err());
    }
    assert!(TaskStore::open(&path, binding()).is_ok());
    let unrelated = dir.path().join("unrelated.sqlite3");
    let c = rusqlite::Connection::open(&unrelated).unwrap();
    c.execute_batch("CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('keep');")
        .unwrap();
    assert!(TaskStore::open(&unrelated, binding()).is_err());
    assert_eq!(
        c.query_row("SELECT value FROM preserved", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn unassigned_wrong_task_and_reordered_events_cannot_enter_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let store = TaskStore::open(&dir.path().join("app.sqlite3"), binding()).unwrap();
    let mut wrong = reply(1);
    wrong.task_id = "other".into();
    assert!(store
        .stage_page(&Page {
            events: vec![wrong],
            next: None
        })
        .is_err());
    let mut unassigned = reply(1);
    if let TaskEventData::Reply(reply) = &mut unassigned.data {
        reply.task_id = None;
    }
    assert!(store
        .stage_page(&Page {
            events: vec![unassigned],
            next: None
        })
        .is_err());
    assert!(store
        .stage_page(&Page {
            events: vec![reply(2), reply(1)],
            next: None
        })
        .is_err());
    assert!(!store.statistics().unwrap().page_pending);
    assert!(store
        .stage_page(&Page {
            events: vec![],
            next: Some("invented".into())
        })
        .is_err());
}

#[test]
fn unknown_acceptance_preserves_original_cas_and_uncertainty_is_not_redispatched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite3");
    let store = TaskStore::open(&path, binding()).unwrap();
    let command = TaskCommand::ConsumeReply {
        request_id: "original".into(),
        task_id: "task".into(),
        event_id: "reply".into(),
        expected_revision: 1,
    };
    store
        .prepare("consume", Lane::Controller, command.clone())
        .unwrap();
    drop(store);
    let store = TaskStore::open(&path, binding()).unwrap();
    let changed = TaskCommand::ConsumeReply {
        request_id: "replacement".into(),
        task_id: "task".into(),
        event_id: "reply".into(),
        expected_revision: 99,
    };
    assert_eq!(
        store
            .prepare("consume", Lane::Controller, changed)
            .unwrap()
            .command,
        command
    );
    store.rejected("consume", "task-state-conflict").unwrap();
    assert_eq!(store.statistics().unwrap().rejected_commands, 1);
    assert_eq!(store.command("consume").unwrap().unwrap().command, command);
    store
        .prepare(
            "uncertain",
            Lane::Invocation,
            TaskCommand::Get {
                task_id: "task".into(),
            },
        )
        .unwrap();
    store
        .accepted(
            "uncertain",
            &TaskOutcome::Tool(TaskToolResult::Uncertain {
                task_id: "task".into(),
                effect_id: "effect".into(),
            }),
        )
        .unwrap();
    assert!(store.pending_invocations().unwrap().is_empty());
    assert!(matches!(
        store.command("uncertain").unwrap().unwrap().outcome,
        Some(TaskOutcome::Tool(TaskToolResult::Uncertain { .. }))
    ));
}

#[test]
fn consume_reconciliation_keeps_rejected_history_and_never_rebases_unknown_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TaskStore::open(&dir.path().join("app.sqlite3"), binding()).unwrap();
    let original = store.consumption("reply", 1).unwrap();
    let original_id = original.command.request_id().unwrap().to_owned();
    assert_eq!(
        store.consumption("reply", 99).unwrap().command,
        original.command
    );
    store
        .rejected(&original.key, "task-state-conflict")
        .unwrap();
    let reconciled = store.consumption("reply", 99).unwrap();
    assert_ne!(reconciled.command.request_id().unwrap(), original_id);
    assert_eq!(
        store.command(&original.key).unwrap().unwrap().command,
        original.command
    );
    assert_eq!(
        store.consumption("reply", 100).unwrap().command,
        reconciled.command
    );
    store.accepted(&reconciled.key, &receipt()).unwrap();
    assert_eq!(
        store.consume_intent("reply").unwrap().unwrap().command,
        reconciled.command
    );
    assert_eq!(store.statistics().unwrap().pending_commands, 0);
    assert_eq!(store.statistics().unwrap().rejected_commands, 1);
}
