use super::*;
use crate::sdk::video_feedback::epochs::tests::{notice, notice_value, selected};
use serde_json::json;
fn options(ms: u64) -> RequestOptions {
    RequestOptions::new(Duration::from_millis(ms)).unwrap()
}
fn publisher_notice(tag: u128, revision: u64) -> Notice {
    let mut raw = notice_value(tag, revision, 100, 3000);
    raw["payload"]["direction"] = json!("publisher");
    Notice::read(&crate::canonical::Envelope::from_value(raw).unwrap()).unwrap()
}
fn inbox(publisher: bool) -> (Inbox, StreamKey, Binding) {
    let mut selected = selected();
    if publisher {
        selected.streams[0].direction = Direction::Sendonly;
    }
    let key = notice(11, 1, 100, 3000).key;
    let binding = Binding {
        stream: selected.streams[0].clone(),
        clock: true,
    };
    let inbox = Inbox::default();
    inbox
        .install(
            &key.conversation_id,
            &key.session_id,
            &key.connection_id,
            &selected,
        )
        .unwrap();
    (inbox, key, binding)
}
fn pli(tag: u128, sequence: u32, ssrc: u32) -> Vec<u8> {
    pack_video_feedback_datagram(&VideoFeedbackDatagram {
        stream_local_id: 7,
        seq: sequence,
        sender_ssrc: 1,
        media_ssrc: ssrc,
        activation_epoch: tag,
    })
    .unwrap()
    .to_vec()
}

#[tokio::test(start_paused = true)]
async fn publisher_feedback_is_bounded_current_and_never_grants_media_receive_direction() {
    let (inbox, key, _) = inbox(true);
    inbox.push(pli(11, u32::MAX, 83)).unwrap();
    tokio::select! {biased;
        _=inbox.receive_feedback(&key,options(1000))=>panic!("feedback admitted without notice"),
        _=tokio::time::sleep(Duration::from_millis(50))=>{},
    }
    inbox.install_epoch(publisher_notice(11, 1)).unwrap();
    let original = inbox.receive_feedback(&key, options(1000)).await.unwrap();
    assert_eq!(original.expires_in(), Duration::from_millis(150));
    assert!(inbox.feedback_is_current(&key, &original).unwrap());
    assert_eq!(original.ssrc(), 83);
    assert_eq!(original.datagram_sequence(), u32::MAX);
    assert_eq!(
        inbox.receive(&key, options(10)).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    // Wrap is valid; duplicates and late older serials cannot renew custody.
    inbox.push(pli(11, 0, 83)).unwrap();
    let wrapped = inbox.receive_feedback(&key, options(10)).await.unwrap();
    assert_eq!(wrapped.datagram_sequence(), 0);
    for sequence in [0, u32::MAX] {
        inbox.push(pli(11, sequence, 83)).unwrap();
    }
    assert_eq!(
        inbox
            .receive_feedback(&key, options(2))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    // The actual source remains the same but only the newer notice can join
    // feedback that arrived for a future source epoch.
    inbox.push(pli(12, 1, 83)).unwrap();
    tokio::time::advance(Duration::from_millis(199)).await;
    inbox.install_epoch(publisher_notice(12, 2)).unwrap();
    tokio::time::advance(Duration::from_millis(2)).await;
    assert_eq!(
        inbox
            .receive_feedback(&key, options(2))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    assert!(!inbox.feedback_is_current(&key, &original).unwrap());
    inbox.push(pli(12, 1, 83)).unwrap(); // expired duplicate cannot restart the age
    assert_eq!(
        inbox
            .receive_feedback(&key, options(2))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    inbox.push(pli(12, 2, 84)).unwrap(); // right epoch but wrong actual SSRC
    assert_eq!(
        inbox
            .receive_feedback(&key, options(201))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    // Multiple current requests consume one slot; only the freshest hint remains.
    inbox.push(pli(12, 3, 83)).unwrap();
    inbox.push(pli(12, 4, 83)).unwrap();
    let current = inbox.receive_feedback(&key, options(10)).await.unwrap();
    assert_eq!(current.datagram_sequence(), 4);
    let (other, _, _) = self::inbox(true);
    other.install_epoch(publisher_notice(12, 2)).unwrap();
    assert!(!other.feedback_is_current(&key, &current).unwrap());
    inbox
        .apply_directions(&[(key.clone(), Direction::Inactive)])
        .unwrap();
    assert!(!inbox.feedback_is_current(&key, &current).unwrap());
    inbox
        .apply_directions(&[(key.clone(), Direction::Sendonly)])
        .unwrap();
    inbox.install_epoch(publisher_notice(12, 2)).unwrap();
    inbox.push(pli(12, 5, 83)).unwrap();
    assert_eq!(
        inbox
            .receive_feedback(&key, options(201))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    inbox.install_epoch(publisher_notice(13, 3)).unwrap();
    inbox.push(pli(13, 6, 83)).unwrap();
    let current = inbox.receive_feedback(&key, options(10)).await.unwrap();
    assert!(inbox.feedback_is_current(&key, &current).unwrap());
    inbox
        .retire(&key.conversation_id, &key.session_id, &key.connection_id)
        .unwrap();
    assert!(!inbox.feedback_is_current(&key, &current).unwrap());
    assert!(inbox.stats().unwrap().feedback_drops >= 5);
    let (receiver, _, _) = self::inbox(false);
    receiver.push(pli(11, 0, 83)).unwrap();
    assert_eq!(receiver.stats().unwrap().control_drops, 1);
}

#[test]
fn queued_keyframe_request_cannot_retarget_or_retry_an_uncertain_submission() {
    let (inbox, key, binding) = inbox(false);
    assert!(inbox.keyframe_target(&key).is_err());
    inbox.install_epoch(notice(11, 1, 100, 3000)).unwrap();
    let old = inbox.keyframe_target(&key).unwrap();
    inbox.install_epoch(notice(12, 2, 101, 6000)).unwrap();
    assert_eq!(
        inbox
            .send_keyframe(&binding, &old, |_| panic!("stale request submitted"))
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let target = inbox.keyframe_target(&key).unwrap();
    let result = inbox
        .send_keyframe(&binding, &target, |bytes| {
            let parsed = observe_video_feedback_datagram(bytes).unwrap();
            assert_eq!(
                (
                    parsed.stream_local_id,
                    parsed.seq,
                    parsed.media_ssrc,
                    parsed.activation_epoch
                ),
                (7, 0, 83, 12)
            );
            assert_ne!(parsed.sender_ssrc, 0);
            Ok(())
        })
        .unwrap();
    assert_eq!(result.datagram_sequence, 0);
    assert_eq!(
        inbox
            .send_keyframe(&binding, &target, |_| Err(Error::media(
                ErrorKind::TransportUncertain,
                true
            )))
            .unwrap_err()
            .certainty,
        Certainty::Uncertain
    );
    let result = inbox
        .send_keyframe(&binding, &target, |bytes| {
            assert_eq!(observe_video_feedback_datagram(bytes).unwrap().seq, 2);
            Ok(())
        })
        .unwrap();
    assert_eq!(result.datagram_sequence, 2);
    let (other, _, _) = self::inbox(false);
    other.install_epoch(notice(12, 2, 101, 6000)).unwrap();
    assert_eq!(
        other
            .send_keyframe(&binding, &target, |_| panic!(
                "cross-client target submitted"
            ))
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    inbox
        .apply_directions(&[(key.clone(), Direction::Inactive)])
        .unwrap();
    assert!(inbox.keyframe_target(&key).is_err());
    assert!(inbox
        .send_keyframe(&binding, &target, |_| panic!("held target submitted"))
        .is_err());
    inbox.end();
    assert!(inbox
        .send_keyframe(&binding, &target, |_| panic!("ended target submitted"))
        .is_err());
}
