use super::*;
#[derive(Clone, Copy)]
pub(super) enum RefreshReply {
    Valid,
    ForeignOwner,
    WrongProfile,
    WrongAssurance,
    Expired,
    EmptyToken,
    UnchangedToken,
    Refused,
    ForeignReply,
    Missing,
    Streaming,
}
impl RefreshReply {
    pub(super) fn value(self, request: &str) -> Value {
        let mut value = envelope(
            "auth.session",
            json!({"identity_id":"id_one","session_token":"renewed-private-session-canary","expires_at":chrono::Utc::now()+chrono::Duration::minutes(2),"assurance":"identified","application_profile":"connect/2"}),
        );
        match self {
            Self::ForeignOwner => value["payload"]["identity_id"] = json!("id_foreign"),
            Self::WrongProfile => value["payload"]["application_profile"] = json!("foreign/1"),
            Self::WrongAssurance => value["payload"]["assurance"] = json!("verified"),
            Self::Expired => {
                value["payload"]["expires_at"] =
                    json!(chrono::Utc::now() - chrono::Duration::seconds(1))
            }
            Self::EmptyToken => value["payload"]["session_token"] = json!(""),
            Self::UnchangedToken => {
                value["payload"]["session_token"] = json!("controlled-post-auth")
            }
            Self::Refused => {
                value = envelope(
                    "error",
                    json!({"code":403,"category":"auth","reason":"private-refusal-canary"}),
                )
            }
            _ => {}
        }
        value["in_reply_to"] = json!(if matches!(self, Self::ForeignReply | Self::Streaming) {
            "unrelated-auth-request"
        } else {
            request
        });
        value
    }
}
#[tokio::test]
async fn canonical_refresh_validates_signed_owner_profile_assurance_expiry_and_rotated_token() {
    for reply in [
        RefreshReply::Valid,
        RefreshReply::ForeignOwner,
        RefreshReply::WrongProfile,
        RefreshReply::WrongAssurance,
        RefreshReply::Expired,
        RefreshReply::EmptyToken,
        RefreshReply::UnchangedToken,
    ] {
        let (mut client, server, _) = controlled(Fault::Refresh(reply)).await;
        let original_token = client.session_token.clone();
        let original_expiry = client.authenticated_until;
        // The server's later expiry remains bounded by the configured pin.
        let pin = chrono::Utc::now() + chrono::Duration::seconds(50);
        client.server_key_expires_at = pin;
        let result = client
            .refresh_credential("explicit-successor-canary".into())
            .await;
        if matches!(reply, RefreshReply::Valid) {
            let view = result.unwrap();
            assert_eq!(view.identity_id, "id_one");
            assert_eq!(view.assurance, "identified");
            assert_eq!(view.expires_at, pin);
            assert_eq!(view.application_profile, "connect/2");
            assert!(!format!("{view:?}").contains("canary"));
            assert_ne!(client.session_token, original_token);
            assert_eq!(client.authenticated_until, pin);
            assert!(!client.closed.load(Ordering::Acquire));
        } else {
            assert!(matches!(
                result,
                Err(ClientError::Authentication | ClientError::Protocol)
            ));
            assert!(client.closed.load(Ordering::Acquire));
            assert_eq!(client.session_token, original_token);
            assert_eq!(client.authenticated_until, original_expiry);
        }
        client.close();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn canonical_refresh_correlated_refusal_preserves_the_previous_binding_without_echoing_backend_text(
) {
    let (mut client, server, _) = controlled(Fault::Refresh(RefreshReply::Refused)).await;
    let old = (client.session_token.clone(), client.authenticated_until);
    let error = client
        .refresh_credential("explicit-successor-canary".into())
        .await
        .unwrap_err();
    assert!(matches!(&error,ClientError::Application(error) if error.code=="forbidden"));
    assert!(!format!("{error:?}").contains("canary"));
    assert!(!client.closed.load(Ordering::Acquire));
    assert_eq!(
        (client.session_token.clone(), client.authenticated_until),
        old
    );
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn canonical_refresh_missing_foreign_and_streaming_replies_cannot_extend_the_old_absolute_deadline(
) {
    for reply in [
        RefreshReply::Missing,
        RefreshReply::ForeignReply,
        RefreshReply::Streaming,
    ] {
        let (mut client, server, _) = controlled(Fault::Refresh(reply)).await;
        let old = client.session_token.clone();
        client.authenticated_until = chrono::Utc::now() + chrono::Duration::milliseconds(100);
        let start = tokio::time::Instant::now();
        assert!(matches!(
            client.refresh_credential("explicit-successor".into()).await,
            Err(ClientError::Timeout | ClientError::Authentication)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(client.closed.load(Ordering::Acquire));
        assert_eq!(client.session_token, old);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn canonical_refresh_invalid_input_and_pre_cancelled_intent_send_no_command() {
    let (mut client, server, ready) = controlled(Fault::Refresh(RefreshReply::Valid)).await;
    for token in ["".to_owned(), "line\nbreak".into(), "x".repeat(257)] {
        assert!(matches!(
            client.refresh_credential(token).await,
            Err(ClientError::Configuration)
        ));
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        client
            .refresh_credential_with_cancel("explicit-successor".into(), cancel)
            .await,
        Err(ClientError::Cancelled)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), ready.notified())
            .await
            .is_err()
    );
    assert!(!client.closed.load(Ordering::Acquire));
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
