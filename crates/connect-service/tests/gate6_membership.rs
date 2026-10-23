//! Authenticated ordinary-agent composition uses the current store policy.
#![allow(clippy::unwrap_used)]
use connect_protocol::{membership::*, provisioning::IdentityKind, Limits, Operation, Outcome};
use connect_service::{Service, StoreValidator};
use connect_store::{Scope, Store};
use rvoip_auth_core::BearerValidator;
use std::sync::Arc;

#[tokio::test]
async fn ordinary_agent_membership_checks_principal_origin_and_current_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let alice = store.create_identity("alice").unwrap().id;
    let expires = chrono::Utc::now().timestamp() + 600;
    let administrator = store.issue(&alice, Scope::operator(), expires).unwrap();
    let connect_protocol::provisioning::ProvisioningOutcome::Identity {
        identity_id: agent, ..
    } = store
        .execute_provisioning(
            &administrator.id,
            connect_protocol::provisioning::ProvisioningCommand::Identity {
                operation_id: "agent".into(),
                alias: "agent".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!("agent")
    };
    let owner = store
        .issue(&alice, Scope::collaboration(vec!["*".into()]), expires)
        .unwrap();
    let peer = store
        .issue(&agent, Scope::collaboration(vec!["*".into()]), expires)
        .unwrap();
    let Outcome::Conversation(conversation) = store
        .execute(
            &owner.id,
            &Operation::Create {
                request_id: "conversation".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
            },
        )
        .unwrap()
    else {
        panic!("conversation")
    };
    let service = Service::new(store.clone());
    let validator = StoreValidator::new(store.clone());
    let principal = validator.validate_principal(&owner.token).await.unwrap();
    let request = MembershipRequest {
        operation_id: "invite".into(),
        conversation_id: conversation.id.clone(),
        expected_revision: 0,
        change: MembershipAction::Invite {
            identity_id: agent.clone(),
            kind: IdentityKind::Ai,
            role: "agent".into(),
            expires_at: expires,
        },
    };
    for case in 0..4 {
        let mut wrong = principal.clone();
        match case {
            0 => wrong.issuer = Some("other-issuer".into()),
            1 => wrong.tenant = Some("other-installation".into()),
            2 => wrong.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
            _ => {
                wrong.subject =
                    format!("{}reserved", connect_protocol::tasks::TASK_PRINCIPAL_PREFIX)
            }
        }
        assert_eq!(
            service
                .execute_membership_authenticated(wrong, request.clone())
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(
            service
                .membership_snapshot(owner.id.clone(), conversation.id.clone())
                .await
                .unwrap()
                .revision,
            0
        );
    }
    let MembershipOutcome::Invitation { invitation, .. } = service
        .execute_membership_authenticated(principal.clone(), request.clone())
        .await
        .unwrap()
    else {
        panic!("invite")
    };
    let invitations = service
        .membership_invitations(peer.id.clone(), None, 10)
        .await
        .unwrap();
    assert_eq!(invitations.invitations.len(), 1);
    let peer_principal = validator.validate_principal(&peer.token).await.unwrap();
    let join = MembershipRequest {
        operation_id: "join".into(),
        conversation_id: conversation.id.clone(),
        expected_revision: 1,
        change: MembershipAction::Join {
            invitation_id: invitation.invitation_id,
        },
    };
    let MembershipOutcome::Participant {
        participant,
        revision,
        ..
    } = service
        .execute_membership_authenticated(peer_principal.clone(), join.clone())
        .await
        .unwrap()
    else {
        panic!("join")
    };
    assert_eq!(participant.identity_id, agent);
    assert_eq!(revision, 2);
    // No task or task grant was created to give the agent normal conversation access.
    assert_eq!(
        service
            .membership_snapshot(peer.id.clone(), conversation.id.clone())
            .await
            .unwrap()
            .participants
            .len(),
        2
    );
    let administrator = store.issue(&alice, Scope::operator(), expires).unwrap();
    store
        .revoke(&store.authenticate(&administrator.token).unwrap(), &peer.id)
        .unwrap();
    assert!(service
        .execute_membership_authenticated(peer_principal, join)
        .await
        .is_err());
    assert!(service
        .membership_snapshot(peer.id, conversation.id.clone())
        .await
        .is_err());
    // The retained authenticated principal cannot bypass a later credential revoke.
    store
        .revoke(
            &store.authenticate(&administrator.token).unwrap(),
            &owner.id,
        )
        .unwrap();
    assert!(service
        .execute_membership_authenticated(principal, request)
        .await
        .is_err());
}
