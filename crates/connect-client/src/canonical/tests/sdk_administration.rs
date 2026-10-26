//! Owned trusted HTTP submission with a real signed independent status reader.
use super::*;
use crate::application::{ApplicationClient, ApplicationClientConfig};
use crate::sdk::{
    self, AdministrativeClient, Certainty, Client, ErrorKind, MutationOptions, RequestOptions,
};
use connect_protocol::provisioning::{IdentityKind, ProvisioningOutcome, ProvisioningResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Completed,
    UnknownFirst,
    WrongCommand,
    WrongOutcome,
    MissingResult,
}
fn outcome() -> ProvisioningOutcome {
    ProvisioningOutcome::Identity {
        identity_id: "identity_new".into(),
        alias: "new-admin".into(),
        identity_kind: IdentityKind::Human,
        revision: 0,
        revoked: false,
    }
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn mutation() -> MutationOptions {
    MutationOptions {
        operation_id: "original-admin".parse().unwrap(),
        request: options(),
        extensions: None,
    }
}
fn command() -> Value {
    json!({"type":"identity","operation_id":"original-admin","alias":"new-admin","kind":"human","display_name":"private backend name"})
}
fn saved() -> Value {
    json!({"schema":"connect-operation-reference/1","operation_id":"original-admin","command":"identity.provision","scope":{}})
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut p = [0; 4];
    reader.read_exact(&mut p).await.ok()?;
    let n = u32::from_be_bytes(p) as usize;
    assert!(n <= MAX_ENVELOPE_BYTES);
    let mut b = vec![0; n];
    reader.read_exact(&mut b).await.ok()?;
    Some(Envelope::parse(&b).unwrap())
}
async fn status(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    case: Case,
    index: usize,
) {
    assert_eq!(request.kind(), "operation.get");
    assert!(request.scope(Scope::Conversation).is_none());
    assert!(request.scope(Scope::Operation).is_none());
    assert_eq!(
        request.original()["payload"]["operation_id"],
        "original-admin"
    );
    let unknown = matches!(case, Case::UnknownFirst) && index == 0;
    let mut p = json!({"operation_id":"original-admin","command_type":if matches!(case,Case::WrongCommand){"identity.revoke"}else{"identity.provision"},"state":if unknown{"unknown"}else{"completed"},"revision":1});
    if !unknown {
        p["record_id"] = json!("record_admin");
        p["result"] = if matches!(case, Case::WrongOutcome) {
            json!({"kind":"credential","identity_id":"identity_new","credential_id":"credential_one","expires_at":1,"revoked":false})
        } else {
            serde_json::to_value(outcome()).unwrap()
        };
    }
    if matches!(case, Case::MissingResult) {
        p.as_object_mut().unwrap().remove("result");
    }
    for (kind, payload) in [("operation.updated", p), ("ack", json!({}))] {
        let mut v = envelope(kind, payload);
        v["in_reply_to"] = json!(request.id());
        let bytes = serde_json::to_vec(&signer.sign(&v).unwrap()).unwrap();
        writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&bytes).await.unwrap();
    }
}
pub(super) async fn emit(
    case: Case,
    first: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
) {
    status(writer, signer, &first, case, 0).await;
    let mut index = 1;
    while let Some(request) = read(reader).await {
        status(writer, signer, &request, case, index).await;
        index += 1;
    }
}
struct Http {
    config: ApplicationClientConfig,
    ready: Arc<Notify>,
    release: Arc<Notify>,
    bodies: Arc<tokio::sync::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
async fn http(delay: bool, wrong: bool, refuse: Option<u16>) -> Http {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let ready = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let bodies = Arc::new(tokio::sync::Mutex::new(vec![]));
    let seen = bodies.clone();
    let entered = ready.clone();
    let gate = release.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = vec![];
            let mut b = [0; 1];
            while !header.ends_with(b"\r\n\r\n") {
                if stream.read_exact(&mut b).await.is_err() {
                    return;
                }
                header.push(b[0]);
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with("POST /v2/identities HTTP/1.1\r\n"));
            let len: usize = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            assert!(len <= 65_536);
            let mut body = vec![0; len];
            stream.read_exact(&mut body).await.unwrap();
            seen.lock()
                .await
                .push(serde_json::from_slice(&body).unwrap());
            entered.notify_one();
            if delay {
                gate.notified().await;
            }
            let result = if let Some(code) = refuse {
                Err(connect_protocol::Error::new(
                    match code {
                        403 => "forbidden",
                        409 => "stale-revision",
                        503 => "storage-unavailable",
                        _ => unreachable!(),
                    },
                    "private backend detail",
                ))
            } else if wrong {
                Ok(ProvisioningOutcome::Credential {
                    credential_id: "credential_one".into(),
                    identity_id: "identity_new".into(),
                    expires_at: 1,
                    revoked: false,
                })
            } else {
                Ok(outcome())
            };
            let body = serde_json::to_vec(&ProvisioningResponse {
                profile: "connect/2".into(),
                result,
            })
            .unwrap();
            let response=format!("HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",match refuse {Some(403)=>"403 Forbidden",Some(409)=>"409 Conflict",Some(503)=>"503 Unavailable",None=>"200 OK",_=>unreachable!()},body.len());
            if stream.write_all(response.as_bytes()).await.is_ok() {
                let _ = stream.write_all(&body).await;
            }
            let _ = stream.shutdown().await;
        }
    });
    Http {
        config: ApplicationClientConfig {
            endpoint,
            credential: "controlled-admin".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
            timeout_ms: 3000,
        },
        ready,
        release,
        bodies,
        task,
    }
}
async fn setup(case: Case, http: &Http) -> (AdministrativeClient, tokio::task::JoinHandle<()>) {
    let (peer, host, _) = controlled(Fault::SdkAdministration(case)).await;
    let control = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let app = ApplicationClient::open(http.config.clone()).await.unwrap();
    (AdministrativeClient::from_parts(control, app), host)
}
async fn finish(admin: &AdministrativeClient, host: tokio::task::JoinHandle<()>, http: Http) {
    admin.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
    http.task.abort();
    let _ = http.task.await;
}
#[tokio::test]
async fn administrative_provision_validates_original_intent_and_restores_only_administrative_references(
) {
    let http = http(false, false, None).await;
    let (admin, host) = setup(Case::Completed, &http).await;
    let intent = admin.validate_provisioning(command()).unwrap();
    assert_eq!(intent.operation_id().as_str(), "original-admin");
    assert!(!format!("{intent:?}").contains("private backend name"));
    let persisted = intent.to_json();
    let before_io = intent.reference_json();
    let operation = admin.provision(intent, mutation()).await.unwrap();
    let saved = operation.reference_json();
    assert_eq!(saved, crate::canonical::tests::sdk_administration::saved());
    assert_eq!(saved, before_io);
    assert_eq!(
        operation.wait(options()).await.unwrap().value["identity_id"],
        "identity_new"
    );
    assert_eq!(
        admin
            .restore(saved)
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value["kind"],
        "identity"
    );
    assert_eq!(http.bodies.lock().await.as_slice(), [persisted]);
    assert!(admin.restore(json!({"schema":"connect-operation-reference/1","operation_id":"original-admin","command":"message.send","scope":{"conversation_id":"room"}})).is_err());
    finish(&admin, host, http).await;
}
#[tokio::test]
async fn administrative_local_preflight_refuses_mismatched_ids_extensions_scope_and_bad_routes_without_http(
) {
    let http = http(false, false, None).await;
    let (admin, host) = setup(Case::Completed, &http).await;
    let mut mismatch = mutation();
    mismatch.operation_id = "another".parse().unwrap();
    assert!(
        matches!(admin.provision(admin.validate_provisioning(command()).unwrap(),mismatch).await,Err(e) if e.kind==ErrorKind::Validation&&e.certainty==Certainty::NotSubmitted)
    );
    let mut extension = mutation();
    extension.extensions = Some(std::collections::BTreeMap::from([(
        "extra".into(),
        json!(true),
    )]));
    assert!(
        matches!(admin.provision(admin.validate_provisioning(command()).unwrap(),extension).await,Err(e) if e.kind==ErrorKind::Unsupported&&e.certainty==Certainty::NotSubmitted)
    );
    assert!(admin.validate_provisioning(json!({"type":"revoke_identity","operation_id":"original-admin","identity_id":"..","expected_revision":0})).is_err());
    let mut changed = saved();
    changed["scope"] = json!({"conversation_id":"foreign"});
    assert!(admin.restore(changed).is_err());
    assert!(http.bodies.lock().await.is_empty());
    admin
        .restore(saved())
        .unwrap()
        .status(options())
        .await
        .unwrap();
    finish(&admin, host, http).await;
}
#[tokio::test]
async fn cancelled_started_http_keeps_original_status_and_owns_completion_without_republication() {
    let http = http(true, false, None).await;
    let (admin, host) = setup(Case::UnknownFirst, &http).await;
    let mut request = mutation();
    let token = request.request.signal.clone();
    request.request.timeout = Duration::from_secs(2);
    let submitted = admin.provision(admin.validate_provisioning(command()).unwrap(), request);
    tokio::pin!(submitted);
    tokio::select! {result=&mut submitted=>panic!("unexpected completion: {}",result.is_ok()),_=http.ready.notified()=>()};
    token.cancel();
    let e = submitted.await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert_eq!(e.certainty, Certainty::Uncertain);
    assert_eq!(e.operation.unwrap().operation_id.as_str(), "original-admin");
    assert_eq!(
        admin
            .restore(saved())
            .unwrap()
            .status(options())
            .await
            .unwrap()
            .state,
        sdk::OperationState::Unknown
    );
    http.release.notify_one();
    assert_eq!(
        admin
            .restore(saved())
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value["identity_id"],
        "identity_new"
    );
    assert_eq!(http.bodies.lock().await.len(), 1);
    finish(&admin, host, http).await;
}
#[tokio::test]
async fn administrative_wrong_http_result_and_business_refusal_keep_original_error_context() {
    for (wrong, refuse) in [
        (true, None),
        (false, Some(403)),
        (false, Some(409)),
        (false, Some(503)),
    ] {
        let http = http(false, wrong, refuse).await;
        let (admin, host) = setup(Case::Completed, &http).await;
        let e = admin
            .provision(admin.validate_provisioning(command()).unwrap(), mutation())
            .await
            .err()
            .unwrap();
        assert_eq!(
            e.operation.as_ref().unwrap().operation_id.as_str(),
            "original-admin"
        );
        assert_eq!(
            e.kind,
            match refuse {
                Some(403) => ErrorKind::Authentication,
                Some(409) => ErrorKind::Conflict,
                Some(503) => ErrorKind::Application,
                None => ErrorKind::Protocol,
                _ => unreachable!(),
            }
        );
        assert_eq!(
            e.certainty,
            if matches!(refuse, Some(403 | 409)) {
                Certainty::Refused
            } else {
                Certainty::Uncertain
            }
        );
        admin
            .restore(saved())
            .unwrap()
            .status(options())
            .await
            .unwrap();
        finish(&admin, host, http).await;
    }
}
#[tokio::test]
async fn administrative_status_checks_original_command_type_and_outcome_kind() {
    for case in [Case::WrongCommand, Case::WrongOutcome, Case::MissingResult] {
        let http = http(false, false, None).await;
        let (admin, host) = setup(case, &http).await;
        let operation = admin
            .provision(admin.validate_provisioning(command()).unwrap(), mutation())
            .await
            .unwrap();
        let error = operation.wait(options()).await.err().unwrap();
        assert_eq!(error.kind, ErrorKind::Protocol);
        assert_eq!(error.certainty, Certainty::ReadOnly);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "original-admin"
        );
        if !matches!(case, Case::WrongCommand) {
            assert_eq!(
                error.observation.unwrap().state,
                sdk::OperationState::Completed
            );
        }
        let invalid = RequestOptions {
            timeout: Duration::ZERO,
            signal: sdk::CancellationToken::new(),
        };
        let error = operation.wait(invalid).await.err().unwrap();
        assert_eq!(error.kind, ErrorKind::Configuration);
        assert_eq!(error.certainty, Certainty::ReadOnly);
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            "original-admin"
        );
        finish(&admin, host, http).await;
    }
}
#[tokio::test]
async fn administrative_close_cancels_owned_http_and_retains_joins_after_cancelled_cleanup() {
    let http = http(true, false, None).await;
    let (admin, host) = setup(Case::Completed, &http).await;
    let request = admin.provision(admin.validate_provisioning(command()).unwrap(), mutation());
    tokio::pin!(request);
    tokio::select! {result=&mut request=>panic!("unexpected completion: {}",result.is_ok()),_=http.ready.notified()=>()};
    admin
        .restore(saved())
        .unwrap()
        .status(options())
        .await
        .unwrap();
    let cancelled = options();
    cancelled.signal.cancel();
    assert_eq!(
        admin.close(cancelled).await.unwrap_err().kind,
        ErrorKind::CleanupIncomplete
    );
    assert!(request.await.is_err());
    admin.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
    http.release.notify_one();
    http.task.abort();
    let _ = http.task.await;
}
#[tokio::test]
async fn administrative_full_queue_keeps_status_live_and_closes_unwritten_commands_without_http() {
    use std::{future::Future, task::Poll};
    let http = http(true, false, None).await;
    let (admin, host) = setup(Case::Completed, &http).await;
    let mut pending = (0..16)
        .map(|index| {
            let id = format!("capacity-{index}");
            let mut value = command();
            value["operation_id"] = json!(id);
            let mut options = mutation();
            options.operation_id = id.parse().unwrap();
            Box::pin(admin.provision(admin.validate_provisioning(value).unwrap(), options))
        })
        .collect::<Vec<_>>();
    // Poll every request into the actual bounded queue; none can complete while
    // the first HTTP response remains held by the independent server.
    std::future::poll_fn(|cx| {
        for request in &mut pending {
            assert!(request.as_mut().poll(cx).is_pending());
        }
        Poll::Ready(())
    })
    .await;
    http.ready.notified().await;
    let mut value = command();
    value["operation_id"] = json!("capacity-excess");
    let mut options = mutation();
    options.operation_id = "capacity-excess".parse().unwrap();
    options.request.timeout = Duration::from_millis(30);
    let error = admin
        .provision(admin.validate_provisioning(value).unwrap(), options)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Deadline);
    assert_eq!(error.certainty, Certainty::NotSubmitted);
    admin
        .restore(saved())
        .unwrap()
        .status(super::sdk_administration::options())
        .await
        .unwrap();
    admin
        .close(super::sdk_administration::options())
        .await
        .unwrap();
    for (index, request) in pending.iter_mut().enumerate() {
        let error = request.as_mut().await.err().unwrap();
        assert_eq!(
            error.operation.unwrap().operation_id.as_str(),
            format!("capacity-{index}")
        );
        assert_eq!(
            error.certainty,
            if index == 0 {
                Certainty::Uncertain
            } else {
                Certainty::NotSubmitted
            }
        );
    }
    assert_eq!(http.bodies.lock().await.len(), 1);
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
    http.task.abort();
    let _ = http.task.await;
}
