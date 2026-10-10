use super::{effect_helpers::*, fixture::Fixture};
use crate::{DiscoveryRuntime, Limits, Store};
use connect_protocol::{tasks::*, Operation, Outcome};

#[test]
fn matching_effect_survives_restore_and_distinct_tasks_namespace_wire_receipts() {
    let f = Fixture::new();
    let one = f.task();
    let a = send(&f, &one, "same", "first", false);
    let accepted = f.run(&a).unwrap();
    assert!(
        matches!(&accepted,TaskOutcome::Tool(TaskToolResult::Completed {outcome,..}) if matches!(outcome.as_ref(),Outcome::Accepted {request_id,..} if request_id=="same"))
    );
    assert_eq!(f.run(&a).unwrap(), accepted);
    let mut changed = a.clone();
    let TaskCommand::Invoke { operation, .. } = &mut changed.command else {
        panic!("invoke")
    };
    let Operation::Send { body, .. } = operation.as_mut() else {
        panic!("send")
    };
    *body = "different".into();
    assert_eq!(f.run(&changed).unwrap_err().code, "request-conflict");
    let mut create = f.create();
    let TaskCommand::Create { request_id, .. } = &mut create.command else {
        panic!("create")
    };
    *request_id = "two".into();
    let two = Fixture::record(f.run(&create).unwrap());
    let b = send(&f, &two, "same", "first", false);
    let other = f.run(&b).unwrap();
    assert_ne!(message(&accepted), message(&other));
    let backup = f._dir.path().join("effect-backup.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(restored.execute_task(&f.key.id, &a).unwrap(), accepted);
    assert_eq!(restored.execute_task(&f.key.id, &b).unwrap(), other);
    assert_eq!(count(&f, "messages"), 2);
    assert_eq!(count(&f, "task_effect_authority"), 2);
    assert_eq!(count(&f, "requests"), 3);
}
#[test]
fn known_business_rejection_is_retained_without_partial_message_or_retry() {
    let f = Fixture::new();
    let task = f.task();
    let a = send(&f, &task, "empty", "", false);
    let error = f.run(&a).unwrap_err();
    assert_eq!(error.code, "invalid-request");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(f.run(&a).unwrap_err(), error);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(count(&f, "messages"), 0);
    let e = effect(&f, &task);
    assert_eq!(e.state, EffectState::Failed);
    assert_eq!(e.error, Some(error));
}
#[test]
fn revoked_or_expired_invocation_cannot_replay_an_accepted_effect() {
    let f = Fixture::new();
    let task = f.task();
    let handle = handle(&f, &task);
    let a = send(&f, &task, "send", "accepted", false);
    f.store
        .execute_task_invocation_at(&handle, &a, f.now)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation_at(&handle, &a, f.now + 120)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .revoke_task_grant(&f.key.id, handle.grant_id())
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &a)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(count(&f, "messages"), 1);
}
#[test]
fn task_cancel_denies_even_matching_owner_replay_but_retains_effect_inspection() {
    let f = Fixture::new();
    let task = f.task();
    let a = send(&f, &task, "accepted", "one", false);
    f.run(&a).unwrap();
    f.run(&TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: None,
    }))
    .unwrap();
    assert_eq!(f.run(&a).unwrap_err().code, "forbidden");
    assert_eq!(effect(&f, &task).state, EffectState::Succeeded);
    assert_eq!(count(&f, "messages"), 1);
}

#[test]
fn a_new_owner_credential_cannot_replay_work_from_a_revoked_backing_credential() {
    let f = Fixture::new();
    let task = f.task();
    let a = send(&f, &task, "send", "One effect", false);
    f.run(&a).unwrap();
    let replacement = f
        .store
        .issue(&f.owner, crate::Scope::operator(), f.now + 3_600)
        .unwrap();
    let operator = f.store.authenticate(&replacement.token).unwrap();
    f.store.revoke(&operator, &f.key.id).unwrap();
    assert_eq!(
        f.store.execute_task(&replacement.id, &a).unwrap_err().code,
        "unauthorized"
    );
    let id: String = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT id FROM task_effects", [], |r| r.get(0))
        .unwrap();
    assert!(
        matches!(f.store.execute_task(&replacement.id,&TaskRequest::new(TaskCommand::EffectGet {task_id:task.id,effect_id:id})).unwrap(),TaskOutcome::Effect(e) if e.state==EffectState::Succeeded)
    );
    assert_eq!(count(&f, "messages"), 1);
}
#[test]
fn nested_reads_do_not_create_mutation_effects_and_use_configured_discovery() {
    let f = Fixture::new();
    let mut create = f.create();
    let TaskCommand::Create { definition, .. } = &mut create.command else {
        panic!("create")
    };
    definition
        .scope
        .operations
        .push(TaskPermission::Capabilities);
    let task = Fixture::record(f.run(&create).unwrap());
    let a = TaskRequest::new(TaskCommand::Invoke {
        request_id: "cap".into(),
        task_id: task.id,
        step_id: "read".into(),
        operation: Box::new(Operation::Capabilities {
            address: "connect:owner@local".into(),
            conversation_id: Some(f.cid.clone()),
        }),
        reply_to: None,
        wait_for_reply: false,
    });
    let runtime = DiscoveryRuntime {
        native_audio: true,
        ..Default::default()
    };
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = f
        .store
        .execute_task_with_runtime(&f.key.id, &a, &runtime)
        .unwrap()
    else {
        panic!("tool")
    };
    let Outcome::Capabilities(cap) = outcome.as_ref() else {
        panic!("capabilities")
    };
    assert!(cap
        .routes
        .iter()
        .any(|r| r.id == "native-audio" && r.readiness == "configured"));
    assert_eq!(count(&f, "task_effects"), 0);
}
