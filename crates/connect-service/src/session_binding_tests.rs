#![allow(clippy::unwrap_used)]
use super::*;
use crate::StoreValidator;
use connect_protocol::{tasks::*, Limits, Operation, Outcome};
use connect_store::Scope;
use rvoip_auth_core::BearerValidator;

#[tokio::test]
async fn native_resource_projection_checks_origin_binding_and_current_authority() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let owner = store.create_identity("owner").unwrap().id;
    let peer = store.create_identity("peer").unwrap().id;
    let now = chrono::Utc::now().timestamp();
    let key = store.issue(&owner, Scope::operator(), now + 3600).unwrap();
    let peer_key = store
        .issue(&peer, Scope::communications(vec![owner]), now + 3600)
        .unwrap();
    let Outcome::Conversation(conversation) = store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "conversation".into(),
                participants: vec![peer.clone()],
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    let permissions = vec![TaskPermission::SessionGet, TaskPermission::SessionInvite];
    let TaskOutcome::Record(task) = store
        .execute_task(
            &key.id,
            &TaskRequest::new(TaskCommand::Create {
                request_id: "task".into(),
                definition: TaskDefinition {
                    conversation_id: conversation.id.clone(),
                    scope: TaskScope {
                        operations: permissions.clone(),
                        recipients: vec![peer.clone()],
                        sms_enrollments: vec![],
                    },
                    expires_at: now + 600,
                    initiating_event_id: None,
                },
            }),
        )
        .unwrap()
    else {
        panic!("task")
    };
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = store
        .execute_task(
            &key.id,
            &TaskRequest::new(TaskCommand::Invoke {
                request_id: "invite".into(),
                task_id: task.id.clone(),
                step_id: "voice".into(),
                operation: Box::new(Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: conversation.id.clone(),
                    participant_id: peer,
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
    store
        .execute(
            &peer_key.id,
            &Operation::SessionAccept {
                request_id: "accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let grant = store
        .issue_local_task_grant(
            &key.id,
            "grant",
            &GrantDefinition {
                task_id: task.id.clone(),
                binding: InvocationBinding {
                    kind: InvocationKind::Native,
                    adapter: "test".into(),
                    external_id: "native".into(),
                },
                operations: permissions,
                expires_at: now + 120,
            },
        )
        .unwrap();
    let validator = StoreValidator::new(store.clone());
    let principal = validator
        .validate_principal(grant.expose_secret())
        .await
        .unwrap();
    // The store/resolver boundary can be qualified before granting any raw wire
    // capability. Enabling UCTP envelope scopes remains a separate security step.
    assert_eq!(
        principal.scopes,
        vec![connect_protocol::REQUIRED_SCOPE.to_owned()]
    );
    let resolver =
        StoreSessionBindingResolver::new(store.clone(), Orchestrator::new(Default::default()))
            .unwrap();
    let wire = SessionId::from_string(&session.id);
    let canonical = resolver.resolve_session(&principal, &wire).unwrap();
    assert_eq!(
        canonical.as_str(),
        format!(
            "connect:{}:{}",
            store.installation_id().unwrap(),
            session.id
        )
    );
    resolver
        .reauthorize_session(&principal, &wire, &canonical)
        .unwrap();
    assert_eq!(
        resolver
            .reauthorize_session(&principal, &wire, &SessionId::from_string("other"))
            .unwrap_err()
            .code,
        403
    );
    assert_eq!(
        resolver
            .resolve_session(&principal, &SessionId::from_string("unknown"))
            .unwrap_err()
            .code,
        403
    );
    for field in ["issuer", "tenant", "expiry"] {
        let mut forged = principal.clone();
        match field {
            "issuer" => forged.issuer = Some("external".into()),
            "tenant" => forged.tenant = Some("other-installation".into()),
            "expiry" => forged.expires_at = chrono::DateTime::from_timestamp(0, 0),
            _ => unreachable!(),
        }
        assert_eq!(
            resolver.resolve_session(&forged, &wire).unwrap_err().code,
            403,
            "{field}"
        );
    }
    let owner_principal = validator.validate_principal(&key.token).await.unwrap();
    assert_eq!(
        resolver.resolve_session(&owner_principal, &wire).unwrap(),
        canonical
    );
    assert_eq!(
        resolver.project_credential(&key.id, &session.id).unwrap(),
        canonical
    );
    store.revoke_task_grant(&key.id, &grant.grant.id).unwrap();
    assert_eq!(
        resolver
            .reauthorize_session(&principal, &wire, &canonical)
            .unwrap_err()
            .code,
        403
    );
    assert_eq!(
        resolver.resolve_session(&owner_principal, &wire).unwrap(),
        canonical
    );
}
