//! Delegated terminal control preserves transactional denial and current scope.
use super::fixture::Fixture;
use crate::{Store, TaskInvocation};
use connect_protocol::{tasks::*, Limits};

fn setup() -> (Fixture, TaskRecord, TaskInvocation) {
    let f = Fixture::new();
    let mut request = f.create();
    let TaskCommand::Create { definition, .. } = &mut request.command else {
        panic!("create")
    };
    definition.scope.operations.push(TaskPermission::TaskCancel);
    let task = Fixture::record(f.run(&request).unwrap());
    let definition = GrantDefinition {
        task_id: task.id.clone(),
        binding: InvocationBinding {
            kind: InvocationKind::Native,
            adapter: "cancel-fixture".into(),
            external_id: "one-task".into(),
        },
        operations: vec![TaskPermission::TaskCancel],
        expires_at: f.now + 120,
    };
    f.store
        .register_task_binding(&f.key.id, &task.id, &definition.binding)
        .unwrap();
    let grant = f
        .store
        .issue_task_grant(&f.key.id, "cancel-grant", &definition)
        .unwrap();
    let invocation = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &definition.binding)
        .unwrap();
    (f, task, invocation)
}
fn cancel(task: &TaskRecord) -> TaskRequest {
    TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel-exact".into(),
        task_id: task.id.clone(),
        reason: None,
    })
}

#[test]
fn delegated_cancel_receipt_capacity_failure_rolls_back_state_event_and_grant_revocation() {
    let (f, task, invocation) = setup();
    let strict = Store::open(
        f.store.path(),
        Limits {
            operation_receipts: 0,
            ..Limits::default()
        },
    )
    .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        strict
            .execute_task_invocation_at(&invocation, &cancel(&task), f.now)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        Fixture::record(
            f.run(&TaskRequest::new(TaskCommand::Get {
                task_id: task.id.clone()
            }))
            .unwrap()
        ),
        task
    );
    // A failed admission leaves the grant usable; the same original intent can
    // succeed once the capacity configuration is explicitly repaired.
    let cancelled = Fixture::record(
        f.store
            .execute_task_invocation_at(&invocation, &cancel(&task), f.now)
            .unwrap(),
    );
    assert_eq!(cancelled.state, TaskState::Cancelled);
}

#[test]
fn cancelled_or_elapsed_grant_denies_control_replay_without_changing_receipts() {
    let (f, task, invocation) = setup();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation_at(&invocation, &cancel(&task), f.now + 121)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let accepted = f
        .store
        .execute_task_invocation_at(&invocation, &cancel(&task), f.now)
        .unwrap();
    let after = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation_at(&invocation, &cancel(&task), f.now)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), after);
    assert_eq!(f.run(&cancel(&task)).unwrap(), accepted);
}
