use super::super::tests::{epoch, original, scope};
use super::*;
fn video() -> Vec<u8> {
    let mut bytes = vec![
        1, 0, 0, 7, 0, 0, 0, 0, 0x80, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 83, 0x10, 0x31, 0, 0,
    ];
    bytes = rvoip_uctp::substrate::tag_video_epoch_datagram(&bytes, 1)
        .unwrap()
        .to_vec();
    bytes
}
fn audio() -> Vec<u8> {
    vec![
        1, 0, 0, 8, 0, 0, 0, 4, 0x80, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 84, 1,
    ]
}
pub(crate) async fn emit(
    connection: &quinn::Connection,
    request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
) {
    super::super::tests::emit(request.clone(), writer, reader, signer, false).await;
    let receiver = signer
        .sign(epoch(request.id(), 1, false).original())
        .unwrap();
    let raw = serde_json::to_vec(&receiver).unwrap();
    writer
        .write_all(&(raw.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&raw).await.unwrap();
    let hint = pack_video_feedback_datagram(&VideoFeedbackDatagram {
        stream_local_id: 7,
        seq: 1,
        sender_ssrc: 2,
        media_ssrc: 83,
        activation_epoch: 1,
    })
    .unwrap();
    connection.send_datagram(hint.clone()).unwrap();
    connection.send_datagram(video().into()).unwrap();
    let actual = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    let parsed = observe_video_feedback_datagram(&actual).unwrap();
    assert_eq!(parsed.stream_local_id, 7);
    assert_eq!(parsed.activation_epoch, 1);
    assert_eq!(parsed.media_ssrc, 83);
    assert_eq!(parsed.seq, 0);
    // The duplicate must not acquire a fresh lifetime or force another frame.
    connection.send_datagram(hint).unwrap();
    connection.send_datagram(audio().into()).unwrap();
}
#[tokio::test]
async fn explicit_raw_video_owner_joins_signed_packets_requests_bootstrap_and_consumes_feedback_once(
) {
    use crate::canonical::tests::{controlled, Fault};
    let (mut client, server, _) = controlled(Fault::NativeVideoIo).await;
    assert!(client.claim_native_video_datagrams().is_err());
    client.start_signaling_reader().unwrap();
    let request = command_value("connection.offer", original(), scope()).unwrap();
    let id = request["id"].as_str().unwrap().to_owned();
    client.native_video.begin(id, scope(), original()).unwrap();
    client.write(request, true).await.unwrap();
    let answer = client.receive().await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    let callback = client.receive().await.unwrap();
    let expected = {
        let g = client.native_video.state.lock().unwrap();
        protocol(g.as_ref().unwrap().negotiating.confirmation(&callback)).unwrap()
    };
    client
        .answer_connection_offer(&callback, expected, &scope())
        .await
        .unwrap();
    let mut io = client.claim_native_video_datagrams().unwrap();
    assert!(client.claim_native_datagrams().is_err());
    let packet = tokio::time::timeout(Duration::from_secs(2), io.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet.bytes, video());
    let history = packet.video_epoch.unwrap();
    assert_eq!(history.epoch, format!("{:032x}", 1));
    assert_eq!(history.first_timestamp, 0);
    assert_eq!(history.ssrc, 83);
    assert!(io.take_keyframe_request(7, 83).unwrap());
    assert!(!io.take_keyframe_request(7, 83).unwrap());
    let packet = tokio::time::timeout(Duration::from_secs(2), io.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet.bytes, audio());
    assert!(packet.video_epoch.is_none());
    assert!(!io.take_keyframe_request(7, 83).unwrap());
    // A hint's original clock remains bounded even if a caller stops polling.
    let hint = pack_video_feedback_datagram(&VideoFeedbackDatagram {
        stream_local_id: 7,
        seq: 2,
        sender_ssrc: 2,
        media_ssrc: 83,
        activation_epoch: 1,
    })
    .unwrap();
    io.hint(&hint, Instant::now() - AGE).unwrap();
    assert!(!io.take_keyframe_request(7, 83).unwrap());
    client.close();
    assert!(io.take_keyframe_request(7, 83).is_err());
    drop(io);
    client.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn decoder_restart_quarantines_old_keyframes_and_preserves_feedback_bounds() {
    use crate::canonical::tests::{controlled, Fault};
    let (mut client, server, _) = controlled(Fault::NativeVideoIo).await;
    client.start_signaling_reader().unwrap();
    let request = command_value("connection.offer", original(), scope()).unwrap();
    let id = request["id"].as_str().unwrap().to_owned();
    client.native_video.begin(id, scope(), original()).unwrap();
    client.write(request, true).await.unwrap();
    let answer = client.receive().await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    let callback = client.receive().await.unwrap();
    let expected = {
        let guard = client.native_video.state.lock().unwrap();
        protocol(guard.as_ref().unwrap().negotiating.confirmation(&callback)).unwrap()
    };
    client
        .answer_connection_offer(&callback, expected, &scope())
        .await
        .unwrap();
    let mut io = client.claim_native_video_datagrams().unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), io.receive())
            .await
            .unwrap()
            .unwrap()
            .bytes,
        video()
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), io.receive())
            .await
            .unwrap()
            .unwrap()
            .bytes,
        audio()
    );
    let notice = io.shared.state.lock().unwrap().as_ref().unwrap().active[&(7, false)].clone();
    // These original fragments exercise recovery state, not codec decoding.
    // The history/allocation and feedback transport came from the signed peer.
    let packet = |sequence: u16, key: bool, arrived: Instant| {
        let mut bytes = video();
        bytes[4..8].copy_from_slice(&u32::from(sequence).to_be_bytes());
        bytes[26..28].copy_from_slice(&sequence.to_be_bytes());
        bytes[28..32].copy_from_slice(&(u32::from(sequence) * 3000).to_be_bytes());
        bytes[37] = if key { 0x30 } else { 0x31 };
        Waiting {
            packet: observe_video_epoch_datagram(&bytes).unwrap(),
            bytes,
            arrived,
        }
    };
    io.receive_video(packet(1, true, Instant::now()), notice.clone())
        .unwrap();
    assert!(!io.recovery[&7].waiting);
    let sequence = io.recovery[&7].sequence;
    let last_request = io.recovery[&7].last_request;
    assert!(io.reset_video_receive_decoder(8).is_err());
    assert!(io.reset_video_receive_decoder(65535).is_err());
    let at = io.reset_video_receive_decoder(7).unwrap();
    assert!(io.recovery[&7].waiting);
    assert_eq!(io.recovery[&7].sequence, sequence);
    assert_eq!(io.recovery[&7].last_request, last_request);
    let old_arrival = at - Duration::from_micros(1);
    let retained = io
        .receive_video(packet(2, true, old_arrival), notice.clone())
        .unwrap();
    assert_eq!(retained.received_at, old_arrival);
    assert_eq!(retained.video_epoch.as_ref(), Some(&notice.history));
    assert!(
        io.recovery[&7].waiting,
        "quarantined keyframe bootstrapped a new decoder"
    );
    assert_eq!(io.recovery[&7].assembler.diagnostics().completed_frames, 0);
    // A caller cannot bypass the existing100ms rate limit by resetting again.
    io.reset_video_receive_decoder(7).unwrap();
    assert_eq!(io.recovery[&7].sequence, sequence);
    assert_eq!(io.recovery[&7].last_request, last_request);
    tokio::time::sleep(Duration::from_millis(110)).await;
    io.receive_video(packet(3, false, Instant::now()), notice.clone())
        .unwrap();
    assert!(io.recovery[&7].waiting);
    assert_eq!(io.recovery[&7].sequence, sequence + 1);
    io.receive_video(packet(4, false, Instant::now()), notice.clone())
        .unwrap();
    assert_eq!(
        io.recovery[&7].sequence,
        sequence + 1,
        "feedback rate renewed"
    );
    io.receive_video(packet(5, true, Instant::now()), notice)
        .unwrap();
    assert!(!io.recovery[&7].waiting);
    assert_eq!(io.recovery[&7].last_frame, Some(5));
    client.close();
    assert!(io.reset_video_receive_decoder(7).is_err());
    drop(io);
    client.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

// Real signed allocation/history and real QUIC feedback bytes. The controlled
// peer deliberately loses the first request and sends NO more media until a
// distinct new request arrives on the same original authenticated connection.
pub(crate) async fn emit_recovery(
    connection: &quinn::Connection,
    request: Envelope,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    lose_first: bool,
) {
    super::super::tests::emit(request.clone(), writer, reader, signer, false).await;
    let receiver = signer
        .sign(epoch(request.id(), 1, false).original())
        .unwrap();
    let raw = serde_json::to_vec(&receiver).unwrap();
    writer
        .write_all(&(raw.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&raw).await.unwrap();
    connection.send_datagram(video().into()).unwrap();
    if !lose_first {
        return;
    }
    let first = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    let first = observe_video_feedback_datagram(&first).unwrap();
    assert_eq!(
        (
            first.stream_local_id,
            first.seq,
            first.activation_epoch,
            first.media_ssrc
        ),
        (7, 0, 1, 83)
    );
    let received = Instant::now();
    // The first request is intentionally not acknowledged or routed. This
    // controlled loss must not require another video/audio/clock datagram.
    let second = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    let second = observe_video_feedback_datagram(&second).unwrap();
    assert_eq!(
        (
            second.stream_local_id,
            second.seq,
            second.activation_epoch,
            second.media_ssrc
        ),
        (7, 1, 1, 83)
    );
    assert!(
        received.elapsed() >= Duration::from_millis(90),
        "100ms sender cooldown was bypassed"
    );
    connection.send_datagram(audio().into()).unwrap();
}
async fn recovery_owner(
    loss: bool,
    budget: Duration,
) -> (
    CanonicalClient,
    NativeVideoDatagrams,
    tokio::task::JoinHandle<()>,
) {
    use crate::canonical::tests::{controlled, Fault};
    let (mut client, server, _) = controlled(Fault::NativeVideoRecovery(loss)).await;
    client.start_signaling_reader().unwrap();
    let request = command_value("connection.offer", original(), scope()).unwrap();
    client
        .native_video
        .begin(request["id"].as_str().unwrap().into(), scope(), original())
        .unwrap();
    client.write(request, true).await.unwrap();
    assert_eq!(client.receive().await.unwrap().kind(), "connection.answer");
    let callback = client.receive().await.unwrap();
    let expected = {
        let guard = client.native_video.state.lock().unwrap();
        protocol(guard.as_ref().unwrap().negotiating.confirmation(&callback)).unwrap()
    };
    client
        .answer_connection_offer(&callback, expected, &scope())
        .await
        .unwrap();
    client.deadline = budget;
    let mut io = client.claim_native_video_datagrams().unwrap();
    io.reset_video_receive_decoder(7).unwrap();
    assert_eq!(io.receive().await.unwrap().bytes, video());
    (client, io, server)
}
#[tokio::test]
async fn private_bootstrap_lost_request_retries_new_sequence_during_complete_receive_silence() {
    let (mut client, mut io, server) = recovery_owner(true, Duration::from_secs(2)).await;
    let history = io.recovery[&7].history.clone();
    let first = io.recovery[&7].last_request.unwrap();
    let packet = tokio::time::timeout(Duration::from_millis(500), io.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(packet.bytes, audio());
    assert_eq!(io.recovery[&7].history, history);
    assert_eq!(io.recovery[&7].sequence, 2);
    assert!(
        io.recovery[&7].last_request.unwrap().duration_since(first) >= Duration::from_millis(100)
    );
    assert!(io.recovery[&7].waiting);
    client.close();
    drop(io);
    client.shutdown().await.unwrap();
    server.await.unwrap();
}
#[tokio::test]
async fn recovery_wake_does_not_renew_original_receive_timeout_or_survive_cancelled_poll() {
    let (mut client, mut io, server) = recovery_owner(false, Duration::from_millis(260)).await;
    let began = Instant::now();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), io.receive())
            .await
            .unwrap(),
        Err(ClientError::Timeout)
    ));
    assert!(
        began.elapsed() >= Duration::from_millis(250)
            && began.elapsed() < Duration::from_millis(600)
    );
    assert!(io.recovery[&7].sequence >= 3);
    let before = io.recovery[&7].sequence;
    tokio::time::sleep(Duration::from_millis(140)).await;
    assert_eq!(
        io.recovery[&7].sequence, before,
        "receive owner spawned detached recovery"
    );
    client.close();
    drop(io);
    client.shutdown().await.unwrap();
    server.await.unwrap();
}
#[tokio::test]
async fn recovery_silence_refuses_withdrawn_signed_history_suspended_owner_and_closed_peer() {
    for suspended in [false, true] {
        let (mut client, mut io, server) = recovery_owner(false, Duration::from_secs(2)).await;
        let before = io.recovery[&7].sequence;
        {
            let mut guard = io.shared.state.lock().unwrap();
            if suspended {
                guard.as_mut().unwrap().suspended = true;
            } else {
                guard.as_mut().unwrap().active.remove(&(7, false));
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(160), io.receive())
                .await
                .is_err()
        );
        assert_eq!(io.recovery[&7].sequence, before);
        client.close();
        assert!(io.retry_waiting_receivers().is_err());
        assert_eq!(io.recovery[&7].sequence, before);
        drop(io);
        client.shutdown().await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn completed_recovery_key_does_not_request_again_for_an_old_partial() {
    let (mut client, mut io, server) = recovery_owner(false, Duration::from_secs(2)).await;
    let notice = io.shared.state.lock().unwrap().as_ref().unwrap().active[&(7, false)].clone();
    let mut push = |sequence: u16, timestamp: u32, start: bool, marker: bool, key: bool| {
        let mut bytes = video();
        bytes[4..8].copy_from_slice(&u32::from(sequence).to_be_bytes());
        bytes[25] = 96 | if marker { 0x80 } else { 0 };
        bytes[26..28].copy_from_slice(&sequence.to_be_bytes());
        bytes[28..32].copy_from_slice(&timestamp.to_be_bytes());
        bytes[36] = if start { 0x10 } else { 0 };
        bytes[37] = if key { 0x30 } else { 0x31 };
        let packet = Waiting {
            packet: observe_video_epoch_datagram(&bytes).unwrap(),
            bytes,
            arrived: Instant::now(),
        };
        io.receive_video(packet, notice.clone()).unwrap();
        let recovery = &io.recovery[&7];
        (
            recovery.waiting,
            recovery.sequence,
            recovery.assembler.diagnostics().pending_frames,
        )
    };
    push(10, 100, true, true, true);
    let mut before = 0;
    for i in 0..8 {
        before = push(11 + i * 2, 200 + u32::from(i) * 100, true, false, false).2;
    }
    let key = push(27, 1000, true, true, true);
    push(30, 1200, false, true, false);
    let delta = push(28, 1100, true, true, false);
    let reordered = push(29, 1200, true, false, false);
    let gap = push(32, 1400, true, true, false);
    client.close();
    drop(io);
    client.shutdown().await.unwrap();
    server.await.unwrap();

    assert_eq!(
        before, 8,
        "original incomplete frames must fill the bounded assembler"
    );
    assert!(!key.0, "the exact complete recovery key must end the wait");
    assert!(
        !delta.0,
        "obsolete partial forced a key wait after exact recovery"
    );
    assert_eq!(
        delta.1, key.1,
        "contiguous delta cannot schedule another PLI"
    );
    assert!(
        !reordered.0,
        "newer reordered tail must survive the key boundary"
    );
    assert_eq!(reordered.2, 0);
    assert!(gap.0, "a genuinely newer loss must still request recovery");
}
