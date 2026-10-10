use super::fixture::Fixture;
use crate::Scope;
use connect_protocol::tasks::*;

#[test]
fn owner_replacement_credential_cannot_continue_revoked_backing_authority() {
    let f = Fixture::new();
    let task = f.task();
    let replacement = f
        .store
        .issue(&f.owner, Scope::operator(), f.now + 3_600)
        .unwrap();
    f.store
        .revoke(
            &f.store.authenticate(&replacement.token).unwrap(),
            &f.key.id,
        )
        .unwrap();
    assert_eq!(f.run(&f.create()).unwrap_err().code, "unauthorized");
    assert_eq!(
        f.store
            .execute_task(
                &replacement.id,
                &Fixture::transition(&task, "run", TaskState::Running)
            )
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let get = TaskRequest::new(TaskCommand::Get {
        task_id: task.id.clone(),
    });
    assert!(f.store.execute_task(&replacement.id, &get).is_ok());
    let cancel = TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id,
        reason: None,
    });
    assert!(f.store.execute_task(&replacement.id, &cancel).is_ok());
}
#[test]
fn creation_does_not_mint_recipient_operation_origin_or_lifetime_authority() {
    let f = Fixture::new();
    let mut request = f.create();
    let TaskCommand::Create { definition, .. } = &mut request.command else {
        panic!("create")
    };
    definition.scope.recipients = vec![f.store.create_identity("outsider").unwrap().id];
    assert_eq!(f.run(&request).unwrap_err().code, "forbidden");
    let mut request = f.create();
    let TaskCommand::Create { definition, .. } = &mut request.command else {
        panic!("create")
    };
    definition.initiating_event_id = Some("forged-event".into());
    assert_eq!(f.run(&request).unwrap_err().code, "forbidden");
    let key = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_create".into(), "send".into()],
                recipients: vec![f.owner.clone()],
            },
            f.now + 60,
        )
        .unwrap();
    assert_eq!(
        f.store.execute_task(&key.id, &f.create()).unwrap_err().code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
