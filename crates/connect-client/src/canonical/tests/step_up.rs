use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Reply {
    Valid,
    Refused,
    RefusedThenChange,
    Interrupted,
    ForeignOwner,
    ForeignProfile,
    WrongAssurance,
    Expired,
    UnchangedToken,
    WrongScope,
    WrongPrevious,
    WrongReason,
    ForeignChange,
    ChangeFirst,
    DuplicateSession,
    OriginalFirst,
    MissingChange,
    MissingOriginal,
    Streaming,
}
async fn write(writer: &mut quinn::SendStream, signer: &Sig9421Signer, value: Value) -> bool {
    let signed = signer.sign(&value).unwrap();
    // A signed foreign application profile is deliberately malformed at the
    // envelope parser boundary. Other fault replies must remain valid frames.
    if value["payload"]["application_profile"] != "other/1" {
        Envelope::from_value(signed.clone()).expect("step-up fixture frame must parse");
    }
    let raw = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(raw.len() as u32).to_be_bytes())
        .await
        .is_ok()
        && writer.write_all(&raw).await.is_ok()
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.ok()?;
    let mut bytes = vec![0; u32::from_be_bytes(prefix) as usize];
    reader.read_exact(&mut bytes).await.ok()?;
    Some(Envelope::parse(&bytes).unwrap())
}
pub(super) async fn emit(
    fault: Reply,
    original: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    ready: &Notify,
) {
    let scoped = |kind: &str, payload: Value, reply: &str| {
        let mut value = envelope(kind, payload);
        value["in_reply_to"] = json!(reply);
        for field in ["cid", "sid", "connid", "operation_id"] {
            if !original.original()[field].is_null() {
                value[field] = original.original()[field].clone();
            }
        }
        value
    };
    let challenge = scoped(
        "identity.step-up-request",
        json!({"required":"task-scoped","allowed_methods":["bearer"],"reason":"private-policy-canary"}),
        original.id(),
    );
    let challenge_id = challenge["id"].as_str().unwrap().to_owned();
    if !write(writer, signer, challenge).await {
        return;
    }
    ready.notify_one();
    let Some(response) = read(reader).await else {
        return;
    };
    assert_eq!(response.kind(), "identity.step-up-response");
    assert_ne!(response.id(), original.id());
    assert_eq!(response.scope(Scope::Reply), Some(challenge_id.as_str()));
    for scope in [
        Scope::Conversation,
        Scope::Session,
        Scope::Connection,
        Scope::Operation,
    ] {
        assert_eq!(response.scope(scope), original.scope(scope));
    }
    assert_eq!(response.expected_revision(), original.expected_revision());
    assert_eq!(response.original()["payload"]["method"], "bearer");
    assert_eq!(
        response.original()["payload"]["credential"],
        "explicit-delegation-canary"
    );
    if matches!(
        fault,
        Reply::Refused | Reply::RefusedThenChange | Reply::Interrupted
    ) {
        let interrupted = matches!(fault, Reply::Interrupted);
        let mut problem = json!({"code":if interrupted {409}else{403},"category":"auth","reason":if interrupted {"operation_interrupted"}else{"assurance_not_established"}});
        if interrupted {
            problem["details"] = json!({"outcome":"unknown"});
        }
        if !write(writer, signer, scoped("error", problem, original.id())).await {
            return;
        }
        write(
            writer,
            signer,
            scoped(
                "error",
                json!({"code":if interrupted {408}else{401},"category":"auth","reason":"private-refusal-canary"}),
                response.id(),
            ),
        )
        .await;
        if matches!(fault, Reply::RefusedThenChange) {
            write(writer,signer,scoped("auth.session",json!({"identity_id":"id_one","assurance":"task-scoped","session_token":"late-refused-session-canary","expires_at":chrono::Utc::now()+chrono::Duration::minutes(2),"application_profile":"connect/2"}),response.id())).await;
            write(writer,signer,scoped("identity.assurance-changed",json!({"previous":"identified","current":"task-scoped","reason":"verified-step-up"}),response.id())).await;
        }
        return;
    }
    let mut session = scoped(
        "auth.session",
        json!({"identity_id":"id_one","assurance":"task-scoped","session_token":"new-step-up-session-canary","expires_at":chrono::Utc::now()+chrono::Duration::minutes(2),"application_profile":"connect/2"}),
        response.id(),
    );
    let mut changed = scoped(
        "identity.assurance-changed",
        json!({"previous":"identified","current":"task-scoped","reason":"verified-step-up"}),
        response.id(),
    );
    let original_ack = scoped(
        "ack",
        json!({"details":{"reply_complete":true}}),
        original.id(),
    );
    match fault {
        Reply::ForeignOwner => session["payload"]["identity_id"] = json!("id_other"),
        Reply::ForeignProfile => session["payload"]["application_profile"] = json!("other/1"),
        Reply::WrongAssurance => session["payload"]["assurance"] = json!("user-authorized"),
        Reply::Expired => {
            session["payload"]["expires_at"] =
                json!(chrono::Utc::now() - chrono::Duration::seconds(1))
        }
        Reply::UnchangedToken => {
            session["payload"]["session_token"] = json!("controlled-post-auth")
        }
        Reply::WrongScope => changed["cid"] = json!("conv_other"),
        Reply::WrongPrevious => changed["payload"]["previous"] = json!("anonymous"),
        Reply::WrongReason => changed["payload"]["reason"] = json!("credential_expired"),
        Reply::ForeignChange => changed["in_reply_to"] = json!("other-response"),
        _ => {}
    }
    if matches!(fault, Reply::OriginalFirst) {
        write(writer, signer, original_ack).await;
        return;
    }
    if matches!(fault, Reply::ChangeFirst) {
        write(writer, signer, changed).await;
        return;
    }
    if !write(writer, signer, session.clone()).await {
        return;
    }
    if matches!(fault, Reply::DuplicateSession) {
        write(writer, signer, session).await;
        return;
    }
    if matches!(fault, Reply::Streaming) {
        for _ in 0..40 {
            if !write(
                writer,
                signer,
                scoped("ack", json!({}), "unrelated-request"),
            )
            .await
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        return;
    }
    if matches!(fault, Reply::MissingChange) {
        return;
    }
    if !write(writer, signer, changed).await {
        return;
    }
    if matches!(fault, Reply::MissingOriginal) {
        return;
    }
    // Live facts remain observations, not the held command's completion.
    if !write(
        writer,
        signer,
        scoped(
            "operation.updated",
            json!({"operation_id":"op_probe","state":"unknown","revision":1}),
            "other-request",
        ),
    )
    .await
    {
        return;
    }
    if !write(writer, signer, original_ack).await {
        return;
    }
    if matches!(fault, Reply::Valid) {
        let Some(heartbeat) = read(reader).await else {
            return;
        };
        assert_eq!(heartbeat.kind(), "auth.keepalive");
        assert_eq!(
            heartbeat.original()["payload"]["session_token"],
            "new-step-up-session-canary"
        );
        let mut ack = envelope("ack", json!({}));
        ack["in_reply_to"] = json!(heartbeat.id());
        write(writer, signer, ack).await;
    }
}
fn scoped() -> CommandScope {
    CommandScope {
        session_id: Some("sess_one".into()),
        connection_id: Some("conn_one".into()),
        operation_id: Some("op_probe".into()),
        expected_revision: Some(7),
        ..scope()
    }
}
async fn finish(client: CanonicalClient, server: tokio::task::JoinHandle<()>) {
    client.close();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn canonical_step_up_resumes_original_scope_and_id_without_resending_and_installs_rotated_session(
) {
    step_up_case_resumes_original_scope_and_id_without_resending_and_installs_rotated_session(
        false,
    )
    .await;
}
#[tokio::test]
async fn owned_canonical_step_up_resumes_original_scope_and_id_without_resending_and_installs_rotated_session(
) {
    step_up_case_resumes_original_scope_and_id_without_resending_and_installs_rotated_session(true)
        .await;
}
async fn step_up_case_resumes_original_scope_and_id_without_resending_and_installs_rotated_session(
    owned: bool,
) {
    let (mut client, server, _) = controlled(Fault::StepUp(Reply::Valid)).await;
    let pin = chrono::Utc::now() + chrono::Duration::seconds(50);
    client.server_key_expires_at = pin;
    let predecessor_expiry = chrono::Utc::now() + chrono::Duration::milliseconds(250);
    if owned {
        client.authenticated_until = predecessor_expiry;
    }
    if owned {
        client.own_signaling().unwrap();
    }
    let challenged = client.request(query(), scoped()).await.unwrap();
    assert_eq!(
        challenged.step_up_request().unwrap().required,
        "task-scoped"
    );
    assert!(
        matches!(challenged.ack(),Err(ClientError::Application(error)) if error.code=="step-up-required")
    );
    assert_eq!(client.step_up_required(), Some("task-scoped"));
    assert!(matches!(
        client.request(query(), scoped()).await,
        Err(ClientError::Protocol)
    ));
    assert!(matches!(
        client
            .refresh_credential("different-credential-canary".into())
            .await,
        Err(ClientError::Protocol)
    ));
    assert!(client.step_up.is_some());
    let completed = client
        .complete_step_up("explicit-delegation-canary".into())
        .await
        .unwrap();
    assert_eq!(completed.replies.request_id, challenged.request_id);
    completed.replies.ack().unwrap();
    assert_eq!(completed.authentication.assurance, "task-scoped");
    assert_eq!(client.step_up_required(), None);
    assert_eq!(completed.authentication.expires_at, pin);
    assert!(!format!("{:?}", completed.authentication).contains("canary"));
    assert!(!client.closed.load(Ordering::Acquire));
    assert_eq!(client.receive().await.unwrap().kind(), "operation.updated");
    if owned {
        let after = predecessor_expiry + chrono::Duration::milliseconds(20);
        if let Ok(wait) = (after - chrono::Utc::now()).to_std() {
            tokio::time::sleep(wait).await;
        }
        assert!(
            !client.closed.load(Ordering::Acquire),
            "validated owned step-up did not renew the original reader"
        );
    }
    let heartbeat = client.keepalive().await.unwrap();
    assert_eq!(
        client.receive().await.unwrap().scope(Scope::Reply),
        Some(heartbeat.as_str())
    );
    finish(client, server).await;
}

#[tokio::test]
async fn canonical_step_up_rejects_mismatched_or_incomplete_signed_transitions_and_retires_uncertainty(
) {
    step_up_case_rejects_mismatched_or_incomplete_signed_transitions_and_retires_uncertainty(false)
        .await;
}
#[tokio::test]
async fn owned_canonical_step_up_rejects_mismatched_or_incomplete_signed_transitions_and_retires_uncertainty(
) {
    step_up_case_rejects_mismatched_or_incomplete_signed_transitions_and_retires_uncertainty(true)
        .await;
}
async fn step_up_case_rejects_mismatched_or_incomplete_signed_transitions_and_retires_uncertainty(
    owned: bool,
) {
    for reply in [
        Reply::ForeignOwner,
        Reply::ForeignProfile,
        Reply::WrongAssurance,
        Reply::Expired,
        Reply::UnchangedToken,
        Reply::WrongScope,
        Reply::WrongPrevious,
        Reply::WrongReason,
        Reply::ForeignChange,
        Reply::ChangeFirst,
        Reply::DuplicateSession,
        Reply::OriginalFirst,
        Reply::MissingChange,
        Reply::MissingOriginal,
        Reply::Streaming,
    ] {
        let (mut client, server, _) = controlled(Fault::StepUp(reply)).await;
        client.deadline = Duration::from_millis(300);
        if owned {
            client.own_signaling().unwrap();
        }
        client.request(query(), scoped()).await.unwrap();
        let started = tokio::time::Instant::now();
        let result = client
            .complete_step_up("explicit-delegation-canary".into())
            .await;
        assert!(result.is_err(), "{reply:?}");
        assert!(client.closed.load(Ordering::Acquire), "{reply:?}");
        assert!(started.elapsed() < Duration::from_secs(1), "{reply:?}");
        assert!(client.step_up.is_none());
        finish(client, server).await;
    }
}

#[tokio::test]
async fn canonical_step_up_definitive_refusal_keeps_previous_binding_and_consumes_challenge() {
    step_up_case_definitive_refusal_keeps_previous_binding_and_consumes_challenge(false).await;
}
#[tokio::test]
async fn owned_canonical_step_up_definitive_refusal_keeps_previous_binding_and_consumes_challenge()
{
    step_up_case_definitive_refusal_keeps_previous_binding_and_consumes_challenge(true).await;
}
async fn step_up_case_definitive_refusal_keeps_previous_binding_and_consumes_challenge(
    owned: bool,
) {
    let (mut client, server, _) = controlled(Fault::StepUp(Reply::Refused)).await;
    if owned {
        client.own_signaling().unwrap();
    }
    client.request(query(), scoped()).await.unwrap();
    let previous = (client.session_token.clone(), client.authenticated_until);
    let error = client
        .complete_step_up("explicit-delegation-canary".into())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ClientError::Application(_)));
    assert!(!format!("{error:?}").contains("canary"));
    assert!(!client.closed.load(Ordering::Acquire));
    assert_eq!(
        (client.session_token.clone(), client.authenticated_until),
        previous
    );
    assert!(client.step_up.is_none());
    let original_problem = client.receive().await.unwrap();
    assert_eq!(original_problem.original()["payload"]["code"], 403);
    // A second completion has no proof and performs no IO or retirement.
    assert!(matches!(
        client
            .complete_step_up("explicit-delegation-canary".into())
            .await,
        Err(ClientError::Protocol)
    ));
    assert!(!client.closed.load(Ordering::Acquire));
    finish(client, server).await;
}

#[tokio::test]
async fn canonical_step_up_interrupted_execution_keeps_original_unknown_separate_from_auth_refusal()
{
    step_up_case_interrupted_execution_keeps_original_unknown_separate_from_auth_refusal(false)
        .await;
}
#[tokio::test]
async fn owned_canonical_step_up_interrupted_execution_keeps_original_unknown_separate_from_auth_refusal(
) {
    step_up_case_interrupted_execution_keeps_original_unknown_separate_from_auth_refusal(true)
        .await;
}
async fn step_up_case_interrupted_execution_keeps_original_unknown_separate_from_auth_refusal(
    owned: bool,
) {
    let (mut client, server, _) = controlled(Fault::StepUp(Reply::Interrupted)).await;
    if owned {
        client.own_signaling().unwrap();
    }
    let original = client.request(query(), scoped()).await.unwrap();
    let previous = client.session_token.clone();
    assert!(matches!(
        client
            .complete_step_up("explicit-delegation-canary".into())
            .await,
        Err(ClientError::Application(_))
    ));
    let problem = client.receive().await.unwrap();
    assert_eq!(
        problem.scope(Scope::Reply),
        Some(original.request_id.as_str())
    );
    assert_eq!(problem.original()["payload"]["code"], 409);
    assert_eq!(
        problem.original()["payload"]["details"]["outcome"],
        "unknown"
    );
    assert_eq!(client.session_token, previous);
    assert!(!client.closed.load(Ordering::Acquire));
    finish(client, server).await;
}

#[tokio::test]
async fn canonical_step_up_cancel_after_response_stops_partial_transition_without_installing_candidate(
) {
    step_up_case_cancel_after_response_stops_partial_transition_without_installing_candidate(false)
        .await;
}
#[tokio::test]
async fn owned_canonical_step_up_cancel_after_response_stops_partial_transition_without_installing_candidate(
) {
    step_up_case_cancel_after_response_stops_partial_transition_without_installing_candidate(true)
        .await;
}
async fn step_up_case_cancel_after_response_stops_partial_transition_without_installing_candidate(
    owned: bool,
) {
    let (mut client, server, ready) = controlled(Fault::StepUp(Reply::MissingChange)).await;
    if owned {
        client.own_signaling().unwrap();
    }
    client.request(query(), scoped()).await.unwrap();
    ready.notified().await; // challenge was sent; consume that observation
    let cancellation = tokio_util::sync::CancellationToken::new();
    let stop = cancellation.clone();
    let trigger = tokio::spawn(async move {
        ready.notified().await; // response was consumed and candidate session sent
        stop.cancel();
    });
    let previous = client.session_token.clone();
    assert!(matches!(
        client
            .complete_step_up_with_cancel("explicit-delegation-canary".into(), cancellation)
            .await,
        Err(ClientError::Cancelled)
    ));
    trigger.await.unwrap();
    assert_eq!(client.session_token, previous);
    assert!(client.closed.load(Ordering::Acquire));
    assert!(client.step_up.is_none());
    finish(client, server).await;
}

#[tokio::test]
async fn canonical_step_up_presend_cancel_preserves_challenge_and_original_deadline_still_expires()
{
    step_up_case_presend_cancel_preserves_challenge_and_original_deadline_still_expires(false)
        .await;
}
#[tokio::test]
async fn owned_canonical_step_up_presend_cancel_preserves_challenge_and_original_deadline_still_expires(
) {
    step_up_case_presend_cancel_preserves_challenge_and_original_deadline_still_expires(true).await;
}
async fn step_up_case_presend_cancel_preserves_challenge_and_original_deadline_still_expires(
    owned: bool,
) {
    let (mut client, server, _) = controlled(Fault::StepUp(Reply::Valid)).await;
    client.deadline = Duration::from_millis(150);
    if owned {
        client.own_signaling().unwrap();
    }
    client.request(query(), scoped()).await.unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        client
            .complete_step_up_with_cancel("explicit-delegation-canary".into(), cancellation)
            .await,
        Err(ClientError::Cancelled)
    ));
    assert!(client.step_up.is_some());
    assert!(!client.closed.load(Ordering::Acquire));
    tokio::time::sleep(Duration::from_millis(170)).await;
    assert!(matches!(
        client
            .complete_step_up("explicit-delegation-canary".into())
            .await,
        Err(ClientError::Timeout)
    ));
    assert!(client.closed.load(Ordering::Acquire));
    finish(client, server).await;
}

#[tokio::test]
async fn owned_step_up_late_transition_after_definitive_refusal_is_terminal_without_deadline_extension(
) {
    let (mut client, server, _) = controlled(Fault::StepUp(Reply::RefusedThenChange)).await;
    client.own_signaling().unwrap();
    client.request(query(), scoped()).await.unwrap();
    let previous = (client.session_token.clone(), client.authenticated_until);
    assert!(client
        .complete_step_up("explicit-delegation-canary".into())
        .await
        .is_err());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while !client.closed.load(Ordering::Acquire) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a late unvalidated transition kept its physical peer alive"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        (client.session_token.clone(), client.authenticated_until),
        previous
    );
    assert!(client.keepalive().await.is_err());
    finish(client, server).await;
}
