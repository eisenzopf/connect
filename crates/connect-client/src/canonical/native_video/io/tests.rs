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
