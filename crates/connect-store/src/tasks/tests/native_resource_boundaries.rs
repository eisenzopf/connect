use super::native_resource_fixture::MediaFixture;
use connect_protocol::{tasks::*, Operation, Outcome};

#[test]
fn session_in_another_conversation_is_denied_despite_current_owner_membership() {
    let m = MediaFixture::new();
    let Outcome::Conversation(other) =
        m.f.store
            .execute(
                &m.f.key.id,
                &Operation::Create {
                    request_id: "other-conversation".into(),
                    participants: vec![m.peer_id.clone()],
                },
            )
            .unwrap()
    else {
        panic!("conversation")
    };
    let Outcome::Session(session) =
        m.f.store
            .execute(
                &m.f.key.id,
                &Operation::SessionInvite {
                    request_id: "other-invite".into(),
                    conversation_id: other.id.clone(),
                    participant_id: m.peer_id.clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
    else {
        panic!("session")
    };
    m.f.store
        .execute(
            &m.peer.id,
            &Operation::SessionAccept {
                request_id: "other-accept".into(),
                conversation_id: other.id,
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    assert!(m
        .f
        .store
        .authorize_session_binding(&m.f.key.id, &session.id)
        .is_ok());
    let subject = m.grant(
        &m.task,
        "grant",
        vec![TaskPermission::SessionGet, TaskPermission::SessionInvite],
    );
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&subject, &session.id)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn narrow_invocation_created_session_keeps_provenance_across_explicit_grant_replacement() {
    let m = MediaFixture::new();
    let permissions = vec![TaskPermission::SessionGet, TaskPermission::SessionInvite];
    let original = m.grant(&m.task, "original", permissions.clone());
    let authority =
        m.f.store
            .revalidate_native_task_principal(&original)
            .unwrap();
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) =
        m.f.store
            .execute_task_invocation(
                authority.invocation(),
                &TaskRequest::new(TaskCommand::Invoke {
                    request_id: "invite".into(),
                    task_id: m.task.id.clone(),
                    step_id: "voice".into(),
                    operation: Box::new(Operation::SessionInvite {
                        request_id: "invite".into(),
                        conversation_id: m.f.cid.clone(),
                        participant_id: m.peer_id.clone(),
                        media: vec!["audio".into()],
                    }),
                    reply_to: None,
                    wait_for_reply: false,
                }),
            )
            .unwrap()
    else {
        panic!("invite")
    };
    let Outcome::Session(session) = *outcome else {
        panic!("session")
    };
    // Invited-only sessions cannot attach before participant acceptance.
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&original, &session.id)
            .unwrap_err()
            .code,
        "forbidden"
    );
    m.f.store
        .execute(
            &m.peer.id,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: m.f.cid.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&original, &session.id)
        .is_ok());
    m.f.store
        .revoke_task_grant(&m.f.key.id, task_grant_id(&original).unwrap())
        .unwrap();
    let replacement = m.grant(&m.task, "replacement", permissions);
    assert_ne!(original, replacement);
    assert_eq!(
        m.f.store
            .authorize_native_task_session_binding(&original, &session.id)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(m
        .f
        .store
        .authorize_native_task_session_binding(&replacement, &session.id)
        .is_ok());
    let effects: u64 =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM task_effects WHERE task_id=?1",
                [&m.task.id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(
        effects, 1,
        "replacing authority must not recreate the resource"
    );
}
