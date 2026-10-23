use super::{
    fixture::Fixture,
    grants::{binding, definition, get, grant},
};
use crate::{Scope, Store};
use connect_protocol::{tasks::*, Limits};

#[test]
fn task_and_issuer_expiry_are_independent_upper_bounds() {
    let f = Fixture::new();
    let mut create = f.create();
    let TaskCommand::Create {
        definition: task, ..
    } = &mut create.command
    else {
        panic!("create")
    };
    task.expires_at = f.now + 60;
    let task = Fixture::record(f.run(&create).unwrap());
    let origin = binding("short", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    assert_eq!(
        f.store
            .issue_task_grant(
                &f.key.id,
                "too-long",
                &definition(&task, origin.clone(), f.now)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let issuer = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_grant".into(), "send".into()],
                recipients: vec![f.owner.clone()],
            },
            f.now + 30,
        )
        .unwrap();
    let mut def = definition(&task, origin, f.now);
    def.expires_at = f.now + 31;
    assert_eq!(
        f.store
            .issue_task_grant(&issuer.id, "issuer-ceiling", &def)
            .unwrap_err()
            .code,
        "forbidden"
    );
    def.expires_at = f.now + 30;
    assert!(f
        .store
        .issue_task_grant(&issuer.id, "bounded", &def)
        .is_ok());
}

#[test]
fn issuer_and_registrar_revocation_deny_cached_handles_and_discovery() {
    for revoke_issuer in [false, true] {
        let f = Fixture::new();
        let task = f.task();
        let registrar = f
            .store
            .issue(&f.owner, Scope::operator(), f.now + 600)
            .unwrap();
        let issuer = f
            .store
            .issue(&f.owner, Scope::operator(), f.now + 600)
            .unwrap();
        let origin = binding("separate-delegation", InvocationKind::HostSession);
        f.store
            .register_task_binding(&registrar.id, &task.id, &origin)
            .unwrap();
        let grant = f
            .store
            .issue_task_grant(
                &issuer.id,
                "issue",
                &definition(&task, origin.clone(), f.now),
            )
            .unwrap();
        let handle = f
            .store
            .authenticate_task_grant(grant.expose_secret(), &origin)
            .unwrap();
        assert_eq!(f.store.task_invocation_grant(&handle).unwrap(), grant.grant);
        f.store
            .revoke(
                &f.store.authenticate(&f.key.token).unwrap(),
                if revoke_issuer {
                    &issuer.id
                } else {
                    &registrar.id
                },
            )
            .unwrap();
        assert!(f
            .store
            .execute_task_invocation(&handle, &get(&task))
            .is_err());
        assert!(f.store.task_invocation_grant(&handle).is_err());
    }
}

#[test]
fn lost_membership_and_removed_issuer_operations_deny_previous_authority() {
    for membership in [false, true] {
        let f = Fixture::new();
        let task = f.task();
        let issued = grant(&f, &task);
        let handle = f
            .store
            .authenticate_task_grant(issued.expose_secret(), &issued.grant.binding)
            .unwrap();
        let c = f.store.connection().unwrap();
        if membership {
            c.execute(
                "UPDATE participants SET active=0 WHERE conversation_id=?1 AND identity_id=?2",
                rusqlite::params![f.cid, f.owner],
            )
            .unwrap();
        } else {
            c.execute(
                "UPDATE credentials SET operations=?1 WHERE id=?2",
                rusqlite::params![
                    serde_json::to_string(&vec!["task_grant", "history"]).unwrap(),
                    f.key.id
                ],
            )
            .unwrap();
        }
        drop(c);
        assert_eq!(
            f.store
                .execute_task_invocation(&handle, &get(&task))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}

#[test]
fn binding_and_grant_capacity_preserve_matching_retries_and_rollback_failures() {
    let f = Fixture::new();
    let task = f.task();
    let capped = Store::open(
        &f._dir.path().join("state.sqlite3"),
        Limits {
            operation_receipts: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    let origin = binding("one", InvocationKind::Native);
    capped
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    capped
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    assert_eq!(
        capped
            .register_task_binding(&f.key.id, &task.id, &binding("two", InvocationKind::Native))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let def = definition(&task, origin, f.now);
    let grant = capped.issue_task_grant(&f.key.id, "one", &def).unwrap();
    assert_eq!(
        capped
            .issue_task_grant(&f.key.id, "one", &def)
            .unwrap()
            .expose_secret(),
        grant.expose_secret()
    );
    assert_eq!(
        capped
            .issue_task_grant(&f.key.id, "two", &def)
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    let c = capped.connection().unwrap();
    for table in ["task_bindings", "task_grants"] {
        let count: i64 = c
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn an_unknown_or_other_installation_secret_cannot_authenticate() {
    let f = Fixture::new();
    let task = f.task();
    let grant = grant(&f, &task);
    let handle = f
        .store
        .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
        .unwrap();
    let unknown = format!("ctg1.{}.{}", uuid::Uuid::new_v4(), "0".repeat(64));
    assert_eq!(
        f.store
            .authenticate_task_grant(&unknown, &grant.grant.binding)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let other = Fixture::new();
    assert!(other
        .store
        .authenticate_task_grant(grant.expose_secret(), &grant.grant.binding)
        .is_err());
    assert_eq!(
        other
            .store
            .execute_task_invocation(&handle, &get(&task))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn concurrent_matching_issuance_has_one_secret_and_changed_intent_conflicts() {
    let f = Fixture::new();
    let task = f.task();
    let origin = binding("race", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    let def = definition(&task, origin, f.now);
    let other = Store::open(&f._dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let grants = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            f.store.issue_task_grant(&f.key.id, "race", &def).unwrap()
        });
        let two = scope.spawn(|| {
            barrier.wait();
            other.issue_task_grant(&f.key.id, "race", &def).unwrap()
        });
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_eq!(grants.0.expose_secret(), grants.1.expose_secret());
    let mut changed = def;
    changed.expires_at += 1;
    assert_eq!(
        f.store
            .issue_task_grant(&f.key.id, "race", &changed)
            .unwrap_err()
            .code,
        "request-conflict"
    );
}
