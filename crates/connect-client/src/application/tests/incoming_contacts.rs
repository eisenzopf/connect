use super::*;
use connect_protocol::incoming_contacts::*;
fn selection() -> IncomingContactCommand {
    IncomingContactCommand::Resolve(IncomingContactSelection {
        operation_id: "original-choice".into(),
        contact_id: "ich_original".into(),
        candidate_id: "icc_original".into(),
        expected_revision: 0,
    })
}
fn contact() -> IncomingContact {
    IncomingContact {
        id: "ich_original".into(),
        kind: IncomingContactKind::Voice,
        provider: "vapi-telephone".into(),
        reason: IncomingContactReason::AmbiguousRoute,
        state: IncomingContactState::Resolved,
        revision: 1,
        decision: Some(IncomingContactDecision {
            operation_id: "original-choice".into(),
            actor_identity_id: "id_original".into(),
            actor_credential_id: "credential_original".into(),
            candidate_id: Some("icc_original".into()),
            message_id: None,
            session_id: Some("session_original".into()),
            created_at: 2,
        }),
        candidates: vec![IncomingContactCandidate {
            id: "icc_original".into(),
            route_id: "ingress_original".into(),
            conversation_id: "conv_original".into(),
            endpoint_id: "endpoint_original".into(),
            participant_id: "part_original".into(),
            membership_epoch: 1,
        }],
        created_at: 1,
    }
}
async fn peer(status: &str, body: Vec<u8>) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let status = status.to_owned();
    let job = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_at(&mut stream, "/v2/incoming-contacts").await;
        let header = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(&body).await;
        request
    });
    (endpoint, job)
}
#[tokio::test]
async fn incoming_decisions_correlate_original_contact_candidate_operation_state_and_kind() {
    for change in [
        "valid",
        "contact",
        "candidate",
        "operation",
        "state",
        "medium",
        "epoch",
        "profile",
        "status",
        "unknown-field",
    ] {
        let mut response = serde_json::to_value(IncomingContactResponse {
            profile: INCOMING_CONTACT_PROFILE.into(),
            result: Ok(IncomingContactOutcome::Contact { contact: contact() }),
        })
        .unwrap();
        let c = &mut response["result"]["Ok"]["contact"];
        match change {
            "contact" => c["id"] = serde_json::json!("ich_foreign"),
            "candidate" => c["decision"]["candidate_id"] = serde_json::json!("icc_foreign"),
            "operation" => c["decision"]["operation_id"] = serde_json::json!("foreign-choice"),
            "state" => c["state"] = serde_json::json!("held"),
            "medium" => c["kind"] = serde_json::json!("sms"),
            "epoch" => c["candidates"][0]["membership_epoch"] = serde_json::json!(0),
            "unknown-field" => c["body"] = serde_json::json!("private-response-canary"),
            "profile" => response["profile"] = serde_json::json!("connect/legacy"),
            _ => {}
        }
        let (endpoint, job) = peer(
            if change == "status" {
                "503 Unavailable"
            } else {
                "200 OK"
            },
            serde_json::to_vec(&response).unwrap(),
        )
        .await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let result = client.incoming_contact_command(selection()).await;
        if change == "valid" {
            assert!(result.is_ok());
        } else {
            assert!(
                matches!(result, Err(ClientError::Protocol)),
                "{change}: {result:?}"
            );
        }
        let command: IncomingContactCommand = serde_json::from_slice(&job.await.unwrap()).unwrap();
        assert_eq!(command, selection());
    }
}
#[tokio::test]
async fn incoming_provider_errors_are_sanitized_and_never_retried_with_another_operation() {
    for code in [
        "operation-pending",
        "provider-unavailable",
        "provider-binding-conflict",
        "transport-uncertain",
    ] {
        let body = serde_json::to_vec(&IncomingContactResponse {
            profile: INCOMING_CONTACT_PROFILE.into(),
            result: Err(Error::new(code, "private-provider-control-url-canary")),
        })
        .unwrap();
        let (endpoint, job) = peer("503 Unavailable", body).await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let error = client
            .incoming_contact_command(selection())
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("canary"));
        assert!(matches!(error,ClientError::Application(e) if e.code==code));
        assert_eq!(
            serde_json::from_slice::<IncomingContactCommand>(&job.await.unwrap()).unwrap(),
            selection()
        );
    }
}
#[tokio::test]
async fn incoming_invalid_cancelled_and_expired_commands_have_no_http_effect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = ApplicationClient::open(configuration(listener.local_addr().unwrap()))
        .await
        .unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        client
            .incoming_contact_command_with_cancel(selection(), cancelled)
            .await,
        Err(ClientError::Cancelled)
    ));
    assert!(
        matches!(client.incoming_contact_command(IncomingContactCommand::List {after:None,limit:33}).await,Err(ClientError::Application(error)) if error.code=="invalid-request")
    );
    assert!(
        matches!(client.incoming_contact_command(IncomingContactCommand::Reject {operation_id:"reject".into(),contact_id:"ich_original".into(),expected_revision:0,reason:"  ".into()}).await,Err(ClientError::Application(error)) if error.code=="invalid-request")
    );
    client.config.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(matches!(
        client.incoming_contact_command(selection()).await,
        Err(ClientError::Authentication)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}
