use super::{
    fixture::Fixture,
    grants::{binding, definition},
};
use connect_protocol::{tasks::*, Operation, Outcome};

#[test]
fn protected_issuance_uses_the_frozen_task_identifier_shape() {
    let f = Fixture::new();
    let task = f.task();
    let origin = binding("host:one.session", InvocationKind::HostSession);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    let definition = definition(&task, origin, f.now);
    let grant = f
        .store
        .issue_task_grant(&f.key.id, "issuer:host.refresh-1", &definition)
        .unwrap();
    assert_eq!(
        f.store
            .issue_task_grant(&f.key.id, "issuer:host.refresh-1", &definition)
            .unwrap()
            .expose_secret(),
        grant.expose_secret()
    );
    assert_eq!(
        f.store
            .issue_task_grant(&f.key.id, "bad/identifier", &definition)
            .unwrap_err()
            .code,
        "invalid-request"
    );
}

#[test]
fn an_immutable_room_cannot_broadcast_outside_the_task_recipient_ceiling() {
    let mut f = Fixture::new();
    let alice = f.store.create_identity("alice").unwrap().id;
    let bob = f.store.create_identity("bob").unwrap().id;
    f.cid = match f
        .store
        .execute(
            &f.key.id,
            &Operation::Create {
                request_id: "room".into(),
                participants: vec![alice.clone(), bob],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("room"),
    };
    let mut create = f.create();
    let TaskCommand::Create {
        definition: task, ..
    } = &mut create.command
    else {
        panic!("task")
    };
    task.scope.operations.push(TaskPermission::Resolve);
    task.scope.recipients = vec![alice];
    let task = Fixture::record(f.run(&create).unwrap());
    let origin = binding("one", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    let mut definition = definition(&task, origin.clone(), f.now);
    definition.operations.push(TaskPermission::Resolve);
    let grant = f
        .store
        .issue_task_grant(&f.key.id, "issue", &definition)
        .unwrap();
    let handle = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &origin)
        .unwrap();
    let before = crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap();
    for operation in [
        Operation::Send {
            request_id: "send".into(),
            conversation_id: f.cid.clone(),
            body: "This room also contains an unauthorized recipient".into(),
        },
        Operation::Resolve {
            address: "connect:bob@local".into(),
        },
    ] {
        let request = TaskRequest::new(TaskCommand::Invoke {
            request_id: "send".into(),
            task_id: task.id.clone(),
            step_id: "step".into(),
            operation: Box::new(operation),
            reply_to: None,
            wait_for_reply: false,
        });
        assert_eq!(
            f.store
                .execute_task_invocation(&handle, &request)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert_eq!(
        crate::Store::inspect_snapshot(&f._dir.path().join("state.sqlite3")).unwrap(),
        before
    );
}

#[test]
fn an_inbox_grant_cannot_use_the_generic_account_inbox() {
    let f = Fixture::new();
    let mut create = f.create();
    let TaskCommand::Create {
        definition: task, ..
    } = &mut create.command
    else {
        panic!("task")
    };
    task.scope.operations.push(TaskPermission::Inbox);
    let task = Fixture::record(f.run(&create).unwrap());
    let origin = binding("one", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    let mut definition = definition(&task, origin.clone(), f.now);
    definition.operations = vec![TaskPermission::Inbox];
    let grant = f
        .store
        .issue_task_grant(&f.key.id, "issue", &definition)
        .unwrap();
    let handle = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &origin)
        .unwrap();
    let request = TaskRequest::new(TaskCommand::Invoke {
        request_id: "read".into(),
        task_id: task.id.clone(),
        step_id: "step".into(),
        operation: Box::new(Operation::Inbox {
            cursor: None,
            limit: 100,
            wait_ms: 0,
        }),
        reply_to: None,
        wait_for_reply: false,
    });
    assert_eq!(
        f.store
            .execute_task_invocation(&handle, &request)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(
        matches!(f.store.execute_task_invocation(&handle,&TaskRequest::new(TaskCommand::Inbox {task_id:task.id,after:None,limit:100,wait_ms:0})).unwrap(),TaskOutcome::Inbox {events,..} if events.len()==1)
    );
}
