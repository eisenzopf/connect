//! Live canonical HTTP failure and recovery cases using the production listener.
use super::*;
use connect_client::canonical::CanonicalHttpClient;

fn wire(kind: &str, body: Value, cid: Option<&str>, operation: Option<&str>) -> Value {
    let mut value = json!({"v":2,"type":kind,"id":format!("http_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"payload":body});
    if kind.starts_with("session.") {
        value["sid"] = json!("future-session");
    }
    if let Some(cid) = cid {
        value["cid"] = json!(cid);
    }
    if let Some(operation) = operation {
        value["operation_id"] = json!(operation);
    }
    value
}
async fn post(
    f: &Fixture,
    case: &str,
    request: Value,
    credential: &str,
    valid: bool,
) -> (u16, Value) {
    let client =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    let response = client
        .post(format!(
            "http://{}/v2/commands",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(credential)
        .json(&request)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(
        response.headers()[reqwest::header::CACHE_CONTROL],
        "no-store"
    );
    let response: Value = response.json().await.unwrap();
    if let Ok(directory) = std::env::var("CONNECT_G6_CANONICAL_HTTP_SAMPLES") {
        std::fs::write(std::path::Path::new(&directory).join(format!("{case}.json")),serde_json::to_vec_pretty(&json!({"case":case,"request":request,"valid_request":valid,"status":status,"response":response})).unwrap()).unwrap();
    }
    (status, response)
}
#[tokio::test]
async fn canonical_http_refuses_malformed_unsupported_spoofed_and_expired_requests_without_effects()
{
    let f = Fixture::new("EdDSA", 8).await;
    let mut sdk = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = sdk
        .create_conversation("http-refusal-room", input)
        .await
        .unwrap();
    sdk.send_message(
        "http-original-send",
        &room.id,
        "http_original_message",
        connect_client::MessageDraft::text_all("original"),
    )
    .await
    .unwrap();
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let private = std::fs::read(&f.client.signing_private_key).unwrap();
    let signer =
        rvoip_auth_core::sig9421::Sig9421Signer::ed25519(&f.client.signing_key_id, &private)
            .unwrap();
    let signed = signer
        .sign(&wire("conversation.list", json!({}), None, None))
        .unwrap();
    let mut old = wire("conversation.list", json!({}), None, None);
    old["v"] = json!(1);
    let mut uninstalled = wire(
        "stream.unsubscribe",
        json!({"strm_ids":["future-stream"]}),
        Some(&room.id),
        Some("uninstalled-stream-control"),
    );
    uninstalled["sid"] = json!("future-session");
    uninstalled["connid"] = json!("future-connection");
    uninstalled["expected_revision"] = json!(0);
    let cases = [
        ("transport-signature-refused", signed, 400, false),
        ("old-version", old, 400, false),
        (
            "old-operation-dialect",
            json!({"op":"send","request_id":"private-input-marker"}),
            400,
            false,
        ),
        (
            "unknown-family",
            wire("private-input-marker", json!({}), None, None),
            400,
            false,
        ),
        ("future-stream-family", uninstalled, 501, true),
        (
            "wrong-sender",
            wire(
                "message.send",
                json!({"msg_id":"wrong_sender_message","from":"someone_else","content_type":"text/plain","body":"private-input-marker","to":"all"}),
                Some(&room.id),
                Some("wrong-sender"),
            ),
            403,
            true,
        ),
        (
            "wrong-operation-resource",
            wire(
                "operation.get",
                json!({"operation_id":"http-original-send"}),
                Some("foreign"),
                None,
            ),
            403,
            true,
        ),
        (
            "history-operation-assertion",
            wire(
                "message.history",
                json!({}),
                Some(&room.id),
                Some("unrelated-operation"),
            ),
            400,
            true,
        ),
        (
            "live-http-refusal",
            wire(
                "conversation.subscribe",
                json!({"live":true}),
                Some(&room.id),
                None,
            ),
            501,
            true,
        ),
    ];
    for (case, request, expected, valid) in cases {
        let (status, response) = post(&f, case, request.clone(), &f.client.credential, valid).await;
        assert_eq!(status, expected, "{case}: {response}");
        assert!(!response.to_string().contains("private-input-marker"));
        if valid {
            assert_eq!(response["request_id"], request["id"]);
            assert_eq!(
                response["envelopes"].as_array().unwrap().last().unwrap()["in_reply_to"],
                request["id"]
            );
        }
    }
    let (status,response)=post(&f,"request-over-budget",wire("message.send",json!({"msg_id":"oversized","content_type":"text/plain","body":"x".repeat(70_000),"to":"all"}),Some(&room.id),Some("oversized")),&f.client.credential,true).await;
    assert_eq!(status, 413);
    assert_eq!(response["problem"]["reason"], "request_too_large");
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let limited = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope {
                operations: vec!["history".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let (status, _) = post(
        &f,
        "missing-session-floor",
        wire("conversation.list", json!({}), None, None),
        &limited.token,
        true,
    )
    .await;
    assert_eq!(status, 403);
    let expired = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 1,
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (status, _) = post(
        &f,
        "expired-credential",
        wire("conversation.list", json!({}), None, None),
        &expired.token,
        true,
    )
    .await;
    assert_eq!(status, 401);
    // Issuance itself changes the credential inventory; compare operation/data
    // effects before issuing those deliberately attenuated test credentials.
    let (status, response) = post(
        &f,
        "authorized-original-receipt",
        wire(
            "operation.get",
            json!({"operation_id":"http-original-send"}),
            Some(&room.id),
            None,
        ),
        &f.client.credential,
        true,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        response["envelopes"][0]["payload"]["result"]["data"]["message_id"],
        "http_original_message"
    );
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_http_cancelled_wait_replays_the_original_mutation_once() {
    let f = Fixture::new("EdDSA", 8).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut native, "http-cancellation-room").await;
    let lock = rusqlite::Connection::open(&f.config.database).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let stop = cancellation.clone();
    let payload = payload(
        "message.send",
        json!({"msg_id":"http_cancelled_message","label":"connect-messages","content_type":"text/plain","body":"recover original","body_encoding":"utf8","to":"all","reliability":{"mode":"reliable_ordered"}}),
    );
    let scope = CommandScope {
        conversation_id: Some(cid.clone()),
        operation_id: Some("http-cancelled-send".into()),
        ..Default::default()
    };
    let job = tokio::spawn(async move { http.request_with_cancel(payload, scope, stop).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    assert!(matches!(job.await.unwrap(), Err(ClientError::Cancelled)));
    lock.execute_batch("ROLLBACK").unwrap();
    let replay = native
        .send_message(
            "http-cancelled-send",
            &cid,
            "http_cancelled_message",
            connect_client::MessageDraft::text_all("recover original"),
        )
        .await
        .unwrap();
    assert_eq!(replay.message_id, "http_cancelled_message");
    let page = native
        .history(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .filter(|m| m.msg_id.as_str() == "http_cancelled_message")
            .count(),
        1
    );
    native.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_http_bad_json_authentication_order_and_storage_recovery_preserve_original_ids() {
    let f = Fixture::new("EdDSA", 8).await;
    let mut sdk = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = sdk
        .create_conversation("http-storage-room", input)
        .await
        .unwrap();
    let client =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    for (case, token, expected) in [
        ("invalid-token-before-json", "invalid", 401),
        ("malformed-json", f.client.credential.as_str(), 400),
    ] {
        let raw = "{private-malformed-input";
        let response = client
            .post(format!(
                "http://{}/v2/commands",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(raw)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        assert_eq!(
            response.headers()[reqwest::header::CACHE_CONTROL],
            "no-store"
        );
        let value: Value = response.json().await.unwrap();
        assert!(!value.to_string().contains("private-malformed-input"));
        if let Ok(directory) = std::env::var("CONNECT_G6_CANONICAL_HTTP_SAMPLES") {
            std::fs::write(std::path::Path::new(&directory).join(format!("{case}.json")),serde_json::to_vec_pretty(&json!({"case":case,"request_text":raw,"valid_request":false,"status":expected,"response":value})).unwrap()).unwrap();
        }
    }
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    c.execute_batch("ALTER TABLE credentials RENAME TO deliberately_unavailable_credentials")
        .unwrap();
    let request = wire(
        "message.send",
        json!({"msg_id":"http_storage_message","label":"connect-messages","content_type":"text/plain","body":"storage recovery","body_encoding":"utf8","to":"all","reliability":{"mode":"reliable_ordered"}}),
        Some(&room.id),
        Some("http-storage-send"),
    );
    let (status, response) = post(
        &f,
        "storage-unavailable",
        request,
        &f.client.credential,
        true,
    )
    .await;
    c.execute_batch("ALTER TABLE deliberately_unavailable_credentials RENAME TO credentials")
        .unwrap();
    assert_eq!(status, 503);
    assert_eq!(response["problem"]["category"], "storage");
    assert!(!response.to_string().contains("deliberately_unavailable"));
    let sent = sdk
        .send_message(
            "http-storage-send",
            &room.id,
            "http_storage_message",
            connect_client::MessageDraft::text_all("storage recovery"),
        )
        .await
        .unwrap();
    assert_eq!(sent.message_id, "http_storage_message");
    let page = sdk
        .history(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    drop(c);
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_http_retired_generic_dispatch_refuses_canonical_commands_and_keeps_explicit_extensions(
) {
    use connect_client::application::{ApplicationClient, ApplicationOperation};
    let f = Fixture::new("EdDSA", 8).await;
    let client =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let create = Operation::Create {
        request_id: "must-not-create-legacy-conversation".into(),
        request: connect_client::conversations::CreateRequest::persistent(vec![]),
    };
    assert!(
        matches!(ApplicationOperation::try_from(create.clone()),Err(ClientError::Application(e)) if e.code=="canonical-command-required")
    );
    let response = client
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&f.client.credential)
        .json(&create)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 501);
    let response: connect_protocol::Response = response.json().await.unwrap();
    assert_eq!(response.result.unwrap_err().code, "unsupported-command");
    for op in [
        Operation::CallStart {
            request_id: "retired-provider-call".into(),
            conversation_id: "untrusted".into(),
            route_id: "untrusted".into(),
            destination: "+12025550123".into(),
        },
        Operation::SipDial {
            request_id: "retired-sip-dial".into(),
            conversation_id: "untrusted".into(),
            session_id: "untrusted".into(),
            destination: "sip:untrusted@invalid.example".into(),
        },
    ] {
        assert!(
            matches!(ApplicationOperation::try_from(op.clone()),Err(ClientError::Application(e)) if e.code=="canonical-command-required")
        );
        let response = client
            .post(format!(
                "http://{}/v1/operations",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&f.client.credential)
            .json(&op)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 501);
    }
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let mut application = ApplicationClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    assert!(matches!(
        application
            .execute(ApplicationOperation::try_from(Operation::Health {}).unwrap())
            .await
            .unwrap(),
        Outcome::Health { .. }
    ));
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_http_duplicate_json_keys_are_refused_before_effects_with_authentication_first() {
    let f = Fixture::new("EdDSA", 8).await;
    let mut sdk = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = sdk
        .create_conversation("duplicate-json-room", input)
        .await
        .unwrap();
    let request = wire(
        "message.send",
        json!({"msg_id":"duplicate_json_message","body":"duplicate-private-marker","content_type":"text/plain","to":"all"}),
        Some(&room.id),
        Some("duplicate-json-send"),
    );
    let original = request.to_string();
    let envelope = original.replacen("\"id\":", "\"id\":\"ambiguous-transport-id\",\"id\":", 1);
    let payload = original.replacen(
        "\"body\":\"duplicate-private-marker\"",
        "\"body\":\"first-body\",\"body\":\"duplicate-private-marker\"",
        1,
    );
    assert_ne!(envelope, original);
    assert_ne!(payload, original);
    let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
    let client =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    for (case, raw, token, status) in [
        (
            "duplicate-envelope-id",
            &envelope,
            f.client.credential.as_str(),
            400,
        ),
        (
            "duplicate-payload-body",
            &payload,
            f.client.credential.as_str(),
            400,
        ),
        (
            "invalid-token-before-duplicate-json",
            &envelope,
            "invalid-local-token",
            401,
        ),
    ] {
        let response = client
            .post(format!(
                "http://{}/v2/commands",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(raw.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(
            response.headers()[reqwest::header::CACHE_CONTROL],
            "no-store"
        );
        let response: Value = response.json().await.unwrap();
        assert_eq!(response["problem"]["code"], status);
        assert!(!response.to_string().contains("duplicate-private-marker"));
        if let Ok(directory) = std::env::var("CONNECT_G6_CANONICAL_HTTP_SAMPLES") {
            std::fs::write(std::path::Path::new(&directory).join(format!("{case}.json")),serde_json::to_vec_pretty(&json!({"case":case,"valid_request":false,"raw_request":raw,"status":status,"response":response})).unwrap()).unwrap();
        }
    }
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.server.shutdown().await.unwrap();
}
