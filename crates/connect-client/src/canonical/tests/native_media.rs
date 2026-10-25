//! Controlled post-auth physical datagrams; no service admission/negotiation claim.
use super::*;
#[derive(Clone, Copy)]
pub(crate) enum Traffic {
    ClockThenPcm,
    ForeignClock,
    ClockOnly,
}
pub(super) async fn emit(traffic: Traffic, connection: &quinn::Connection, ready: &Notify) {
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
