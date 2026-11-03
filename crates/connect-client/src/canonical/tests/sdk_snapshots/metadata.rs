//! Signed read-only metadata refusal; no mutation or stale authority is accepted.
use super::*;
#[derive(Clone, Copy)]
pub(in crate::canonical) enum Mode {
    Initial,
    Page,
    Forbidden,
    Other503,
    Deadline,
    Cancelled,
}
pub(super) async fn emit(
    mode: Mode,
    mut request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    assert_eq!(request.kind(), "conversation.get");
    assert!(request.original()["payload"].get("page").is_none());
    if matches!(mode, Mode::Page) {
        write(
            writer,
            signer,
            &request,
            "conversation.snapshot",
            snapshot(1, "original", true),
        )
        .await;
        write(writer, signer, &request, "ack", json!({})).await;
        request = read(reader).await.unwrap();
        assert_eq!(request.original()["payload"]["page"], "evicted-page");
    }
    let payload = request.original()["payload"].clone();
    let problem = match mode {
        Mode::Forbidden => {
            json!({"code":403,"category":"authorization","reason":"current-authority-refused"})
        }
        Mode::Other503 => json!({"code":503,"category":"server","reason":"unrelated-failure"}),
        _ => json!({"code":503,"category":"authorization","reason":"authority_unavailable"}),
    };
    write(writer, signer, &request, "error", problem.clone()).await;
    if matches!(mode, Mode::Cancelled) {
        signal.notify_one();
    }
    if matches!(mode, Mode::Forbidden | Mode::Other503) {
        // Any retry, including a fresh query, makes this controlled case fail.
        assert!(read(reader).await.is_none());
        return;
    }
    while let Some(next) = read(reader).await {
        assert_eq!(next.kind(), "conversation.get");
        assert_eq!(next.original()["payload"], payload);
        assert_eq!(next.original()["cid"], request.original()["cid"]);
        assert!(next.scope(Scope::Operation).is_none());
        if matches!(mode, Mode::Deadline | Mode::Cancelled) {
            write(writer, signer, &next, "error", problem.clone()).await;
            continue;
        }
        let response = if matches!(mode, Mode::Page) {
            // The continuation belongs to the original snapshot; no mixed revision.
            json!({"revision":1,"cursor":"cursor-1","items":[],"has_more":false})
        } else {
            snapshot(2, "current", false)
        };
        write(writer, signer, &next, "conversation.snapshot", response).await;
        write(writer, signer, &next, "ack", json!({})).await;
        assert!(read(reader).await.is_none());
        return;
    }
}
#[tokio::test]
async fn signed_metadata_refusal_recovers_initial_or_same_page_without_mutation() {
    for mode in [Mode::Initial, Mode::Page] {
        let (result, _, _) = refresh(Case::Metadata(mode), Duration::from_secs(3)).await;
        let value = result.unwrap();
        let (revision, participant) = if matches!(mode, Mode::Page) {
            (1, "original")
        } else {
            (2, "current")
        };
        assert_eq!(value.revision.get(), revision);
        assert_eq!(value.participants.len(), 1);
        assert_eq!(value.participants[0].id.as_str(), participant);
    }
}
#[tokio::test]
async fn metadata_recovery_stops_on_current_denial_or_unrelated_service_failure() {
    for (mode, kind) in [
        (Mode::Forbidden, ErrorKind::Authentication),
        (Mode::Other503, ErrorKind::Application),
    ] {
        let (result, _, _) = refresh(Case::Metadata(mode), Duration::from_secs(3)).await;
        let error = result.unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.certainty, crate::sdk::Certainty::Refused);
    }
}
#[tokio::test]
async fn metadata_recovery_preserves_original_deadline_and_cancellation() {
    let (result, elapsed, _) =
        refresh(Case::Metadata(Mode::Deadline), Duration::from_millis(180)).await;
    assert_eq!(result.unwrap_err().kind, ErrorKind::Deadline);
    assert!(elapsed < Duration::from_millis(400));
    let (peer, host, ready) = controlled(Fault::SdkSnapshot(Case::Metadata(Mode::Cancelled))).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let options = RequestOptions::new(Duration::from_secs(3)).unwrap();
    let cancel = options.signal.clone();
    let conversation = client.conversation_by_id("conv_one".parse().unwrap());
    let result = conversation.refresh(options);
    tokio::pin!(result);
    tokio::select! {
        result = &mut result => panic!("read ended before controlled cancellation: {result:?}"),
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
