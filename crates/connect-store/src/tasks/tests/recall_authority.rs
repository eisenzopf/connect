use super::{
    fixture::Fixture,
    recall_helpers::{grant, page, request, send},
};
use crate::{Scope, Store};
use connect_protocol::{tasks::*, Limits};

#[test]
fn cursor_is_bound_to_task_installation_and_recall_resource() {
    let f = Fixture::new();
    send(&f, &f.cid, "one", "fact");
    let one = f.task();
    let (_, cursor, _) = page(f.run(&request(&one, None, 1)).unwrap());
    let mut create = f.create();
    let TaskCommand::Create { request_id, .. } = &mut create.command else {
        panic!("create")
    };
    *request_id = "second-task".into();
    let two = Fixture::record(f.run(&create).unwrap());
    assert_eq!(
        f.run(&request(&two, cursor.clone(), 1)).unwrap_err().code,
        "invalid-request"
    );
    let other = Fixture::new();
    let task = other.task();
    assert_eq!(
        other
            .run(&request(&task, cursor.clone(), 1))
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let inbox = TaskRequest::new(TaskCommand::Inbox {
        task_id: one.id,
        after: cursor.clone(),
        limit: 1,
        wait_ms: 0,
    });
    assert_eq!(f.run(&inbox).unwrap_err().code, "invalid-request");
    let mut forged = cursor.unwrap().into_bytes();
    let last = forged.len() - 1;
    forged[last] = if forged[last] == b'a' { b'b' } else { b'a' };
    assert_eq!(
        f.run(&request(&two, Some(String::from_utf8(forged).unwrap()), 1))
            .unwrap_err()
            .code,
        "invalid-request"
    );
}

#[test]
fn current_grant_and_backing_authority_apply_to_every_continuation_page() {
    for deny in [
        "grant-revoke",
        "grant-expire",
        "credential-revoke",
        "membership",
        "cancel",
    ] {
        let f = Fixture::new();
        send(&f, &f.cid, "one", "old");
        send(&f, &f.cid, "two", "next");
        let task = f.task();
        let (issued, invocation) = grant(&f, &task);
        let (_, cursor, _) = page(
            f.store
                .execute_task_invocation_at(&invocation, &request(&task, None, 1), f.now)
                .unwrap(),
        );
        let timestamp = match deny {
            "grant-revoke" => {
                f.store
                    .revoke_task_grant(&f.key.id, &issued.grant.id)
                    .unwrap();
                f.now
            }
            "grant-expire" => issued.grant.expires_at,
            "credential-revoke" => {
                let actor = f.store.authenticate(&f.key.token).unwrap();
                f.store.revoke(&actor, &f.key.id).unwrap();
                f.now
            }
            "membership" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "DELETE FROM participants WHERE conversation_id=?1",
                        [&f.cid],
                    )
                    .unwrap();
                f.now
            }
            "cancel" => {
                f.run(&TaskRequest::new(TaskCommand::Cancel {
                    request_id: "cancel".into(),
                    task_id: task.id.clone(),
                    reason: None,
                }))
                .unwrap();
                f.now
            }
            _ => panic!("denial"),
        };
        assert!(
            f.store
                .execute_task_invocation_at(&invocation, &request(&task, cursor, 1), timestamp)
                .is_err(),
            "{deny}"
        );
    }
}

#[test]
fn owner_recall_requires_current_read_scope_inside_task_ceiling() {
    let f = Fixture::new();
    send(&f, &f.cid, "one", "fact");
    let task = f.task();
    let no_history = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_recall".into()],
                recipients: vec![f.owner.clone()],
            },
            f.now + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .execute_task(&no_history.id, &request(&task, None, 1))
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut create = f.create();
    let TaskCommand::Create {
        request_id,
        definition,
    } = &mut create.command
    else {
        panic!("create")
    };
    *request_id = "send-only".into();
    definition.scope.operations = vec![TaskPermission::Send];
    let narrow = Fixture::record(f.run(&create).unwrap());
    assert_eq!(
        f.run(&request(&narrow, None, 1)).unwrap_err().code,
        "forbidden"
    );
    let TaskCommand::Create {
        request_id,
        definition,
    } = &mut create.command
    else {
        panic!("create")
    };
    definition.scope.operations = vec![TaskPermission::ContextView];
    *request_id = "context-only".into();
    let context = Fixture::record(f.run(&create).unwrap());
    assert_eq!(page(f.run(&request(&context, None, 1)).unwrap()).0.len(), 1);
}

#[test]
fn cursor_newer_than_restored_snapshot_is_rejected_without_reset_or_leak() {
    let f = Fixture::new();
    send(&f, &f.cid, "one", "fact");
    let task = f.task();
    let backup = f._dir.path().join("old.sqlite3");
    f.store.backup(&backup).unwrap();
    send(&f, &f.cid, "two", "after backup");
    let (_, cursor, _) = page(f.run(&request(&task, None, 100)).unwrap());
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_eq!(
        restored
            .execute_task(&f.key.id, &request(&task, cursor, 100))
            .unwrap_err()
            .code,
        "invalid-request"
    );
}
