use super::*;
use connect_protocol::credential_issuance::{CredentialIssueOutcome, CredentialIssueRequest};
use serde_json::json;
fn request() -> CredentialIssueRequest {
    CredentialIssueRequest {
        operation_id: "issue-original".into(),
        identity_id: "agent".into(),
        operations: vec!["operation.get".into()],
        recipients: vec!["peer".into()],
        expires_at: chrono::Utc::now().timestamp() + 30,
    }
}
#[tokio::test]
async fn issuance_checks_current_profile_intent_secret_and_server_bindings_without_retry() {
    for fault in [
        "valid",
        "profile",
        "operation_id",
        "identity_id",
        "operations",
        "recipients",
        "expires_at",
        "issued_at",
        "future-issued",
        "expired",
        "credential_id",
        "issuer_identity_id",
        "issuer_credential_id",
        "installation_id",
        "credential",
        "unknown",
        "status",
        "error",
    ] {
        let input = request();
        let mut value = serde_json::to_value(CredentialIssueOutcome {
            operation_id: input.operation_id.clone(),
            identity_id: input.identity_id.clone(),
            operations: input.operations.clone(),
            recipients: input.recipients.clone(),
            expires_at: input.expires_at,
            issued_at: chrono::Utc::now().timestamp(),
            credential: "a".repeat(64),
            credential_id: uuid::Uuid::new_v4().to_string(),
            issuer_identity_id: uuid::Uuid::new_v4().to_string(),
            issuer_credential_id: uuid::Uuid::new_v4().to_string(),
            installation_id: uuid::Uuid::new_v4().to_string(),
        })
        .unwrap();
        match fault {
            "operation_id"
            | "identity_id"
            | "credential_id"
            | "issuer_identity_id"
            | "issuer_credential_id"
            | "installation_id" => value[fault] = json!("wrong"),
            "operations" => value[fault] = json!(["admin"]),
            "recipients" => value[fault] = json!(["other"]),
            "expires_at" => value[fault] = json!(input.expires_at + 1),
            "issued_at" => value[fault] = json!(0),
            "future-issued" => value["issued_at"] = json!(input.expires_at + 1),
            "expired" => value["expires_at"] = json!(chrono::Utc::now().timestamp() - 1),
            "credential" => value[fault] = json!("A".repeat(64)),
            "unknown" => value["user_authorization"] = json!({"user_id":"forged"}),
            _ => {}
        }
        let body=serde_json::to_vec(&json!({"profile":if fault=="profile"{"other/1"}else{"connect/2"},"result":if fault=="error"{json!({"Err":{"code":"forbidden","message":"private-issuance-canary"}})}else{json!({"Ok":value})}})).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let header = format!(
            "HTTP/1.1 {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            if fault == "status" {
                "403 Forbidden"
            } else {
                "200 OK"
            },
            body.len()
        );
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let bytes = read_at(&mut stream, "/v2/credentials").await;
            stream.write_all(header.as_bytes()).await.unwrap();
            let _ = stream.write_all(&body).await;
            bytes
        });
        let mut client = ApplicationClient::open(configuration(address))
            .await
            .unwrap();
        let result = client.issue_scoped_credential(input.clone()).await;
        if fault == "valid" {
            assert_eq!(result.unwrap().operation_id, input.operation_id);
        } else if fault == "error" {
            let error = result.unwrap_err();
            assert!(!format!("{error:?}").contains("private-issuance-canary"));
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)), "{fault}");
        }
        assert_eq!(
            serde_json::from_slice::<CredentialIssueRequest>(&peer.await.unwrap()).unwrap(),
            input
        );
    }
}
#[tokio::test]
async fn issuance_cancellation_invalid_intent_and_task_mode_refuse_before_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = ApplicationClient::open(configuration(listener.local_addr().unwrap()))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        client
            .issue_scoped_credential_with_cancel(request(), cancel)
            .await,
        Err(ClientError::Cancelled)
    ));
    let mut invalid = request();
    invalid.operations = vec!["admin".into()];
    assert!(client.issue_scoped_credential(invalid).await.is_err());
    client.task_profile = true;
    assert!(matches!(
        client.issue_scoped_credential(request()).await,
        Err(ClientError::Configuration)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}
