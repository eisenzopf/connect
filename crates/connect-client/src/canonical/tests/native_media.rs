//! Controlled post-auth physical datagrams; no service admission/negotiation claim.
use super::*;
#[derive(Clone, Copy)]
pub(crate) enum Traffic {
    VideoFeedback,
    ClockThenPcm,
    ForeignClock,
    ClockOnly,
    QueuePressure,
}
pub(super) async fn emit(traffic: Traffic, connection: &quinn::Connection, ready: &Notify) {
    if matches!(traffic, Traffic::QueuePressure) {
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut observed = Vec::new();
        loop {
            let quiet = if observed.is_empty() {
                until
            } else {
                until.min(tokio::time::Instant::now() + Duration::from_millis(50))
            };
            match tokio::time::timeout_at(quiet, connection.read_datagram()).await {
                Ok(Ok(bytes)) => observed.push(bytes.to_vec()),
                Ok(Err(_)) => return,
                Err(_) => break,
            }
        }
        let audio = observed
            .iter()
            .filter(|b| b.get(9) == Some(&111))
            .cloned()
            .collect::<Vec<_>>();
        let summary =
            serde_json::to_vec(&json!({"datagrams":observed.len(),"audio":audio})).unwrap();
        connection.send_datagram_wait(summary.into()).await.unwrap();
        ready.notify_one();
        return;
    }
    if matches!(traffic, Traffic::VideoFeedback) {
        let bytes = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
            .await
            .unwrap()
            .unwrap();
        let request = rvoip_uctp::substrate::observe_video_feedback_datagram(&bytes).unwrap();
        assert_eq!(
            (
                request.stream_local_id,
                request.seq,
                request.media_ssrc,
                request.activation_epoch
            ),
            (7, 77, 83, 11)
        );
        connection.send_datagram(bytes).unwrap();
        let mut rtp = vec![0x80, 96, 0, 100];
        rtp.extend_from_slice(&3000u32.to_be_bytes());
        rtp.extend_from_slice(&83u32.to_be_bytes());
        rtp.extend_from_slice(&[0x10, 0x31]);
        let mut native = vec![1, 0, 0, 7];
        native.extend_from_slice(&100u32.to_be_bytes());
        native.extend_from_slice(&rtp);
        let tagged = rvoip_uctp::substrate::tag_video_epoch_datagram(&native, 11).unwrap();
        connection.send_datagram(tagged).unwrap();
        ready.notify_one();
        return;
    }
    let wire = pack_sender_clock_datagram(&SenderClockDatagram {
        stream_local_id: if matches!(traffic, Traffic::ForeignClock) {
            8
        } else {
            7
        },
        seq: 0,
        report: SenderClockReport {
            ssrc: 41,
            ntp: 1,
            rtp: 0,
            packets: 1,
            octets: 160,
            cname: "controlled-clock".into(),
        },
    })
    .unwrap();
    if matches!(traffic, Traffic::ClockOnly) {
        let mut count = 0;
        loop {
            if connection.send_datagram(wire.clone()).is_err() {
                break;
            }
            count += 1;
            if count == 3 {
                ready.notify_one();
            }
            tokio::select! {_=connection.closed()=>break,_=tokio::time::sleep(Duration::from_millis(1))=>{}}
        }
    } else {
        connection.send_datagram(wire).unwrap();
        ready.notify_one();
        if matches!(traffic, Traffic::ForeignClock) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        let pcm = rvoip_uctp::substrate::pack_rtp_datagram(&rvoip_uctp::substrate::RtpDatagram {
            flags: 0,
            stream_local_id: 7,
            seq: 1,
            rtp: rvoip_uctp::substrate::RtpMediaPayload {
                payload: vec![0x7f; 160].into(),
                payload_type: 0,
                sequence_number: 1,
                timestamp: 0,
                ssrc: 41,
            },
        })
        .unwrap();
        connection.send_datagram(pcm).unwrap();
    }
}
