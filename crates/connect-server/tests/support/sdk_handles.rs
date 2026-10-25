//! Actual production listener exercises public owned SDK handles and recovery.
use super::*;
use connect_client::sdk::{
    self, Client, ConversationPolicy, MutationOptions, NonEmpty, RequestOptions,
};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(10)).unwrap()
}
fn mutation(id: &str) -> MutationOptions {
    MutationOptions {
        operation_id: id.parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
#[tokio::test]
async fn public_sdk_conversation_private_send_recovery_and_owned_shutdown() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let creating = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("sdk-create"),
            )
            .await
            .unwrap();
        let saved_creation = creating.reference_json();
        assert!(saved_creation["scope"].as_object().unwrap().is_empty());
        let completed = creating.wait(options()).await.unwrap();
        assert_eq!(completed.snapshot.state, sdk::OperationState::Completed);
        let room = completed.value;
        let current = room.refresh(options()).await.unwrap();
        assert_eq!(current.participants.len(), 2);
        let recipient = current
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        let sending = room
            .send(
                "sdk-message".parse().unwrap(),
                NonEmpty {
                    first: recipient.clone(),
                    rest: vec![],
                },
                "Private original content".into(),
                mutation("sdk-send"),
            )
            .await
            .unwrap();
        let saved = sending.reference_json();
        let acceptance = sending.wait(options()).await.unwrap();
        assert_eq!(acceptance.value.message_id.as_str(), "sdk-message");
        assert_eq!(acceptance.value.operation.operation_id.as_str(), "sdk-send");
        let cancelled = RequestOptions {
            signal: tokio_util::sync::CancellationToken::new(),
            ..options()
        };
        cancelled.signal.cancel();
        let result = sending.wait(cancelled).await;
        assert!(
            matches!(result,Err(e) if e.kind==sdk::ErrorKind::Cancelled && e.certainty==sdk::Certainty::ReadOnly && e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        assert_eq!(
            sending.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        // Changed content keeps the original ID and is refused, never renamed.
        let conflict = room
            .send(
                "sdk-message".parse().unwrap(),
                NonEmpty {
                    first: recipient.clone(),
                    rest: vec![],
                },
                "Changed content".into(),
                mutation("sdk-send"),
            )
            .await;
        assert!(
            matches!(conflict,Err(e) if e.certainty==sdk::Certainty::Refused && e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        let mut wrong = saved.clone();
        wrong["command"] = json!("conversation.create");
        assert!(client
            .restore(wrong, client.operation_kinds().message_send)
            .is_err());
        let mut bad = saved.clone();
        bad["scope"]["session_id"] = json!("forged-session");
        assert!(client
            .restore(bad, client.operation_kinds().message_send)
            .is_err());
        let mut secret = saved.clone();
        secret["credential"] = json!("must-not-be-accepted");
        assert!(client
            .restore(secret, client.operation_kinds().message_send)
            .is_err());
        client.close(options()).await.unwrap();
        assert!(
            matches!(room.refresh(options()).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled)
        );
        // A fresh authenticated client only restores and queries the original IDs.
        let reconnected = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let original = reconnected
            .restore(saved, reconnected.operation_kinds().message_send)
            .unwrap();
        assert_eq!(
            original.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        assert_eq!(
            original
                .wait(options())
                .await
                .unwrap()
                .value
                .message_id
                .as_str(),
            "sdk-message"
        );
        let original_create = reconnected
            .restore(
                saved_creation,
                reconnected.operation_kinds().conversation_create,
            )
            .unwrap();
        assert_eq!(
            original_create.wait(options()).await.unwrap().value.id(),
            room.id()
        );
        // The canonical public history remains authoritative: one original message.
        let mut inspection = CanonicalClient::open(f.client.clone()).await.unwrap();
        let history = inspection
            .history(
                room.id().as_str(),
                serde_json::from_value(json!({"limit":100})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].msg_id.as_str(), "sdk-message");
        inspection.shutdown().await.unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
        let subject = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        f.server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::RevokeCredential {
                    operation_id: "sdk-revoke".into(),
                    credential_id: subject,
                },
            )
            .unwrap();
        let revoked = original.status(options()).await;
        assert!(
            matches!(revoked,Err(e) if e.operation.as_ref().unwrap().operation_id.as_str()=="sdk-send")
        );
        reconnected.close(options()).await.unwrap();
        reconnected.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
    }
}
