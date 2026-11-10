use super::*;

fn update(state: &str, revision: u64) -> payloads::OperationUpdated {
    serde_json::from_value(json!({
        "operation_id":"original-subscription", "record_id":"original-record",
        "command_type":"stream.subscribe", "expected_revision":4,
        "state":state, "revision":revision,
        "result": if state == "completed" {
            json!({"connection_id":"original-connection","stream_revision":5})
        } else { Value::Null }
    }))
    .unwrap()
}
fn observe(receipt: &mut SubscriptionReceipt, update: &payloads::OperationUpdated) -> Result<bool> {
    receipt.observe(update, "original-subscription", "original-connection", 4)
}

#[test]
fn accepted_subscription_waits_for_original_completed_media_result() {
    let mut receipt = SubscriptionReceipt::default();
    for (revision, state) in ["accepted", "preparing", "ready", "committing"]
        .into_iter()
        .enumerate()
    {
        assert!(!observe(&mut receipt, &update(state, revision as u64 + 1)).unwrap());
    }
    assert!(observe(&mut receipt, &update("completed", 5)).unwrap());
    // Replaying the caller's original ID/revision may return the same final receipt.
    assert!(observe(&mut SubscriptionReceipt::default(), &update("completed", 5)).unwrap());
}

#[test]
fn pending_subscription_cannot_switch_original_record_or_intent() {
    for field in [
        "record_id",
        "operation_id",
        "command_type",
        "expected_revision",
        "revision",
    ] {
        let mut receipt = SubscriptionReceipt::default();
        assert!(!observe(&mut receipt, &update("preparing", 2)).unwrap());
        let mut value = serde_json::to_value(update("completed", 3)).unwrap();
        value[field] = match field {
            "expected_revision" => json!(5),
            "revision" => json!(1),
            _ => json!("foreign"),
        };
        assert!(
            observe(&mut receipt, &serde_json::from_value(value).unwrap()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn subscription_terminal_failure_or_foreign_media_never_completes() {
    for state in ["failed", "cancelled", "interrupted", "unknown"] {
        assert!(
            observe(&mut SubscriptionReceipt::default(), &update(state, 2)).is_err(),
            "{state}"
        );
    }
    for result in [
        Value::Null,
        json!({"connection_id":"foreign","stream_revision":5}),
        json!({"connection_id":"original-connection","stream_revision":6}),
    ] {
        let mut value = update("completed", 2);
        value.result = if result.is_null() { None } else { Some(result) };
        assert!(observe(&mut SubscriptionReceipt::default(), &value).is_err());
    }
    let mut pending = update("preparing", 2);
    pending.problem = Some(json!({"code":"refused"}));
    assert!(observe(&mut SubscriptionReceipt::default(), &pending).is_err());
}

async fn controlled_audio(
    stall: bool,
) -> (
    PublicNativeAudio,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Notify>,
) {
    let (mut client, server, release_ack) = crate::canonical::tests::controlled(
        crate::canonical::tests::Fault::PublicAudioSubscription(stall),
    )
    .await;
    if stall {
        client.deadline = Duration::from_millis(180);
    }
    let peer = NativeAudioPeer::for_controlled_subscription_test(
        client,
        CommandScope {
            conversation_id: Some("conv_one".into()),
            session_id: Some("session_one".into()),
            connection_id: Some("original-connection".into()),
            ..Default::default()
        },
    );
    (
        PublicNativeAudio {
            peer,
            capabilities: json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"}),
        },
        server,
        release_ack,
    )
}
fn subscription() -> Vec<payloads::StreamSubscription> {
    serde_json::from_value(json!([{"from_participant":"part_source","kinds":["audio"]}])).unwrap()
}
#[tokio::test]
async fn public_audio_signed_preparing_get_completed_waits_for_final_ack_before_new_handle() {
    let (mut audio, server, release_ack) = controlled_audio(false).await;
    assert_eq!((audio.peer.handle, audio.stream_revision()), (7, 4));
    let transport = audio.peer.client.connection.clone();
    let mut request = tokio::spawn(async move {
        let result = audio
            .subscribe_audio("original-subscription", 4, subscription())
            .await;
        (result, audio)
    });
    let marker = tokio::time::timeout(Duration::from_secs(1), transport.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&marker[..], b"completed-without-ack");
    // Even the actual completed receipt is not the response terminator.
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut request)
            .await
            .is_err()
    );
    release_ack.notify_one();
    let (result, mut audio) = tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap(), 5);
    assert_eq!((audio.peer.handle, audio.stream_revision()), (9, 5));
    audio.send_pcmu(&[0xff; 160]).unwrap();
    let marker = tokio::time::timeout(Duration::from_secs(1), transport.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&marker[..], b"new-handle-verified");
    audio.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn public_audio_signed_preparing_reads_keep_original_deadline_and_close_without_publishing() {
    let (mut audio, server, _) = controlled_audio(true).await;
    let before = tokio::time::Instant::now();
    let result = audio
        .subscribe_audio("original-subscription", 4, subscription())
        .await;
    assert!(matches!(result, Err(ClientError::Timeout)), "{result:?}");
    assert!(
        before.elapsed() < Duration::from_secs(1),
        "status reads renewed the original budget"
    );
    assert_eq!((audio.peer.handle, audio.stream_revision()), (7, 4));
    assert!(audio.peer.client.closed.load(Ordering::Acquire));
    assert!(matches!(
        audio.send_pcmu(&[0xff; 160]),
        Err(ClientError::Transport)
    ));
    audio.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
