use super::native_resource_fixture::MediaFixture;
use connect_protocol::{tasks::*, Operation};

#[test]
fn participant_role_and_task_provenance_allow_only_the_bound_session() {
    let m = MediaFixture::new();
    let outbound = m.accepted_outbound("outbound", true);
    let inbound = m.accepted_inbound();
    let initiator = m.grant(
        &m.task,
        "initiator",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    let participant = m.grant(
        &m.task,
        "participant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionAccept],
    );
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&initiator, &outbound.id)
            .unwrap(),
        outbound
    );
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&participant, &inbound.id)
            .unwrap(),
        inbound
    );
    for (subject, sid) in [(&initiator, &inbound.id), (&participant, &outbound.id)] {
        assert_eq!(
            m.f.store
                .authorize_native_task_session_binding(subject, sid)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}

#[test]
fn read_only_or_missing_read_grants_cannot_attach_even_to_task_owned_sessions() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("outbound", true);
    for (id, operations) in [
        ("read", vec![TaskPermission::SessionGet]),
        ("invite", vec![TaskPermission::SessionInvite]),
    ] {
        let subject = m.grant(&m.task, id, operations);
        assert_eq!(
            m.f.store
                .authorize_native_task_session_binding(&subject, &session.id)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}

#[test]
fn same_owner_and_conversation_do_not_transfer_resource_authority_between_tasks() {
    let m = MediaFixture::new();
    let linked = m.accepted_outbound("linked", true);
    let primitive = m.accepted_outbound("primitive", false);
    let another = MediaFixture::create_task(&m.f, &m.peer_id, "another-task");
    let operations = vec![TaskPermission::SessionGet, TaskPermission::SessionInvite];
    let own = m.grant(&m.task, "own", operations.clone());
    let other = m.grant(&another, "other", operations);
    for (subject, sid) in [
        (&own, &primitive.id),
        (&other, &linked.id),
        (&own, &"unknown-session".into()),
    ] {
        assert_eq!(
            m.f.store
                .authorize_native_task_session_binding(subject, sid)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&own, &linked.id)
        .is_ok());
}

#[test]
fn session_end_revokes_resource_attachment_without_revoking_read_authority() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("outbound", true);
    let subject = m.grant(
        &m.task,
        "grant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&subject, &session.id)
        .is_ok());
    m.f.store
        .execute(
            &m.f.key.id,
            &Operation::SessionEnd {
                request_id: "end".into(),
                conversation_id: m.f.cid.clone(),
                session_id: session.id.clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(m.f.store.revalidate_native_task_principal(&subject).is_ok());
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&subject, &session.id)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn resource_reauthorization_observes_current_revocation_task_and_parent_state() {
    for change in [
        "grant",
        "grant-expiry",
        "task",
        "parent",
        "membership",
        "conversation",
    ] {
        let m = MediaFixture::new();
        let session = m.accepted_outbound("outbound", true);
        let subject = m.grant(
            &m.task,
            "grant",
            vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
        );
        assert!(m
            .f
            .store
            .authorize_native_task_session_binding(&subject, &session.id)
            .is_ok());
        match change {
            "grant" => {
                m.f.store
                    .revoke_task_grant(&m.f.key.id, task_grant_id(&subject).unwrap())
                    .unwrap()
            }
            "grant-expiry" => {
                m.f.store
                    .connection()
                    .unwrap()
                    .execute("UPDATE task_grants SET expires_at=0", [])
                    .unwrap();
            }
            "task" => {
                m.f.run(&TaskRequest::new(TaskCommand::Cancel {
                    request_id: "cancel".into(),
                    task_id: m.task.id.clone(),
                    reason: None,
                }))
                .unwrap();
            }
            "parent" => {
                m.f.store.connection().unwrap().execute("UPDATE credentials SET operations='[\"task_grant\",\"session_get\"]' WHERE id=?1", [&m.f.key.id]).unwrap();
            }
            "membership" => {
                m.f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE participants SET active=0 WHERE conversation_id=?1 AND identity_id=?2",
                        rusqlite::params![m.f.cid, m.f.owner],
                    )
                    .unwrap();
            }
            "conversation" => {
                m.f.store
                    .connection()
                    .unwrap()
                    .execute("UPDATE conversations SET closed=1 WHERE id=?1", [&m.f.cid])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            m.f.store
                .authorize_native_task_session_binding(&subject, &session.id)
                .unwrap_err()
                .code,
            if change == "conversation" {
                "conflict"
            } else {
                "forbidden"
            },
            "{change}"
        );
    }
}

#[test]
fn owner_credentials_and_non_native_origins_cannot_be_used_as_native_subjects() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("outbound", true);
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&m.f.key.id, &session.id)
        .is_err());
    let grant =
        m.f.store
            .issue_local_task_grant(
                &m.f.key.id,
                "host",
                &GrantDefinition {
                    task_id: m.task.id.clone(),
                    binding: InvocationBinding {
                        kind: InvocationKind::HostSession,
                        adapter: "host-test".into(),
                        external_id: "host".into(),
                    },
                    operations: vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
                    expires_at: m.f.now + 120,
                },
            )
            .unwrap();
    let subject = task_grant_subject(&grant.grant.id).unwrap();
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&subject, &session.id)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn stored_task_resource_binding_survives_store_reopen() {
    let m = MediaFixture::new();
    let session = m.accepted_outbound("outbound", true);
    let subject = m.grant(
        &m.task,
        "grant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    let reopened = crate::Store::open(
        &m.f._dir.path().join("state.sqlite3"),
        connect_protocol::Limits::default(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .authorize_native_task_session_binding(&subject, &session.id)
            .unwrap(),
        session
    );
}
