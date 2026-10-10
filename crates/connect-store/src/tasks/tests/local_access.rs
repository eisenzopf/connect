//! Atomic protected installation does not turn local origin labels into providers.
use super::fixture::Fixture;
use connect_protocol::tasks::*;

fn definition(f: &Fixture, task: &TaskRecord, kind: InvocationKind, id: &str) -> GrantDefinition {
    GrantDefinition {
        task_id: task.id.clone(),
        binding: InvocationBinding {
            kind,
            adapter: "test-local".into(),
            external_id: id.into(),
        },
        operations: vec![TaskPermission::History],
        expires_at: f.now + 120,
    }
}
#[test]
fn local_install_replay_and_conflicting_intent_preserve_one_binding_and_secret() {
    let f = Fixture::new();
    let task = f.task();
    for (kind, id) in [
        (InvocationKind::Native, "native"),
        (InvocationKind::HostSession, "host"),
    ] {
        let def = definition(&f, &task, kind, id);
        let grant = f.store.issue_local_task_grant(&f.key.id, id, &def).unwrap();
        let retry = f.store.issue_local_task_grant(&f.key.id, id, &def).unwrap();
        assert_eq!(grant.grant, retry.grant);
        assert_eq!(grant.expose_secret(), retry.expose_secret());
        let handle = f
            .store
            .authenticate_local_task_grant(grant.expose_secret())
            .unwrap();
        assert_eq!(handle.task_id(), task.id);
        let mut changed = def.clone();
        changed.binding.external_id = format!("{id}-other");
        assert_eq!(
            f.store
                .issue_local_task_grant(&f.key.id, id, &changed)
                .unwrap_err()
                .code,
            "request-conflict"
        );
        let count: u64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM task_bindings WHERE external_id=?1",
                [changed.binding.external_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "conflicting retry registered an unrelated origin");
    }
    let count: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM task_grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}
#[test]
fn local_grants_expire_and_cannot_extend_task_operation_authority() {
    let f = Fixture::new();
    let task = f.task();
    let def = definition(&f, &task, InvocationKind::HostSession, "expiry");
    let grant = f
        .store
        .issue_local_task_grant(&f.key.id, "expiry", &def)
        .unwrap();
    assert!(f
        .store
        .authenticate_local_task_grant_at(grant.expose_secret(), f.now + 121)
        .is_err());
    let mut wider = def;
    wider.operations.push(TaskPermission::ArtifactBegin);
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.key.id, "wider", &wider)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn grant_write_failure_rolls_back_registration_and_original_intent_recovers() {
    let f = Fixture::new();
    let task = f.task();
    let def = definition(&f, &task, InvocationKind::HostSession, "rollback");
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER fail_local_grant BEFORE INSERT ON task_grants BEGIN SELECT RAISE(FAIL,'test'); END;").unwrap();
    drop(c);
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.key.id, "rollback", &def)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    let c = f.store.connection().unwrap();
    let count: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM task_bindings WHERE external_id='rollback'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    c.execute_batch("DROP TRIGGER fail_local_grant").unwrap();
    drop(c);
    let grant = f
        .store
        .issue_local_task_grant(&f.key.id, "rollback", &def)
        .unwrap();
    assert!(f
        .store
        .authenticate_local_task_grant(grant.expose_secret())
        .is_ok());
}
#[test]
fn provider_origins_cannot_be_installed_or_admitted_as_local_grants() {
    let f = Fixture::new();
    let task = f.task();
    for (kind, id) in [
        (InvocationKind::ProviderCall, "call"),
        (InvocationKind::ProviderChat, "chat"),
    ] {
        let def = definition(&f, &task, kind, id);
        assert_eq!(
            f.store
                .issue_local_task_grant(&f.key.id, id, &def)
                .unwrap_err()
                .code,
            "forbidden"
        );
        // Trusted verified adapter path remains available, without becoming a
        // generic local token path just because its token is cryptographically valid.
        f.store
            .register_task_binding(&f.key.id, &task.id, &def.binding)
            .unwrap();
        let grant = f.store.issue_task_grant(&f.key.id, id, &def).unwrap();
        assert!(f
            .store
            .authenticate_task_grant(grant.expose_secret(), &def.binding)
            .is_ok());
        assert_eq!(
            f.store
                .authenticate_local_task_grant(grant.expose_secret())
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
#[test]
fn explicit_refresh_does_not_revive_revoked_grants_or_cancelled_tasks() {
    let f = Fixture::new();
    let task = f.task();
    let def = definition(&f, &task, InvocationKind::HostSession, "refresh");
    let old = f
        .store
        .issue_local_task_grant(&f.key.id, "old", &def)
        .unwrap();
    f.store.revoke_task_grant(&f.key.id, &old.grant.id).unwrap();
    let new = f
        .store
        .issue_local_task_grant(&f.key.id, "new", &def)
        .unwrap();
    assert_ne!(old.grant.id, new.grant.id);
    assert!(f
        .store
        .authenticate_local_task_grant(old.expose_secret())
        .is_err());
    assert!(f
        .store
        .authenticate_local_task_grant(new.expose_secret())
        .is_ok());
    let cancel = TaskRequest::new(TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id,
        reason: None,
    });
    f.run(&cancel).unwrap();
    assert!(f
        .store
        .authenticate_local_task_grant(new.expose_secret())
        .is_err());
    assert_eq!(
        f.store
            .issue_local_task_grant(&f.key.id, "after-cancel", &def)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
