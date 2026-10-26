//! Signed finite exchanges exercise the public advanced facade.
use super::*;
use crate::sdk::{self, Client, RequestOptions};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn query() -> Value {
    json!({"v":2,"id":"caller-attempt","ts":"2026-01-01T00:00:00Z","type":"operation.get","cid":"conv_one","payload":{"operation_id":"original-send","vendor":{"private":"payload-secret"}}})
}
fn message() -> Value {
    let mut v = query();
    v["type"] = json!("message.send");
    v["operation_id"] = json!("original-send");
    let mut body = crate::MessageDraft::text_all("private fixture");
    body.msg_id = Some("original-message".into());
    v["payload"] = serde_json::to_value(body).unwrap();
    v
}
pub(super) async fn emit(
    request: Envelope,
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
) {
    assert_eq!(request.kind(), "operation.get");
    assert_ne!(request.id(), "caller-attempt");
    assert_ne!(request.original()["ts"], query()["ts"]);
    assert_eq!(request.original()["payload"], query()["payload"]);
    sdk_worker::write(writer,signer,&request,"operation.updated",json!({"operation_id":"original-send","state":"unknown","revision":1,"vendor":{"reply":"response-secret"}})).await;
    sdk_worker::write(
        writer,
        signer,
        &request,
        "ack",
        json!({"vendor":"ack-secret"}),
    )
    .await;
}
async fn finish(client: &Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn raw_validation_and_lifecycle_refusals_leave_signed_extensions_intact() {
    let (peer, host, _) = controlled(Fault::SdkRaw).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let raw = client.raw();
    let mut bad = query();
    bad["v"] = json!(1);
    assert_eq!(
        raw.validate(bad).unwrap_err().kind,
        sdk::ErrorKind::Validation
    );
    let mut oversized = query();
    oversized["payload"]["vendor"] = json!("x".repeat(MAX_ENVELOPE_BYTES));
    assert_eq!(
        raw.validate(oversized).unwrap_err().kind,
        sdk::ErrorKind::Validation
    );
    let mut reply = query();
    reply["type"] = json!("ack");
    reply["payload"] = json!({});
    let mut extension = query();
    extension["private_top_level"] = json!("cannot-silently-drop");
    let mut live = query();
    live["type"] = json!("conversation.subscribe");
    live["payload"] = json!({"after":"saved-cursor","live":true,"limit":1,"lease_seconds":10});
    let mut auth = query();
    auth["type"] = json!("auth.keepalive");
    auth["payload"] = json!({"session_token":"private-auth-secret"});
    for v in [reply, extension, live, auth] {
        let v = raw.validate(v).unwrap();
        assert_eq!(
            raw.request(v, options()).await.unwrap_err().kind,
            sdk::ErrorKind::Unsupported
        );
    }
    let mut advisory = query();
    advisory["type"] = json!("capability.advertise");
    advisory["payload"] = json!({"capabilities":{},"trigger":"change"});
    let advisory = raw.validate(advisory).unwrap();
    let e = raw.request(advisory, options()).await.unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Validation, sdk::Certainty::NotSubmitted)
    );
    let v = raw.validate(query()).unwrap();
    assert_eq!(v.to_json(), query());
    assert!(!format!("{v:?}").contains("payload-secret"));
    let frames = raw.request(v, options()).await.unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].kind(), "operation.updated");
    assert_eq!(
        frames[0].to_json()["payload"]["vendor"]["reply"],
        "response-secret"
    );
    assert!(frames[0].to_json().get("signature").is_some());
    assert_eq!(frames[1].kind(), "ack");
    assert_eq!(frames[1].to_json()["payload"]["vendor"], "ack-secret");
    assert!(!format!("{:?}", frames[0]).contains("response-secret"));
    finish(&client, host).await;
}
#[tokio::test]
async fn raw_cancelled_mutation_retains_original_id_and_can_recover_without_replaying() {
    let (peer, host, ready) = controlled(Fault::SdkWorker(sdk_worker::Traffic::Mutation)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let raw = client.raw();
    let cancellation = sdk::CancellationToken::new();
    let cancelled = cancellation.clone();
    let v = raw.validate(message()).unwrap();
    let worker = tokio::spawn(async move {
        raw.request(
            v,
            RequestOptions {
                signal: cancelled,
                ..options()
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready.notified())
        .await
        .unwrap();
    cancellation.cancel();
    let e = worker.await.unwrap().unwrap_err();
    assert_eq!(e.certainty, sdk::Certainty::Uncertain);
    assert_eq!(e.operation.unwrap().operation_id.as_str(), "original-send");
    let frames = client
        .raw()
        .request(client.raw().validate(query()).unwrap(), options())
        .await
        .unwrap();
    assert_eq!(frames[0].to_json()["payload"]["state"], "completed");
    finish(&client, host).await;
}
#[tokio::test]
async fn raw_cancelled_read_and_invalid_options_remain_read_only() {
    let (peer, host, ready) = controlled(Fault::SdkWorker(sdk_worker::Traffic::Query)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let raw = client.raw();
    let v = raw.validate(query()).unwrap();
    let e = raw
        .request(
            v.clone(),
            RequestOptions {
                timeout: Duration::ZERO,
                ..options()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Configuration, sdk::Certainty::ReadOnly)
    );
    let cancelled = sdk::CancellationToken::new();
    let signal = cancelled.clone();
    let worker = tokio::spawn(async move {
        raw.request(
            v,
            RequestOptions {
                signal,
                ..options()
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready.notified())
        .await
        .unwrap();
    cancelled.cancel();
    assert_eq!(
        worker.await.unwrap().unwrap_err().certainty,
        sdk::Certainty::ReadOnly
    );
    let frames = client
        .raw()
        .request(client.raw().validate(query()).unwrap(), options())
        .await
        .unwrap();
    assert_eq!(frames[0].to_json()["payload"]["revision"], 2);
    finish(&client, host).await;
}
#[tokio::test]
async fn raw_foreign_correlated_scope_retires_the_peer_without_returning_frames() {
    let (peer, host, _) = controlled(Fault::ForeignScope).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let e = client
        .raw()
        .request(client.raw().validate(query()).unwrap(), options())
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Protocol, sdk::Certainty::ReadOnly)
    );
    finish(&client, host).await;
}
