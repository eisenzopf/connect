//! Real native SDK IO while its ORIGINAL signed WRITE is pending, not a held response.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Default)]
pub(in crate::canonical) struct WritePause {
    armed: AtomicBool,
    held: AtomicBool,
    after: AtomicBool,
    entered: Notify,
    release: Notify,
}
struct Held<'a>(&'a AtomicBool);
impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl WritePause {
    pub(in crate::canonical) async fn enter(&self) {
        if self.after.load(Ordering::Acquire) {
            return;
        }
        self.pause().await;
    }
    pub(in crate::canonical) async fn after_answer(&self) {
        if !self.after.load(Ordering::Acquire) {
            return;
        }
        self.pause().await;
    }
    async fn pause(&self) {
        if !self.armed.swap(false, Ordering::AcqRel) {
            return;
        }
        self.held.store(true, Ordering::Release);
        let _held = Held(&self.held);
        self.entered.notify_one();
        self.release.notified().await;
    }
}
#[derive(Clone, Copy, Debug)]
pub(in crate::canonical) enum Case {
    Progress,
    Bye,
    Expiry,
    Close,
    Sibling,
    Arrival,
}
pub(super) async fn emit(
    connection: &quinn::Connection,
    case: Case,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
) {
    if matches!(case, Case::Sibling) {
        let first = read(reader).await;
        assert_eq!(first.kind(), "connection.offer");
        assert_eq!(first.scope(Scope::Connection), Some("connection_two"));
        assert_eq!(first.scope(Scope::Session), Some("session_two"));
        Box::pin(super::emit(
            connection,
            super::Case::IsolationSecond,
            first,
            writer,
            reader,
            signer,
            signal,
        ))
        .await;
        return;
    }
    if matches!(case, Case::Close | Case::Arrival) {
        connection.closed().await;
        return;
    }
    for index in 0..2 {
        let bytes = match connection.read_datagram().await {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        assert_eq!(bytes.as_ref(), datagram(7, 100 + index, &raw_rtp(0)));
        connection
            .send_datagram(datagram(8, 200 + index, &raw_rtp(96)).into())
            .unwrap();
    }
    if matches!(case, Case::Bye) {
        let bytes = match connection.read_datagram().await {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        assert_eq!(bytes.as_ref(), datagram(7, 102, &raw_rtp(0)));
        let raw = serde_json::to_vec(
            &signer
                .sign(&envelope("auth.bye", json!({"reason":"revoked"})))
                .unwrap(),
        )
        .unwrap();
        writer
            .write_all(&(raw.len() as u32).to_be_bytes())
            .await
            .unwrap();
        writer.write_all(&raw).await.unwrap();
    } else if matches!(case, Case::Progress) {
        let query = read(reader).await;
        assert_eq!(query.kind(), "operation.get");
        status(writer, signer, &query, "completed").await;
    }
    tokio::select! {
        _ = connection.closed() => (),
        bytes = connection.read_datagram() => assert!(bytes.is_err(), "no media after cancellation/expiry/close"),
    }
}
/// Two independently negotiated Connections share this one SDK. The actual
/// selector journal and callback are the existing reconciliation fixture.
pub(super) async fn emit_sibling(
    connection: &quinn::Connection,
    writer: &mut quinn::SendStream,
    reader: &mut quinn::RecvStream,
    signer: &Sig9421Signer,
    signal: &Notify,
    caps: &Value,
) {
    let select = read(reader).await;
    assert_eq!(select.kind(), "stream.subscribe");
    assert_eq!(select.scope(Scope::Connection), Some("connection_one"));
    reconciliations::reply(
        writer,
        signer,
        &select,
        true,
        "completed",
        reconciliations::Case::Normal,
        false,
    )
    .await;
    let query = read(reader).await;
    assert_eq!(query.original()["payload"]["operation_id"], "selector");
    reconciliations::reply(
        writer,
        signer,
        &query,
        true,
        "completed",
        reconciliations::Case::Normal,
        false,
    )
    .await;
    signal.notified().await;
    let callback =
        reconciliations::send_callback(writer, signer, caps, reconciliations::Case::Normal).await;
    // The parent now performs its genuine journal query; its ORIGINAL write is
    // held by WritePause. Sibling media must complete before any query byte.
    for index in 0..2 {
        let bytes = match connection.read_datagram().await {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        assert_eq!(bytes.as_ref(), datagram(107, 100 + index, &raw_rtp(0)));
        connection
            .send_datagram(datagram(108, 200 + index, &raw_rtp(96)).into())
            .unwrap();
    }
    let query = read(reader).await;
    assert_eq!(
        query.original()["payload"]["operation_id"],
        "auto-selection"
    );
    reconciliations::reply(
        writer,
        signer,
        &query,
        false,
        "preparing",
        reconciliations::Case::Normal,
        false,
    )
    .await;
    let answer = read(reader).await;
    assert_eq!(answer.kind(), "connection.answer");
    assert_eq!(answer.scope(Scope::Reply), Some(callback.id()));
    assert_eq!(answer.scope(Scope::Connection), Some("connection_one"));
    let query = read(reader).await;
    assert_eq!(
        query.original()["payload"]["operation_id"],
        "auto-selection"
    );
    reconciliations::reply(
        writer,
        signer,
        &query,
        false,
        "completed",
        reconciliations::Case::Normal,
        true,
    )
    .await;
    tokio::select! {
        _ = connection.closed() => (),
        bytes = connection.read_datagram() => assert!(bytes.is_err(), "no cancelled sibling media"),
    }
}

async fn caught<T>(
    work: impl std::future::Future<Output = T>,
) -> std::result::Result<T, Box<dyn std::any::Any + Send>> {
    let mut work = std::pin::pin!(work);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::future::Future::poll(work.as_mut(), cx)
        })) {
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(panic) => std::task::Poll::Ready(Err(panic)),
        }
    })
    .await
}
async fn exercise(case: Case) {
    let (mut peer, mut host, signal) =
        controlled(Fault::SdkNative(super::Case::Isolation(case))).await;
    peer.deadline = Duration::from_secs(8);
    if matches!(case, Case::Expiry) {
        peer.authenticated_until = chrono::Utc::now() + chrono::Duration::seconds(3);
    }
    let original_reader = peer.native_datagram_reader.clone();
    let physical_closed = peer.closed.clone();
    let pause = Arc::new(WritePause::default());
    peer.write_pause = Some(pause.clone());
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let mut admitted = false;
    let mut entered = false;
    let mut sent_count = 0;
    let mut received_count = 0;
    let mut first_send_error = None;
    let work = caught(async {
        let operation = session.offer_connection("connection_one".parse().unwrap(), offered(), mutation()).await.unwrap();
        assert_eq!(operation.status(options()).await.unwrap().state, sdk::OperationState::Preparing);
        let connection = operation.wait(options()).await.unwrap().value;
        let media_connection = if matches!(case, Case::Sibling) {
            client.conversation_by_id("conv_one".parse().unwrap()).session_by_id("session_two".parse().unwrap()).offer_connection("connection_two".parse().unwrap(), offered(), MutationOptions {
                operation_id: "native-second".parse().unwrap(), request: options(), extensions: None,
            }).await.unwrap().wait(options()).await.unwrap().value
        } else { connection.clone() };
        if matches!(case, Case::Sibling) {
            connection.subscribe(NonEmpty { first: sdk::StreamSelector::Participant { participant_id: "part_remote".parse().unwrap(), kinds: None }, rest: vec![] },
                sdk::StreamRevision::try_from(1).unwrap(), MutationOptions { operation_id: "selector".parse().unwrap(), request: options(), extensions: None }).await.unwrap().wait(options()).await.unwrap();
        }
        let audio = media_connection.stream_by_id("audio".parse().unwrap());
        let video = media_connection.stream_by_id("video".parse().unwrap());
        admitted = true;
        pause.armed.store(true, Ordering::Release);
        let mut control = if matches!(case, Case::Sibling) { None } else { Some(Box::pin(operation.status(RequestOptions::new(Duration::from_secs(8)).unwrap()))) };
        if matches!(case, Case::Sibling) {
            signal.notify_one();
            tokio::time::timeout(Duration::from_secs(2), pause.entered.notified()).await.unwrap();
        } else { tokio::select! {
            _ = pause.entered.notified() => (),
            result = control.as_mut().expect("original control future").as_mut() => { let _ = result; panic!("control returned before ORIGINAL write barrier"); },
            _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("write barrier not reached"),
        }
        }
        assert!(pause.held.load(Ordering::Acquire)); entered = true;
        if matches!(case, Case::Close) {
            client.close(options()).await.unwrap();
            assert!(!original_reader.load(Ordering::Acquire));
            assert!(physical_closed.load(Ordering::Acquire));
            assert!(control.as_mut().unwrap().await.is_err()); return;
        }
        // OLD single-task SDK expires here while the ORIGINAL write is held.
        for index in 0..2 {
            let sent = audio.send_rtp(sdk::RtpPacket::new(100 + index, raw_rtp(0)).unwrap(), RequestOptions::new(Duration::from_millis(200)).unwrap()).await;
            if let Err(error) = &sent {
                if sent_count == 0 { first_send_error = Some((error.kind, error.certainty)); }
            }
            assert!(sent.is_ok(), "admitted native send must progress while ORIGINAL signed write remains pending");
            let sent = sent.unwrap(); sent_count += 1;
            assert_eq!(sent.datagram_sequence, 100 + index);
            let received = video.receive_packet(RequestOptions::new(Duration::from_millis(200)).unwrap()).await;
            assert!(received.is_ok(), "admitted native receive must progress while ORIGINAL signed write remains pending");
            match received.unwrap() {
                sdk::MediaPacket::Rtp(packet) => {
                    assert_eq!(packet.datagram_sequence, 200 + index); assert_eq!(packet.bytes(), raw_rtp(96)); received_count += 1;
                },
                _ => panic!("original admitted native RTP expected"),
            }
        }
        assert!(pause.held.load(Ordering::Acquire), "both media directions progressed before control release");
        let cancelled = options(); cancelled.signal.cancel();
        let error = audio.send_rtp(sdk::RtpPacket::new(900, raw_rtp(0)).unwrap(), cancelled).await.unwrap_err();
        assert_eq!(error.certainty, sdk::Certainty::NotSubmitted);
        if matches!(case, Case::Sibling) {
            pause.release.notify_one();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if connection.native_reconciliation().unwrap().is_some_and(|v| v.phase == sdk::NativeReconciliationPhase::Recorded) { break; }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            assert_eq!(connection.native_negotiation().unwrap().unwrap().stream_revision.get(), 3);
            assert_eq!(media_connection.native_negotiation().unwrap().unwrap().stream_revision.get(), 1);
        } else if matches!(case, Case::Progress) {
            pause.release.notify_one(); assert_eq!(control.as_mut().unwrap().await.unwrap().state, sdk::OperationState::Completed);
        } else {
            if matches!(case, Case::Bye) { audio.send_rtp(sdk::RtpPacket::new(102, raw_rtp(0)).unwrap(), options()).await.unwrap(); }
            let ended = video.receive_packet(RequestOptions::new(Duration::from_secs(5)).unwrap()).await.unwrap_err();
            // Existing ended-Inbox public API is Cancelled; raw native cause
            // Authentication is checked separately below that API boundary.
            assert_eq!(ended.kind, sdk::ErrorKind::Cancelled);
            assert!(physical_closed.load(Ordering::Acquire));
            let control_error = control.as_mut().unwrap().await.err().expect("original control must stop with its physical peer");
            assert!(matches!(control_error.kind, sdk::ErrorKind::Cancelled | sdk::ErrorKind::Authentication | sdk::ErrorKind::TransportUncertain));
            client.close(options()).await.unwrap();
            let ended = audio.send_rtp(sdk::RtpPacket::new(901, raw_rtp(0)).unwrap(), options()).await.unwrap_err();
            assert_eq!(ended.kind, sdk::ErrorKind::Cancelled);
            assert_eq!(ended.certainty, sdk::Certainty::NotSubmitted);
        }
    }).await;
    // Even an OLD intended media failure must qualify ORIGINAL cleanup first.
    pause.release.notify_one();
    let close = client
        .close(RequestOptions::new(Duration::from_secs(60)).unwrap())
        .await;
    let (host_natural, joined) =
        match tokio::time::timeout(Duration::from_secs(10), &mut host).await {
            Ok(result) => (true, result),
            Err(_) => {
                host.abort();
                (false, (&mut host).await)
            }
        };
    eprintln!("sdk_native_isolation case={case:?} admitted={admitted} original_write_entered={entered} original_sends={sent_count} original_receives={received_count}");
    eprintln!(
        "sdk_native_isolation first_send_error_kind={:?} first_send_error_certainty={:?}",
        first_send_error.map(|value| value.0),
        first_send_error.map(|value| value.1)
    );
    eprintln!(
        "sdk_native_isolation cleanup client_closed={} host_joined={} reader_released={}",
        close.is_ok(),
        host_natural && joined.is_ok(),
        !original_reader.load(Ordering::Acquire)
    );
    assert!(
        close.is_ok() && host_natural && joined.is_ok() && !original_reader.load(Ordering::Acquire),
        "original SDK/QUIC owners must close before work qualification"
    );
    if let Err(panic) = work {
        std::panic::resume_unwind(panic);
    }
}
#[tokio::test]
async fn native_media_both_directions_progress_while_original_signed_write_is_pending() {
    exercise(Case::Progress).await;
}
#[tokio::test]
async fn native_media_pending_control_does_not_bypass_verified_auth_bye() {
    exercise(Case::Bye).await;
}
#[tokio::test]
async fn native_media_pending_control_does_not_renew_original_authority_expiry() {
    exercise(Case::Expiry).await;
}
#[tokio::test]
async fn native_media_child_and_original_reader_join_when_close_interrupts_pending_write() {
    exercise(Case::Close).await;
}

#[tokio::test]
async fn native_sibling_connection_progresses_during_original_reconciliation_write() {
    exercise(Case::Sibling).await;
}

async fn initial_arrival(mode: u8) {
    let (mut peer, mut host, _) =
        controlled(Fault::SdkNative(super::Case::Isolation(Case::Arrival))).await;
    peer.deadline = Duration::from_secs(8);
    let reader = peer.native_datagram_reader.clone();
    let pause = Arc::new(WritePause::default());
    pause.after.store(true, Ordering::Release);
    pause.armed.store(true, Ordering::Release);
    peer.write_pause = Some(pause.clone());
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let session = client
        .conversation_by_id("conv_one".parse().unwrap())
        .session_by_id("session_one".parse().unwrap());
    let work = caught(async {
        let mut streams = offered(); streams.first.direction = Direction::Sendrecv;
        let operation = session.offer_connection("connection_one".parse().unwrap(), streams, mutation()).await.unwrap();
        let mut completed = Box::pin(operation.wait(RequestOptions::new(Duration::from_secs(8)).unwrap()));
        tokio::select! {
            _ = pause.entered.notified() => (),
            result = completed.as_mut() => { let _ = result; panic!("publication passed the original after-answer barrier"); },
            _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("actual-answer barrier not reached"),
        }
        let restored = session.connection_by_id("connection_one".parse().unwrap());
        assert!(matches!(restored.native_negotiation(), Ok(None)), "reservation cannot create public binding authority");
        for name in ["audio", "video"] {
            let denied = restored.stream_by_id(name.parse().unwrap()).receive_packet(options()).await.unwrap_err();
            assert_eq!(denied.kind, sdk::ErrorKind::Unsupported);
        }
        let denied = restored.stream_by_id("audio".parse().unwrap()).send_rtp(sdk::RtpPacket::new(901, raw_rtp(0)).unwrap(), options()).await.unwrap_err();
        assert_eq!(denied.kind, sdk::ErrorKind::Unsupported);
        assert_eq!(denied.certainty, sdk::Certainty::NotSubmitted);
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                let stats = client.media_stats().unwrap();
                if stats.received_datagrams >= 2 {
                    assert_eq!(stats.unknown_handles, 0); assert_eq!(stats.queued_packets, 2); break;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert!(pause.held.load(Ordering::Acquire));
        if mode == 1 {
            // Actual cancellation while only scoped private queues exist must
            // clear them before the SAME child is joined, with no publication.
            client.close(RequestOptions::new(Duration::from_secs(60)).unwrap()).await.unwrap();
            assert!(completed.await.is_err());
            let stats = client.media_stats().unwrap();
            assert_eq!(stats.queued_packets, 0); assert_eq!(stats.queued_bytes, 0);
            assert!(restored.native_negotiation().is_err());
            return;
        }
        if mode == 2 { tokio::time::sleep(Duration::from_millis(210)).await; }
        pause.release.notify_one();
        let connection = completed.await.unwrap().value;
        for (name, sequence, pt) in [("audio", 299, 0), ("video", 300, 96)] {
            match connection.stream_by_id(name.parse().unwrap()).receive_packet(options()).await.unwrap() {
                sdk::MediaPacket::Rtp(packet) if mode == 0 => { assert_eq!(packet.datagram_sequence, sequence); assert_eq!(packet.bytes(), raw_rtp(pt)); },
                sdk::MediaPacket::Gap { dropped_packets } if mode == 2 => assert_eq!(dropped_packets, 1, "publication cannot renew original queue arrival age"),
                _ => panic!("exact initial original or explicit original-age loss required"),
            }
        }
    }).await;
    pause.release.notify_one();
    let close = client
        .close(RequestOptions::new(Duration::from_secs(60)).unwrap())
        .await;
    let (natural, joined) = match tokio::time::timeout(Duration::from_secs(10), &mut host).await {
        Ok(result) => (true, result),
        Err(_) => {
            host.abort();
            (false, (&mut host).await)
        }
    };
    assert!(
        close.is_ok() && natural && joined.is_ok() && !reader.load(Ordering::Acquire),
        "all original initial-reservation owners closed"
    );
    if let Err(panic) = work {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn native_initial_rtp_after_actual_answer_is_retained_without_early_public_binding() {
    initial_arrival(0).await;
    initial_arrival(1).await;
    initial_arrival(2).await;
}
