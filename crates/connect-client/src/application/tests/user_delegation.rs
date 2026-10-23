use super::*;
use connect_protocol::user_delegation::*;
fn request() -> UserDelegationRequest {
    UserDelegationRequest {
        operation_id: "consent-one".into(),
        identity_id: "agent-one".into(),
        operations: vec!["send".into()],
        recipients: vec!["human-one".into()],
        expires_at: chrono::Utc::now().timestamp() + 30,
    }
}
#[tokio::test]
async fn explicit_user_consent_correlates_issuer_target_scope_expiry_and_protected_secret() {
    for fault in [
        "valid",
        "profile",
        "operation",
        "user",
        "identity",
        "operations",
        "recipients",
        "expiry",
        "credential-id",
        "secret",
        "unknown-field",
        "status",
    ] {
        let input = request();
        let mut value = serde_json::json!({"operation_id":input.operation_id,"credential_id":uuid::Uuid::new_v4().to_string(),"identity_id":input.identity_id,"user_id":"human-one","operations":input.operations,"recipients":input.recipients,"expires_at":input.expires_at,"credential":"a".repeat(64)});
        match fault {
            "operation" => value["operation_id"] = serde_json::json!("other-operation"),
            "user" => value["user_id"] = serde_json::json!("other-human"),
            "identity" => value["identity_id"] = serde_json::json!("other-agent"),
            "operations" => value["operations"] = serde_json::json!(["admin"]),
            "recipients" => value["recipients"] = serde_json::json!(["extra-peer"]),
            "expiry" => value["expires_at"] = serde_json::json!(input.expires_at + 1),
            "credential-id" => value["credential_id"] = serde_json::json!("invalid"),
            "secret" => value["credential"] = serde_json::json!("A".repeat(64)),
            "unknown-field" => value["user_assertion"] = serde_json::json!("ignored-authority"),
            _ => {}
        }
        let body=serde_json::to_vec(&serde_json::json!({"profile":if fault=="profile" {"other/1"}else{"connect/2"},"result":{"Ok":value}})).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = if fault == "status" {
            "403 Forbidden"
        } else {
            "200 OK"
        };
        let header = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let received = read_at(&mut stream, "/v2/user-delegations").await;
            stream.write_all(header.as_bytes()).await.unwrap();
            let _ = stream.write_all(&body).await;
            received
        });
        let mut client = ApplicationClient::open(configuration(address))
            .await
            .unwrap();
        let result = client
            .issue_user_delegation("human-one", input.clone())
            .await;
        if fault == "valid" {
            let value = result.unwrap();
            assert_eq!(value.user_id, "human-one");
            assert!(!format!("{value:?}").contains(&value.credential));
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)), "{fault}");
        }
        let received: UserDelegationRequest = serde_json::from_slice(&peer.await.unwrap()).unwrap();
        assert_eq!(received, input);
    }
}
#[tokio::test]
async fn user_consent_invalid_intent_and_presend_cancel_do_not_send_or_choose_fallback_authority() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = ApplicationClient::open(configuration(listener.local_addr().unwrap()))
        .await
        .unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        client
            .issue_user_delegation_with_cancel("human-one", request(), cancellation)
            .await,
        Err(ClientError::Cancelled)
    ));
    let mut invalid = request();
    invalid.operations = vec!["*".into()];
    assert!(client
        .issue_user_delegation("human-one", invalid)
        .await
        .is_err());
    client.task_profile = true;
    assert!(matches!(
        client.issue_user_delegation("human-one", request()).await,
        Err(ClientError::Configuration)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}
