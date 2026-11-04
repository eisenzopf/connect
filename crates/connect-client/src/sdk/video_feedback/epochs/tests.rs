use super::*;
use serde_json::json;

pub(crate) fn notice_value(tag: u128, revision: u64, sequence: u32, timestamp: u32) -> Value {
    json!({"v":2,"type":"stream.epoch","id":format!("epoch-{revision}"),
        "ts":"2026-11-03T00:00:00Z","cid":"room","sid":"session","connid":"connection",
        "payload":{"profile":PROFILE,"strm_id":"camera","stream_local_id":7,
        "original_operation":"operation","original_offer":"offer",
        "incarnation":"00000000000000000000000000000001","stream_generation":1,
        "connection_generation":1,"revision":revision,"direction":"receiver",
        "epoch":format!("{tag:032x}"),"ssrc":83,"first_sequence":sequence,"first_timestamp":timestamp}})
}
pub(crate) fn notice(tag: u128, revision: u64, sequence: u32, timestamp: u32) -> Notice {
    Notice::read(&Envelope::from_value(notice_value(tag, revision, sequence, timestamp)).unwrap())
        .unwrap()
}
pub(crate) fn selected() -> NativeNegotiation {
    let n = notice(1, 1, 0, 0);
    NativeNegotiation {
        video_feedback: [(n.key.stream_id.clone(), n.allocation)].into(),
        source_clock: true,
        stream_revision: StreamRevision::try_from(1).unwrap(),
        streams: vec![NegotiatedStream {
            id: n.key.stream_id,
            medium: Medium::Video,
            direction: Direction::Recvonly,
            codec: json!({"name":"vp8","params":{"payload_type":96,"clock_rate":90000}}),
            stream_local_id: Some(7),
        }],
    }
}
fn packet(tag: Option<u128>, sequence: u32, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let mut rtp = vec![0x80, 96];
    rtp.extend_from_slice(&(sequence as u16).to_be_bytes());
    rtp.extend_from_slice(&timestamp.to_be_bytes());
    rtp.extend_from_slice(&ssrc.to_be_bytes());
    rtp.extend_from_slice(&[0x10, 0x31]);
    let bytes = streams::frame(7, sequence, &rtp);
    tag.map_or_else(
        || bytes.clone(),
        |tag| {
            rvoip_uctp::substrate::tag_video_epoch_datagram(&bytes, tag)
                .unwrap()
                .to_vec()
        },
    )
}
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(1)).unwrap()
}
fn install(inbox: &media::Inbox, selected: &NativeNegotiation) {
    let key = notice(1, 1, 0, 0).key;
    inbox
        .install(
            &key.conversation_id,
            &key.session_id,
            &key.connection_id,
            selected,
        )
        .unwrap();
}

#[test]
fn signed_video_notice_requires_exact_selected_allocation_and_packet_boundary() {
    let n = notice(11, 1, u32::MAX, u32::MAX - 10);
    let selected = selected();
    assert!(n.matches(&selected));
    for (field, value) in [
        ("epoch", json!("0".repeat(32))),
        ("epoch", json!("A".repeat(32))),
        ("incarnation", json!("0".repeat(32))),
        ("revision", json!(0)),
        ("revision", json!(9_007_199_254_740_992_u64)),
        ("first_sequence", json!(4_294_967_296_u64)),
        ("ssrc", json!(0)),
        ("direction", json!("both")),
        ("unexpected", json!(true)),
    ] {
        let mut raw = notice_value(11, 1, 0, 0);
        raw["payload"][field] = value;
        assert!(
            Envelope::from_value(raw).map_or(true, |f| Notice::read(&f).is_err()),
            "{field}"
        );
    }
    for (field, value) in [
        ("original_operation", json!("other-operation")),
        ("original_offer", json!("other-offer")),
        ("stream_generation", json!(2)),
        ("connection_generation", json!(2)),
        ("stream_local_id", json!(8)),
        ("strm_id", json!("other-stream")),
        ("direction", json!("publisher")),
    ] {
        let mut raw = notice_value(11, 1, 0, 0);
        raw["payload"][field] = value;
        let other = Notice::read(&Envelope::from_value(raw).unwrap()).unwrap();
        assert!(!other.matches(&selected), "{field}");
    }
    let make = |tag, seq, timestamp, ssrc| {
        let mut p = RtpPacket::new(seq, packet(None, seq, timestamp, ssrc)[8..].to_vec()).unwrap();
        p.video_tag = Some(tag);
        p
    };
    assert!(n.admits(&make(11, u32::MAX, u32::MAX - 10, 83)));
    assert!(n.admits(&make(11, 0, 3000, 83))); // native sequence and source clock wrap
    assert!(!n.admits(&make(12, 0, 3000, 83)));
    assert!(!n.admits(&make(11, u32::MAX, 0, 83)));
    assert!(!n.admits(&make(11, 0, 3000, 84)));
    assert!(n.definitely_stale(&make(11, u32::MAX - 1, 0, 83)));
    assert!(!n.definitely_stale(&make(12, 1, 6000, 83))); // future tag awaits its notice
}

#[tokio::test(start_paused = true)]
async fn video_epoch_join_preserves_arrival_age_and_refuses_previous_decoder_history() {
    let inbox = media::Inbox::default();
    install(&inbox, &selected());
    let first = notice(11, 1, u32::MAX, 9000);
    let key = first.key.clone();
    inbox.push(packet(Some(11), u32::MAX, 9000, 83)).unwrap();
    // The same packet becomes readable only when its exact notice arrives.
    tokio::select! {biased;
        _=inbox.receive(&key,options())=>panic!("unannounced packet admitted"),
        _=tokio::time::sleep(Duration::from_millis(50))=>{}
    }
    inbox.install_epoch(first.clone()).unwrap();
    let MediaPacket::Rtp(p) = inbox.receive(&key, options()).await.unwrap() else {
        panic!("expected RTP")
    };
    assert_eq!(p.header().timestamp, 9000);
    assert_eq!(p.video_epoch(), Some(&first.history));
    // Reselection keeps handle and SSRC. Its packet must not use the old notice.
    inbox.push(packet(Some(12), 0, 12000, 83)).unwrap();
    tokio::select! {biased;
        _=inbox.receive(&key,options())=>panic!("new tag inherited old history"),
        _=tokio::time::sleep(Duration::from_millis(20))=>{}
    }
    let next = notice(12, 2, 0, 12000);
    inbox.install_epoch(next.clone()).unwrap();
    let MediaPacket::Rtp(p) = inbox.receive(&key, options()).await.unwrap() else {
        panic!("expected new epoch")
    };
    assert_eq!(p.video_epoch(), Some(&next.history));
    inbox.install_epoch(first).unwrap(); // stale signed revision cannot restore old history
    inbox.push(packet(Some(12), 1, 15000, 83)).unwrap();
    let MediaPacket::Rtp(p) = inbox.receive(&key, options()).await.unwrap() else {
        panic!("expected current epoch")
    };
    assert_eq!(p.video_epoch(), Some(&next.history));
    // A late notice never refreshes packet age, including while no reader runs.
    inbox.push(packet(Some(13), 2, 18000, 83)).unwrap();
    tokio::time::advance(Duration::from_millis(199)).await;
    inbox.install_epoch(notice(13, 3, 2, 18000)).unwrap();
    tokio::time::advance(Duration::from_millis(2)).await;
    assert!(matches!(
        inbox.receive(&key, options()).await.unwrap(),
        MediaPacket::Gap { dropped_packets: 1 }
    ));
    // No notice at all still expires on time, without requiring new traffic.
    inbox.push(packet(Some(14), 3, 21000, 83)).unwrap();
    let began = tokio::time::Instant::now();
    assert!(matches!(
        inbox.receive(&key, options()).await.unwrap(),
        MediaPacket::Gap { dropped_packets: 1 }
    ));
    assert_eq!(began.elapsed(), Duration::from_millis(200));
    // Bare RTP cannot bypass the selected epoch format.
    inbox.push(packet(None, 4, 24000, 83)).unwrap();
    assert_eq!(inbox.stats().unwrap().malformed_datagrams, 1);
    assert_eq!(inbox.stats().unwrap().queued_packets, 0);
    // A local hold invalidates active history without forgetting its revision.
    inbox
        .apply_directions(&[(key.clone(), Direction::Inactive)])
        .unwrap();
    inbox
        .apply_directions(&[(key.clone(), Direction::Recvonly)])
        .unwrap();
    inbox.install_epoch(notice(13, 3, 2, 18000)).unwrap();
    inbox.push(packet(Some(13), 5, 27000, 83)).unwrap();
    assert!(matches!(
        inbox.receive(&key, options()).await.unwrap(),
        MediaPacket::Gap { dropped_packets: 1 }
    ));
    inbox.install_epoch(notice(15, 4, 6, 30000)).unwrap();
    inbox.push(packet(Some(15), 6, 30000, 83)).unwrap();
    assert!(
        matches!(inbox.receive(&key,options()).await.unwrap(),MediaPacket::Rtp(p) if p.video_epoch().unwrap().revision==4)
    );
    let plain = media::Inbox::default();
    let mut plain_selected = selected();
    plain_selected.video_feedback.clear();
    install(&plain, &plain_selected);
    plain.push(packet(Some(15), 6, 30000, 83)).unwrap();
    assert_eq!(plain.stats().unwrap().malformed_datagrams, 1);
    assert_eq!(plain.stats().unwrap().queued_packets, 0);
    inbox.end();
    assert_eq!(inbox.stats().unwrap().queued_bytes, 0);
}
