//! Signed read-only continuation recovery; no application mutation is accepted.
use super::*;
use crate::sdk::{Client, ErrorKind, RequestOptions};
mod metadata;

#[derive(Clone, Copy)]
pub(in crate::canonical) enum Case {
    Metadata(metadata::Mode),
    Evicted,
    Repeated,
    Denied,
    OtherConflict,
    InitialResync,
    Deadline,
    Cancelled,
}
async fn read(reader: &mut quinn::RecvStream) -> Option<Envelope> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.ok()?;
    let n = u32::from_be_bytes(prefix) as usize;
    assert!(n > 0 && n <= MAX_ENVELOPE_BYTES);
    let mut bytes = vec![0; n];
    reader.read_exact(&mut bytes).await.ok()?;
    Some(Envelope::parse(&bytes).unwrap())
}
async fn write(
    writer: &mut quinn::SendStream,
    signer: &Sig9421Signer,
    request: &Envelope,
    kind: &str,
    payload: Value,
) {
    assert_eq!(request.kind(), "conversation.get");
    let mut frame = envelope(kind, payload);
    frame["cid"] = request.original()["cid"].clone();
    frame["in_reply_to"] = json!(request.id());
    let signed = signer.sign(&frame).unwrap();
    Envelope::from_value(signed.clone()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
fn snapshot(revision: u64, participant: &str, more: bool) -> Value {
    let mut value = json!({"revision":revision,"cursor":format!("cursor-{revision}"),"items":[
        {"kind":"conversation","data":{"conversation_id":"conv_one","revision":revision,"closed":false,"policy":"persistent"}},
        {"kind":"participant","data":{"participant_id":participant,"conversation_id":"conv_one","identity_id":"identity","kind":"human","role":"participant","membership_epoch":1,"active":true,"removal_reason":null}}
    ],"has_more":more});
    if more {
        value["next_page"] = json!("evicted-page");
    }
    value
}
pub(super) async fn emit(
    case: Case,
    mut request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    if let Case::Metadata(mode) = case {
        metadata::emit(mode, request, writer, reader, signer, signal).await;
        return;
    }
    if matches!(case, Case::InitialResync) {
        write(
            writer,
            signer,
            &request,
            "error",
            json!({"code":409,"category":"state","reason":"resync_required"}),
        )
        .await;
        return;
    }
    for attempt in 0..3 {
        assert_eq!(request.kind(), "conversation.get");
        assert!(request.original()["payload"].get("page").is_none());
        write(
            writer,
            signer,
            &request,
            "conversation.snapshot",
            snapshot(1, "stale", true),
        )
        .await;
        write(writer, signer, &request, "ack", json!({})).await;
        let Some(next) = read(reader).await else {
            return;
        };
        request = next;
        assert_eq!(request.original()["payload"]["page"], "evicted-page");
        if matches!(case, Case::Deadline) {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let problem = match case {
            Case::Denied => {
                json!({"code":403,"category":"authorization","reason":"current-authority-refused"})
            }
            Case::OtherConflict => {
                json!({"code":409,"category":"state","reason":"different-conflict"})
            }
            _ => json!({"code":409,"category":"state","reason":"resync_required"}),
        };
        if attempt == 2 {
            signal.notify_one();
        }
        write(writer, signer, &request, "error", problem).await;
        if matches!(case, Case::Denied | Case::OtherConflict) || attempt == 2 {
            return;
        }
        let Some(next) = read(reader).await else {
            return;
        };
        request = next;
        assert_eq!(request.kind(), "conversation.get");
        assert!(request.original()["payload"].get("page").is_none());
        if matches!(case, Case::Deadline | Case::Cancelled) {
            if matches!(case, Case::Cancelled) {
                signal.notify_one();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            return;
        }
        if matches!(case, Case::Evicted) {
            write(
                writer,
                signer,
                &request,
                "conversation.snapshot",
                snapshot(2, "fresh", false),
            )
            .await;
            write(writer, signer, &request, "ack", json!({})).await;
            return;
        }
    }
}
async fn refresh(
    case: Case,
    timeout: Duration,
) -> (
    crate::sdk::Result<crate::sdk::ConversationSnapshot>,
    Duration,
    bool,
) {
    let (peer, host, signal) = controlled(Fault::SdkSnapshot(case)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let began = tokio::time::Instant::now();
    let result = client
        .conversation_by_id("conv_one".parse().unwrap())
        .refresh(RequestOptions::new(timeout).unwrap())
        .await;
    let elapsed = began.elapsed();
    let repeated_completed = matches!(case, Case::Repeated)
        && tokio::time::timeout(Duration::from_millis(50), signal.notified())
            .await
            .is_ok();
    client
        .close(RequestOptions::new(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
    (result, elapsed, repeated_completed)
}
#[tokio::test]
async fn snapshot_eviction_recovers_without_mixing_partial_facts_or_mutating() {
    let (result, _, _) = refresh(Case::Evicted, Duration::from_secs(3)).await;
    let snapshot = result.unwrap();
    assert_eq!(snapshot.revision.get(), 2);
    assert_eq!(snapshot.participants.len(), 1);
    assert_eq!(snapshot.participants[0].id.as_str(), "fresh");
    assert_eq!(snapshot.cursor.as_str(), "cursor-2");
}
#[tokio::test]
async fn repeated_snapshot_eviction_stops_after_two_read_only_restarts() {
    let (result, _, repeated_completed) = refresh(Case::Repeated, Duration::from_secs(3)).await;
    assert!(repeated_completed);
    let error = result.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(error.problem.unwrap().reason, "resync_required");
}
#[tokio::test]
async fn snapshot_recovery_never_retries_initial_reads_authorization_or_unrelated_conflicts() {
    for (case, kind, reason) in [
        (
            Case::Denied,
            ErrorKind::Authentication,
            "current-authority-refused",
        ),
        (
            Case::OtherConflict,
            ErrorKind::Conflict,
            "different-conflict",
        ),
        (Case::InitialResync, ErrorKind::Conflict, "resync_required"),
    ] {
        let (result, _, _) = refresh(case, Duration::from_secs(3)).await;
        let error = result.unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.problem.unwrap().reason, reason);
    }
}
#[tokio::test]
async fn snapshot_recovery_keeps_the_original_read_deadline() {
    let (result, elapsed, _) = refresh(Case::Deadline, Duration::from_millis(500)).await;
    assert_eq!(result.unwrap_err().kind, ErrorKind::Deadline);
    assert!(elapsed < Duration::from_millis(750));
}
#[tokio::test]
async fn snapshot_recovery_keeps_the_original_cancellation_signal() {
    let (peer, host, ready) = controlled(Fault::SdkSnapshot(Case::Cancelled)).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let options = RequestOptions::new(Duration::from_secs(3)).unwrap();
    let cancel = options.signal.clone();
    let conversation = client.conversation_by_id("conv_one".parse().unwrap());
    let result = conversation.refresh(options);
    tokio::pin!(result);
    tokio::select! {
        value = &mut result => panic!("read ended before fresh continuation recovery: {value:?}"),
        _ = ready.notified() => {}
    }
    cancel.cancel();
    assert_eq!(result.await.unwrap_err().kind, ErrorKind::Cancelled);
    client
        .close(RequestOptions::new(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
