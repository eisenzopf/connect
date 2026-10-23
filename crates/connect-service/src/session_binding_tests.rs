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
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    peer.clone()
                ]),
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
    assert_eq!(
        principal.scopes,
        vec![
            connect_protocol::REQUIRED_SCOPE.to_owned(),
            crate::NATIVE_MEDIA_SCOPE.to_owned()
        ]
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
        assert!(
            resolver
                .session_is_terminal(&forged, &wire, &canonical)
                .is_err(),
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
    // Retain the original live-Session delegated revocation case, then use a
    // separate current grant for terminal classification and its later revoke.
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
    let grant = store
        .issue_local_task_grant(
            &key.id,
            "terminal-grant",
            &GrantDefinition {
                task_id: task.id,
                binding: InvocationBinding {
                    kind: InvocationKind::Native,
                    adapter: "test".into(),
                    external_id: "native-terminal".into(),
                },
                operations: grant.grant.operations.clone(),
                expires_at: now + 120,
            },
        )
        .unwrap();
    let principal = validator
        .validate_principal(grant.expose_secret())
        .await
        .unwrap();
    assert!(!resolver
        .session_is_terminal(&principal, &wire, &canonical)
        .unwrap());
    store
        .execute(
            &peer_key.id,
            &Operation::SessionEnd {
                request_id: "end-native-session".into(),
                conversation_id: conversation.id,
                session_id: session.id.clone(),
                reason: None,
            },
        )
        .unwrap();
    assert!(resolver
        .session_is_terminal(&principal, &wire, &canonical)
        .unwrap());
    assert!(resolver
        .session_is_terminal(&owner_principal, &wire, &canonical)
        .unwrap());
    assert!(resolver
        .reauthorize_session(&principal, &wire, &canonical)
        .is_err());
    assert!(resolver
        .session_is_terminal(&principal, &wire, &SessionId::from_string("other"))
        .is_err());
    store.revoke_task_grant(&key.id, &grant.grant.id).unwrap();
    assert_eq!(
        resolver
            .reauthorize_session(&principal, &wire, &canonical)
            .unwrap_err()
            .code,
        403
    );
    assert!(resolver
        .session_is_terminal(&principal, &wire, &canonical)
        .is_err());
    assert!(resolver.reauthorize_peer(&principal).is_err());
    resolver.reauthorize_peer(&owner_principal).unwrap();
    assert!(resolver
        .session_is_terminal(&owner_principal, &wire, &canonical)
        .unwrap());
}

#[tokio::test]
async fn native_video_grant_admits_exact_profile_and_rejects_duplicate_unsupported_streams() {
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
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    peer.clone()
                ]),
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
                    media: vec!["audio".into(), "video".into()],
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
    assert_eq!(
        principal.scopes,
        vec![
            connect_protocol::REQUIRED_SCOPE.to_owned(),
            crate::NATIVE_MEDIA_SCOPE.to_owned()
        ]
    );
    let resolver =
        StoreSessionBindingResolver::new(store.clone(), Orchestrator::new(Default::default()))
            .unwrap();
    use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
    let payload = serde_json::json!({"by_participant":principal.subject,"substrate":"quic","capabilities":{},"streams_offered":[
        {"id":"audio-main","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},
        {"id":"video-main","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}],"substrate_setup":null});
    for identity in [
        principal.clone(),
        validator.validate_principal(&key.token).await.unwrap(),
    ] {
        let mut exact = payload.clone();
        exact["by_participant"] = serde_json::json!(identity.subject);
        let offer = UctpEnvelope::new(MessageType::ConnectionOffer, exact.clone())
            .with_sid(&session.id)
            .with_connid("video-connection");
        resolver.authorize_envelope(&identity, &offer).unwrap();
        let wrong_conversation = UctpEnvelope::new(MessageType::ConnectionOffer, exact.clone())
            .with_sid(&session.id)
            .with_cid("another-conversation")
            .with_connid("video-connection");
        assert_eq!(
            resolver
                .authorize_envelope(&identity, &wrong_conversation)
                .unwrap_err()
                .code,
            403
        );
        for invalid in [
            "duplicate-id",
            "second-video",
            "unsupported-codec",
            "one-way",
        ] {
            let mut bad = exact.clone();
            match invalid {
                "duplicate-id" => bad["streams_offered"][1]["id"] = serde_json::json!("audio-main"),
                "second-video" => {
                    let extra = bad["streams_offered"][1].clone();
                    bad["streams_offered"].as_array_mut().unwrap().push(extra);
                }
                "unsupported-codec" => {
                    bad["streams_offered"][1]["codec_preferences"] = serde_json::json!(["h264"])
                }
                "one-way" => bad["streams_offered"][1]["direction"] = serde_json::json!("sendonly"),
                _ => unreachable!(),
            }
            let offer = UctpEnvelope::new(MessageType::ConnectionOffer, bad)
                .with_sid(&session.id)
                .with_connid("video-connection");
            assert_eq!(
                resolver
                    .authorize_envelope(&identity, &offer)
                    .unwrap_err()
                    .code,
                403,
                "{invalid}"
            );
        }
    }
    let invite = UctpEnvelope::new(MessageType::SessionInvite, serde_json::json!({"from":principal.subject,"to":[session.participant_id],"medium":"voice-video","intent":"connect-session","capabilities_offer":{"audio":["g.711-mu"],"video":["vp8"]}})).with_cid(&conversation.id).with_sid(&session.id);
    resolver.authorize_envelope(&principal, &invite).unwrap();
    store.revoke_task_grant(&key.id, &grant.grant.id).unwrap();
    assert_eq!(
        resolver
            .authorize_envelope(&principal, &invite)
            .unwrap_err()
            .code,
        403
    );
}
