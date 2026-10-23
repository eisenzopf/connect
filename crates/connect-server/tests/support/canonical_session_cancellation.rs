//! Original caller references are physical metadata; current Store authority and
//! the ordinary logical operation remain authoritative for cancellation.
use super::*;
use connect_client::{
    canonical::{CanonicalHttpClient, SessionInvitationAnswer},
    conversations::CreateRequest,
    session_control::{Action, Request},
};
fn cancel(cid: &str, sid: &str, operation: &str, revision: u64) -> Request {
    Request {
        operation_id: operation.into(),
        conversation_id: cid.into(),
        session_id: sid.into(),
        by_participant: None,
        expected_revision: Some(revision),
        action: Action::Cancel {
            reason_code: 487,
            reason: "request-cancelled".into(),
        },
    }
}
async fn invite(owner: &mut CanonicalClient, cid: &str, target: &str, sid: &str) -> String {
    let batch=owner.request_value("session.invite",json!({"to":[target],"medium":"voice","intent":"synchronous-engagement","capabilities_offer":{"audio":["opus"]}}),CommandScope{conversation_id:Some(cid.into()),session_id:Some(sid.into()),operation_id:Some(format!("{sid}-invite")),..Default::default()}).await.unwrap();
    batch.ack().unwrap();
    batch.request_id
}
fn mutations(f: &Fixture, id: &str) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}
#[tokio::test]
async fn original_caller_cancellation_references_require_physical_scope_and_recover_with_logical_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 16, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut sibling = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut create = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("caller-cancel-room", create)
            .await
            .unwrap();
        let from = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let target = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            246,
            vec!["data".into()],
            Scope::collaboration(vec![f.client.identity_id.clone()]),
        );
        let mut peer = CanonicalClient::open(config).await.unwrap();
        let first = invite(&mut owner, &room.id, &target, "caller-cancel-first").await;
        let ia = peer.receive_session_invitation().await.unwrap();
        let second = invite(&mut owner, &room.id, &target, "caller-cancel-second").await;
        let ib = peer.receive_session_invitation().await.unwrap();
        let mut request = cancel(&room.id, "caller-cancel-first", "caller-cancel-owned", 0);
        request.by_participant = Some(from);
        assert!(
            matches!(sibling.cancel_session_invitation(&first,request.clone()).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        assert_eq!(mutations(&f, &request.operation_id), 0);
        let wrong = cancel(
            &room.id,
            "caller-cancel-second",
            "caller-cancel-wrong-session",
            0,
        );
        let wrong_result = owner.cancel_session_invitation(&first, wrong.clone()).await;
        assert!(
            matches!(wrong_result,Err(ClientError::Application(ref e)) if e.code=="forbidden"),
            "wrong Session result: {wrong_result:?}"
        );
        assert_eq!(mutations(&f, &wrong.operation_id), 0);
        // An unsigned HTTP request cannot supply custody of a native reference.
        let http =
            tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
                .await
                .unwrap();
        let response=http.post(format!("http://{}/v2/commands",f.server.http_address.unwrap())).bearer_auth(&f.client.credential).json(&json!({"v":2,"type":"session.cancel","id":"caller-http-reference","ts":chrono::Utc::now(),"cid":room.id,"sid":"caller-cancel-first","operation_id":"caller-http-reference","in_reply_to":first,"payload":{"reason_code":487,"reason":"request-cancelled"}})).send().await.unwrap();
        assert!(response.status().is_client_error());
        assert_eq!(mutations(&f, "caller-http-reference"), 0);
        let mut spoofed = request.clone();
        spoofed.operation_id = "caller-cancel-spoofed-by".into();
        spoofed.by_participant = Some(target.clone());
        assert!(
            matches!(owner.cancel_session_invitation(&first, spoofed.clone()).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        assert_eq!(mutations(&f, &spoofed.operation_id), 0);
        let current = invite(&mut owner, &room.id, &target, "caller-cancel-first").await;
        assert_ne!(current, first);
        assert!(
            matches!(owner.cancel_session_invitation(&first,request.clone()).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        assert_eq!(mutations(&f, &request.operation_id), 0);
        let original = owner
            .cancel_session_invitation(&current, request.clone())
            .await
            .unwrap();
        assert_eq!(original.state, "cancelled");
        assert_eq!(original.revision, 1);
        let hint = peer.receive().await.unwrap();
        assert_eq!(hint.kind(), "session.cancel");
        assert_eq!(hint.scope(rvoip_uctp::v2::Scope::Reply), Some(ia.id()));
        assert_eq!(
            owner
                .cancel_session_invitation(&current, request.clone())
                .await
                .unwrap(),
            original
        );
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        assert_eq!(
            http.session_command(request.clone()).await.unwrap(),
            Outcome::SessionDecision(original)
        );
        assert_eq!(mutations(&f, &request.operation_id), 1);
        owner.close();
        drop(owner);
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let first_recovery = invite(&mut owner, &room.id, &target, "caller-cancel-first").await;
        let saved = owner
            .cancel_session_invitation(&first_recovery, request.clone())
            .await
            .unwrap();
        assert_eq!(saved.state, "cancelled");
        assert_eq!(saved.revision, 1);
        assert_eq!(mutations(&f, &request.operation_id), 1);
        let request = cancel(
            &room.id,
            "caller-cancel-second",
            "caller-cancel-after-reconnect",
            0,
        );
        assert!(
            matches!(owner.cancel_session_invitation(&second,request.clone()).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        assert_eq!(mutations(&f, &request.operation_id), 0);
        let recovered = invite(&mut owner, &room.id, &target, "caller-cancel-second").await;
        assert_ne!(second, recovered);
        let cancelled = owner
            .cancel_session_invitation(&recovered, request.clone())
            .await
            .unwrap();
        assert_eq!(cancelled.revision, 1);
        let hint = peer.receive().await.unwrap();
        assert_eq!(hint.kind(), "session.cancel");
        assert_eq!(hint.scope(rvoip_uctp::v2::Scope::Reply), Some(ib.id()));
        assert_eq!(
            owner
                .cancel_session_invitation(&recovered, request)
                .await
                .unwrap(),
            cancelled
        );
        let third = invite(&mut owner, &room.id, &target, "caller-cancel-accepted").await;
        let accepted = peer.receive_session_invitation().await.unwrap();
        peer.answer_session_invitation(
            &accepted,
            SessionInvitationAnswer::Accept {
                capabilities: json!({"audio":["opus"]}),
            },
        )
        .await
        .unwrap();
        assert_eq!(owner.receive().await.unwrap().kind(), "session.accept");
        let request = cancel(
            &room.id,
            "caller-cancel-accepted",
            "caller-cancel-too-late",
            1,
        );
        assert!(
            matches!(owner.cancel_session_invitation(&third,request).await,Err(ClientError::Application(ref e)) if e.code=="conflict")
        );
        owner
            .session_command(Request {
                operation_id: "caller-end-accepted".into(),
                conversation_id: room.id.clone(),
                session_id: "caller-cancel-accepted".into(),
                by_participant: None,
                expected_revision: Some(1),
                action: Action::End {
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                },
            })
            .await
            .unwrap();
        sibling.close();
        peer.close();
        owner.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn correlated_cancellation_step_up_keeps_the_owned_reference_and_replays_one_current_authorized_mutation(
) {
    use connect_client::application::ApplicationClient;
    use connect_protocol::user_delegation::{UserAssuranceRule, UserDelegationRequest};
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut create = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("caller-cancel-consent-room", create)
            .await
            .unwrap();
        owner.close();
        drop(owner);
        let user = f
            .server
            .store
            .create_identity("caller-cancel-consenting-human")
            .unwrap()
            .id;
        let human = f
            .server
            .store
            .issue(
                &user,
                Scope {
                    operations: vec!["credential.delegate".into(), "session.cancel".into()],
                    recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
                },
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        f.config.bind = f.server.address;
        f.config.http_bind = f.server.http_address;
        f.config.user_assurance = vec![UserAssuranceRule {
            conversation_id: room.id.clone(),
            command: "session.cancel".into(),
            user_id: user.clone(),
        }];
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            247,
            vec!["data".into()],
            Scope::collaboration(vec![f.client.identity_id.clone()]),
        );
        let mut peer = CanonicalClient::open(config).await.unwrap();
        let reference = invite(
            &mut owner,
            &room.id,
            &f.peer.identity_id,
            "caller-cancel-consent",
        )
        .await;
        let invitation = peer.receive_session_invitation().await.unwrap();
        let request = cancel(
            &room.id,
            "caller-cancel-consent",
            "caller-cancel-held-operation",
            0,
        );
        let blocked_result = owner
            .cancel_session_invitation(&reference, request.clone())
            .await;
        assert!(
            matches!(blocked_result,Err(ClientError::Application(ref e)) if e.code=="step-up-required"),
            "blocked result: {blocked_result:?}"
        );
        assert_eq!(owner.step_up_required(), Some("user-authorized"));
        assert_eq!(mutations(&f, &request.operation_id), 0);
        let mut human_config = canonical_http_config(&f);
        human_config.credential = human.token;
        let mut application = ApplicationClient::open(human_config).await.unwrap();
        let consent = application
            .issue_user_delegation(
                &user,
                UserDelegationRequest {
                    operation_id: "caller-cancel-consent-receipt".into(),
                    identity_id: f.client.identity_id.clone(),
                    operations: vec!["session.cancel".into()],
                    recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
                    expires_at: chrono::Utc::now().timestamp() + 300,
                },
            )
            .await
            .unwrap();
        let original = f.server.store.authenticate(&f.client.credential).unwrap();
        let key = f
            .server
            .store
            .resolve_peer_signing_key(
                &original.credential_id,
                "tnt_default",
                &f.client.signing_key_id,
                &f.client.signing_algorithm,
            )
            .unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        f.server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: "caller-cancel-successor-key".into(),
                    identity_id: f.client.identity_id.clone(),
                    credential_id: consent.credential_id,
                    device_id: f.client.device_id.clone(),
                    key_id: f.client.signing_key_id.clone(),
                    algorithm: if algorithm == "ES256" {
                        SigningAlgorithm::ES256
                    } else {
                        SigningAlgorithm::EdDSA
                    },
                    public_key: URL_SAFE_NO_PAD.encode(key.public_key),
                    not_before: key.not_before,
                    expires_at: key.expires_at,
                    expected_revision: Some(f.key_revision),
                },
            )
            .unwrap();
        let completed = owner
            .complete_step_up(consent.credential.clone())
            .await
            .unwrap();
        assert_eq!(completed.authentication.assurance, "user-authorized");
        completed.replies.ack().unwrap();
        assert!(completed
            .replies
            .frames
            .iter()
            .any(|e| e.kind() == "operation.updated"
                && e.original()["payload"]["operation_id"] == request.operation_id
                && e.original()["payload"]["state"] == "completed"));
        assert_eq!(mutations(&f, &request.operation_id), 1);
        let replay = owner
            .cancel_session_invitation(&reference, request.clone())
            .await
            .unwrap();
        assert_eq!(replay.revision, 1);
        let hint = peer.receive().await.unwrap();
        assert_eq!(hint.kind(), "session.cancel");
        assert_eq!(
            hint.scope(rvoip_uctp::v2::Scope::Reply),
            Some(invitation.id())
        );
        let mut current_http = canonical_http_config(&f);
        current_http.credential = consent.credential;
        let mut http = CanonicalHttpClient::open(current_http).await.unwrap();
        assert_eq!(
            http.session_command(request.clone()).await.unwrap(),
            Outcome::SessionDecision(replay)
        );
        assert_eq!(mutations(&f, &request.operation_id), 1);
        owner.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}
