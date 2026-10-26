//! Named media permissions and their protocol floor; no event-right inheritance.
#![allow(clippy::unwrap_used)]
use connect_protocol::Limits;
use connect_service::StoreValidator;
use connect_store::{Scope, Store};
use rvoip_auth_core::BearerValidator;
use std::sync::Arc;

#[tokio::test]
async fn named_native_subscription_credentials_receive_their_protocol_floor_without_event_authority(
) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&temp.path().join("state.sqlite3"), Limits::default()).unwrap());
    let id = store
        .create_identity("native-subscription-owner")
        .unwrap()
        .id;
    for operation in [
        "stream.subscribe",
        "stream.unsubscribe",
        "conversation.subscribe",
        "connection.offer",
    ] {
        let key = store
            .issue(
                &id,
                Scope {
                    operations: vec![operation.into()],
                    recipients: vec![id.clone()],
                },
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        let credential = StoreValidator::canonical(store.clone())
            .validate_credential(&key.token)
            .await
            .unwrap();
        assert_eq!(
            credential
                .principal
                .has_scope(rvoip_uctp::state::UCTP_SUBSCRIBE_SCOPE),
            operation != "connection.offer"
        );
        let actor = store.revalidate(&key.id).unwrap();
        assert_eq!(
            actor.permits("conversation.subscribe"),
            operation == "conversation.subscribe"
        );
        assert_eq!(
            actor.permits("stream.subscribe"),
            operation == "stream.subscribe"
        );
        assert_eq!(
            actor.permits("stream.unsubscribe"),
            operation == "stream.unsubscribe"
        );
        assert!(!credential
            .principal
            .has_scope(rvoip_uctp::state::UCTP_DATA_SCOPE));
        assert_eq!(
            credential
                .principal
                .has_scope(rvoip_uctp::state::UCTP_SESSION_SCOPE),
            operation == "connection.offer"
        );
    }
}
