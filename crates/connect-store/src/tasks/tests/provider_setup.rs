//! Atomic verified-provider issuance and retained intent under failure/recovery.
use super::fixture::Fixture;
use connect_protocol::tasks::*;

fn definition(f: &Fixture, kind: InvocationKind) -> GrantDefinition {
    GrantDefinition {
        task_id: f.task().id,
        binding: InvocationBinding {
            kind,
            adapter: "verified-provider".into(),
            external_id: "resource-one".into(),
        },
        operations: vec![TaskPermission::History],
        expires_at: f.now + 120,
    }
}
fn counts(f: &Fixture) -> (u64, u64) {
    let c = f.store.connection().unwrap();
    c.query_row(
        "SELECT (SELECT COUNT(*) FROM task_bindings),(SELECT COUNT(*) FROM task_grants)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn preflight_is_read_only_and_matching_issue_replay_retains_exact_authority() {
    for kind in [InvocationKind::ProviderCall, InvocationKind::ProviderChat] {
        let f = Fixture::new();
        let def = definition(&f, kind);
        f.store
            .authorize_provider_task_setup(&f.key.id, "issue", &def)
            .unwrap();
        assert_eq!(counts(&f), (0, 0));
        let first = f
            .store
            .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
            .unwrap();
        f.store
            .authorize_provider_task_setup(&f.key.id, "issue", &def)
            .unwrap();
        let replay = f
            .store
            .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
            .unwrap();
        assert_eq!(first.grant, replay.grant);
        assert_eq!(first.expose_secret(), replay.expose_secret());
        assert_eq!(counts(&f), (1, 1));
        assert!(f
            .store
            .authenticate_task_grant(first.expose_secret(), &def.binding)
            .is_ok());
        assert!(f
            .store
            .authenticate_local_task_grant(first.expose_secret())
            .is_err());
    }
}

#[test]
fn failed_insert_rolls_back_binding_then_definite_retry_recovers_one_grant() {
    let f = Fixture::new();
    let def = definition(&f, InvocationKind::ProviderCall);
    f.store.connection().unwrap().execute_batch(
        "CREATE TRIGGER fail_provider_grant BEFORE INSERT ON task_grants BEGIN SELECT RAISE(FAIL,'test'); END;",
    ).unwrap();
    assert_eq!(
        f.store
            .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(counts(&f), (0, 0));
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_provider_grant")
        .unwrap();
    let first = f
        .store
        .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
        .unwrap();
    let replay = f
        .store
        .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
        .unwrap();
    assert_eq!(first.expose_secret(), replay.expose_secret());
    assert_eq!(counts(&f), (1, 1));
}

#[test]
fn kind_scope_and_expiry_denials_cannot_register_an_origin() {
    let f = Fixture::new();
    let def = definition(&f, InvocationKind::ProviderCall);
    let mut cases = Vec::new();
    for kind in [InvocationKind::Native, InvocationKind::HostSession] {
        let mut invalid = def.clone();
        invalid.binding.kind = kind;
        cases.push(invalid);
    }
    let mut wider = def.clone();
    wider.operations.push(TaskPermission::ArtifactBegin);
    cases.push(wider);
    let mut expired = def.clone();
    expired.expires_at = f.now - 1;
    cases.push(expired);
    let mut beyond_task = def;
    beyond_task.expires_at = f.now + 601;
    cases.push(beyond_task);
    for invalid in cases {
        assert!(f
            .store
            .authorize_provider_task_setup(&f.key.id, "denied", &invalid)
            .is_err());
        assert!(f
            .store
            .issue_verified_provider_task_grant(&f.key.id, "denied", &invalid)
            .is_err());
        assert_eq!(counts(&f), (0, 0));
    }
}

#[test]
fn conflicting_intent_other_task_and_revoked_replay_cannot_rebind() {
    let f = Fixture::new();
    let def = definition(&f, InvocationKind::ProviderChat);
    let first = f
        .store
        .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
        .unwrap();
    let mut changed = def.clone();
    changed.binding.external_id = "resource-two".into();
    assert_eq!(
        f.store
            .authorize_provider_task_setup(&f.key.id, "issue", &changed)
            .unwrap_err()
            .code,
        "request-conflict"
    );
    assert_eq!(
        f.store
            .issue_verified_provider_task_grant(&f.key.id, "issue", &changed)
            .unwrap_err()
            .code,
        "request-conflict"
    );
    let mut request = f.create();
    if let TaskCommand::Create { request_id, .. } = &mut request.command {
        *request_id = "second-task".into();
    }
    let second = Fixture::record(f.run(&request).unwrap());
    changed = def.clone();
    changed.task_id = second.id;
    assert_eq!(
        f.store
            .issue_verified_provider_task_grant(&f.key.id, "other-task", &changed)
            .unwrap_err()
            .code,
        "binding-conflict"
    );
    f.store
        .revoke_task_grant(&f.key.id, &first.grant.id)
        .unwrap();
    assert!(f
        .store
        .authorize_provider_task_setup(&f.key.id, "issue", &def)
        .is_err());
    assert!(f
        .store
        .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
        .is_err());
    assert_eq!(counts(&f), (1, 1));
}

#[test]
fn authority_changed_after_preflight_denies_the_final_transaction() {
    for cancelled in [false, true] {
        let f = Fixture::new();
        let def = definition(&f, InvocationKind::ProviderCall);
        f.store
            .authorize_provider_task_setup(&f.key.id, "issue", &def)
            .unwrap();
        if cancelled {
            f.run(&TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel".into(),
                task_id: def.task_id.clone(),
                reason: None,
            }))
            .unwrap();
        } else {
            f.store
                .revoke(&f.store.authenticate(&f.key.token).unwrap(), &f.key.id)
                .unwrap();
        }
        assert!(f
            .store
            .issue_verified_provider_task_grant(&f.key.id, "issue", &def)
            .is_err());
        assert_eq!(counts(&f), (0, 0));
    }
}
