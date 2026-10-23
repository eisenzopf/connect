use super::fixture::Fixture;
use crate::{Scope, Store};
use connect_protocol::{tasks::*, Limits, Operation};

pub(super) fn binding(id: &str, kind: InvocationKind) -> InvocationBinding {
    InvocationBinding {
        kind,
        adapter: "fixture".into(),
        external_id: id.into(),
    }
}
pub(super) fn definition(
    task: &TaskRecord,
    binding: InvocationBinding,
    now: i64,
) -> GrantDefinition {
    GrantDefinition {
        task_id: task.id.clone(),
        binding,
        operations: vec![TaskPermission::Send],
        expires_at: now + 120,
    }
}
pub(super) fn get(task: &TaskRecord) -> TaskRequest {
    TaskRequest::new(TaskCommand::Get {
        task_id: task.id.clone(),
    })
}
pub(super) fn grant(f: &Fixture, task: &TaskRecord) -> crate::IssuedTaskGrant {
    let origin = binding("native-one", InvocationKind::Native);
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    f.store
        .issue_task_grant(&f.key.id, "issue", &definition(task, origin, f.now))
        .unwrap()
}
#[test]
fn issuance_retry_after_restart_regenerates_secret_without_extending_expiry_or_leaking_it() {
    let f = Fixture::new();
    let task = f.task();
    let issued = grant(&f, &task);
    let def = definition(&task, issued.grant.binding.clone(), f.now);
    let replay = f.store.issue_task_grant(&f.key.id, "issue", &def).unwrap();
    assert_eq!(replay.expose_secret(), issued.expose_secret());
    assert_eq!(replay.grant, issued.grant);
    assert!(!format!("{issued:?}").contains(issued.expose_secret()));
    assert!(format!("{issued:?}").contains("[REDACTED]"));
    let serialized = serde_json::to_string(&issued.grant).unwrap();
    assert!(!serialized.contains(issued.expose_secret()));
    let backup = f._dir.path().join("grant.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    let retry = restored.issue_task_grant(&f.key.id, "issue", &def).unwrap();
    assert_eq!(retry.expose_secret(), issued.expose_secret());
    let inv = restored
        .authenticate_task_grant(retry.expose_secret(), &def.binding)
        .unwrap();
    assert_eq!(inv.task_id(), task.id);
    assert_eq!(
        Fixture::record(restored.execute_task_invocation(&inv, &get(&task)).unwrap()),
        task
    );
    let c = f.store.connection().unwrap();
    let stored: String = c
        .query_row(
            "SELECT nonce||verifier||issuance_digest||operations_json FROM task_grants",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!stored.contains(issued.expose_secret()));
}
#[test]
fn grant_cannot_expand_subset_expiry_recipient_or_unregistered_origin() {
    let f = Fixture::new();
    let task = f.task();
    let origin = binding("native-one", InvocationKind::Native);
    let mut def = definition(&task, origin.clone(), f.now);
    assert_eq!(
        f.store
            .issue_task_grant(&f.key.id, "unregistered", &def)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .register_task_binding(&f.key.id, &task.id, &origin)
        .unwrap();
    def.operations = vec![TaskPermission::CallStart];
    assert_eq!(
        f.store
            .issue_task_grant(&f.key.id, "expand", &def)
            .unwrap_err()
            .code,
        "forbidden"
    );
    def.operations = vec![TaskPermission::Send];
    def.expires_at = f.now + 301;
    assert_eq!(
        f.store
            .issue_task_grant_at(&f.key.id, "expiry", &def, f.now)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let issuer = f
        .store
        .issue(
            &f.owner,
            Scope {
                operations: vec!["task_grant".into(), "history".into()],
                recipients: vec![f.owner.clone()],
            },
            f.now + 600,
        )
        .unwrap();
    def.expires_at = f.now + 120;
    assert_eq!(
        f.store
            .issue_task_grant(&issuer.id, "no-send", &def)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let normal = f
        .store
        .issue(
            &f.owner,
            Scope::messaging(vec![f.owner.clone()]),
            f.now + 600,
        )
        .unwrap();
    assert_eq!(
        f.store
            .register_task_binding(&normal.id, &task.id, &origin)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn forged_signature_unknown_grant_wrong_origin_and_changed_metadata_are_denied() {
    let f = Fixture::new();
    let task = f.task();
    let issued = grant(&f, &task);
    let token = issued.expose_secret();
    let mut forged = token.as_bytes().to_vec();
    let last = forged.len() - 1;
    forged[last] = if forged[last] == b'a' { b'b' } else { b'a' };
    assert_eq!(
        f.store
            .authenticate_task_grant(std::str::from_utf8(&forged).unwrap(), &issued.grant.binding)
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let wrong = binding("native-other", InvocationKind::Native);
    assert_eq!(
        f.store
            .authenticate_task_grant(token, &wrong)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let c = f.store.connection().unwrap();
    c.execute("UPDATE task_grants SET operations_json='[\"history\"]'", [])
        .unwrap();
    drop(c);
    assert_eq!(
        f.store
            .authenticate_task_grant(token, &issued.grant.binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn cached_handle_rechecks_expiry_and_revocation_before_each_read() {
    let f = Fixture::new();
    let task = f.task();
    let issued = grant(&f, &task);
    let inv = f
        .store
        .authenticate_task_grant(issued.expose_secret(), &issued.grant.binding)
        .unwrap();
    assert!(f
        .store
        .execute_task_invocation_at(&inv, &get(&task), issued.grant.expires_at - 1)
        .is_ok());
    assert_eq!(
        f.store
            .execute_task_invocation_at(&inv, &get(&task), issued.grant.expires_at)
            .unwrap_err()
            .code,
        "forbidden"
    );
    f.store
        .revoke_task_grant(&f.key.id, &issued.grant.id)
        .unwrap();
    assert_eq!(
        f.store
            .execute_task_invocation(&inv, &get(&task))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .issue_task_grant(
                &f.key.id,
                "issue",
                &definition(&task, issued.grant.binding, f.now)
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn backing_revocation_and_task_cancel_invalidate_an_already_authenticated_handle() {
    for cancel in [false, true] {
        let f = Fixture::new();
        let task = f.task();
        let issued = grant(&f, &task);
        let inv = f
            .store
            .authenticate_task_grant(issued.expose_secret(), &issued.grant.binding)
            .unwrap();
        if cancel {
            f.run(&TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel".into(),
                task_id: task.id.clone(),
                reason: None,
            }))
            .unwrap();
        } else {
            f.store
                .revoke(&f.store.authenticate(&f.key.token).unwrap(), &f.key.id)
                .unwrap();
        }
        assert!(f.store.execute_task_invocation(&inv, &get(&task)).is_err());
    }
}
#[test]
fn a_call_chat_or_host_header_cannot_select_another_task_or_create_a_binding() {
    let f = Fixture::new();
    let one = f.task();
    let mut create = f.create();
    let TaskCommand::Create { request_id, .. } = &mut create.command else {
        panic!("create")
    };
    *request_id = "task-two".into();
    let two = Fixture::record(f.run(&create).unwrap());
    for kind in [
        InvocationKind::ProviderCall,
        InvocationKind::ProviderChat,
        InvocationKind::HostSession,
    ] {
        let origin = binding(&format!("origin-{kind:?}"), kind);
        f.store
            .register_task_binding(&f.key.id, &one.id, &origin)
            .unwrap();
        assert_eq!(
            f.store
                .register_task_binding(&f.key.id, &two.id, &origin)
                .unwrap_err()
                .code,
            "binding-conflict"
        );
        let issued = f
            .store
            .issue_task_grant(
                &f.key.id,
                &format!("issue-{kind:?}"),
                &definition(&one, origin.clone(), f.now),
            )
            .unwrap();
        let inv = f
            .store
            .authenticate_task_grant(issued.expose_secret(), &origin)
            .unwrap();
        assert_eq!(
            f.store
                .execute_task_invocation(&inv, &get(&two))
                .unwrap_err()
                .code,
            "forbidden"
        );
        let mut wrong = origin;
        wrong.external_id = "forged-header".into();
        assert_eq!(
            f.store
                .authenticate_task_grant(issued.expose_secret(), &wrong)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
#[test]
fn narrow_grant_cannot_use_admin_commands_generic_inbox_or_another_conversation() {
    let f = Fixture::new();
    let task = f.task();
    let issued = grant(&f, &task);
    let inv = f
        .store
        .authenticate_task_grant(issued.expose_secret(), &issued.grant.binding)
        .unwrap();
    for command in [
        TaskCommand::Cancel {
            request_id: "cancel".into(),
            task_id: task.id.clone(),
            reason: None,
        },
        TaskCommand::Recall {
            task_id: task.id.clone(),
            after: None,
            limit: 1,
        },
        TaskCommand::Invoke {
            request_id: "send".into(),
            task_id: task.id.clone(),
            step_id: "step".into(),
            operation: Box::new(Operation::Send {
                request_id: "send".into(),
                conversation_id: "different".into(),
                message: connect_protocol::messages::MessageDraft::text_all("forbidden"),
            }),
            reply_to: None,
            wait_for_reply: false,
        },
    ] {
        assert_eq!(
            f.store
                .execute_task_invocation(&inv, &TaskRequest::new(command))
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
