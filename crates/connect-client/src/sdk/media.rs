//! One bounded native receive inbox; slow media readers cannot block control.
use super::streams::StreamKey;
use super::*;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};
use tokio::sync::Notify;
pub(super) const SEND_CAPACITY: usize = 64;
const STREAM_PACKETS: usize = 32;
const TOTAL_PACKETS: usize = 256;
const TOTAL_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaStats {
    pub received_datagrams: u64,
    pub rtp_packets: u64,
    pub clock_reports: u64,
    pub malformed_datagrams: u64,
    pub unknown_handles: u64,
    pub overflow_drops: u64,
    pub queued_packets: usize,
    pub queued_bytes: usize,
}
pub(super) struct Binding {
    pub stream: NegotiatedStream,
    pub clock: bool,
}
struct Queue {
    binding: Binding,
    items: VecDeque<(MediaPacket, usize)>,
    gap: u64,
}
#[derive(Default)]
struct State {
    queues: BTreeMap<StreamKey, Queue>,
    handles: BTreeMap<u16, StreamKey>,
    bytes: usize,
    packets: usize,
    ended: bool,
    stats: MediaStats,
}
#[derive(Default)]
pub(super) struct Inbox {
    state: Mutex<State>,
    changed: Notify,
}
impl Inbox {
    pub fn install(
        &self,
        cid: &ConversationId,
        sid: &SessionId,
        conn: &ConnectionId,
        selected: &NativeNegotiation,
    ) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        for stream in &selected.streams {
            let Some(handle) = stream.stream_local_id else {
                continue;
            };
            if s.handles.contains_key(&handle) {
                return Err(Error::local(ErrorKind::Protocol));
            }
            let key = StreamKey {
                conversation_id: cid.clone(),
                session_id: sid.clone(),
                connection_id: conn.clone(),
                stream_id: stream.id.clone(),
            };
            s.handles.insert(handle, key.clone());
            s.queues.insert(
                key,
                Queue {
                    binding: Binding {
                        stream: stream.clone(),
                        clock: selected.source_clock,
                    },
                    items: VecDeque::new(),
                    gap: 0,
                },
            );
        }
        Ok(())
    }
    pub fn stats(&self) -> Result<MediaStats> {
        let s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        Ok(MediaStats {
            queued_packets: s.packets,
            queued_bytes: s.bytes,
            ..s.stats
        })
    }
    pub fn malformed(&self) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        s.stats.received_datagrams = s.stats.received_datagrams.saturating_add(1);
        s.stats.malformed_datagrams = s.stats.malformed_datagrams.saturating_add(1);
        Ok(())
    }
    pub fn push(&self, bytes: Vec<u8>) -> Result<()> {
        use crate::canonical::{
            observe_native_media_datagram, ObservedNativeMediaDatagram as Native,
        };
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        if s.ended {
            return Ok(());
        }
        s.stats.received_datagrams = s.stats.received_datagrams.saturating_add(1);
        let parsed = match observe_native_media_datagram(&bytes) {
            Ok(value) => value,
            Err(_) => {
                s.stats.malformed_datagrams = s.stats.malformed_datagrams.saturating_add(1);
                return Ok(());
            }
        };
        let handle = match &parsed {
            Native::Rtp(p) => p.stream_local_id,
            Native::Clock(p) => p.stream_local_id,
        };
        let Some(key) = s.handles.get(&handle).cloned() else {
            s.stats.unknown_handles = s.stats.unknown_handles.saturating_add(1);
            return Ok(());
        };
        let queue = s
            .queues
            .get(&key)
            .ok_or_else(|| Error::local(ErrorKind::Protocol))?;
        let receiving = matches!(
            queue.binding.stream.direction,
            Direction::Recvonly | Direction::Sendrecv
        );
        let packet = match parsed {
            Native::Rtp(p)
                if receiving
                    && p.flags == 0
                    && Some(u64::from(p.rtp.packet().header.payload_type))
                        == queue.binding.stream.codec["params"]["payload_type"].as_u64() =>
            {
                let packet = RtpPacket::new(p.seq, bytes[8..].to_vec())
                    .map_err(|_| Error::local(ErrorKind::Protocol))?;
                s.stats.rtp_packets = s.stats.rtp_packets.saturating_add(1);
                MediaPacket::Rtp(packet)
            }
            Native::Clock(p) if receiving && queue.binding.clock => {
                s.stats.clock_reports = s.stats.clock_reports.saturating_add(1);
                MediaPacket::Clock {
                    datagram_sequence: p.seq,
                    report: p.report,
                }
            }
            _ => {
                s.stats.malformed_datagrams = s.stats.malformed_datagrams.saturating_add(1);
                return Ok(());
            }
        };
        let full = s.packets >= TOTAL_PACKETS || s.bytes.saturating_add(bytes.len()) > TOTAL_BYTES;
        let queue = s
            .queues
            .get_mut(&key)
            .ok_or_else(|| Error::local(ErrorKind::Protocol))?;
        if full || queue.items.len() >= STREAM_PACKETS || queue.gap > 0 {
            queue.gap = queue.gap.saturating_add(1);
            s.stats.overflow_drops = s.stats.overflow_drops.saturating_add(1);
        } else {
            queue.items.push_back((packet, bytes.len()));
            s.packets += 1;
            s.bytes += bytes.len();
        }
        drop(s);
        self.changed.notify_waiters();
        Ok(())
    }
    pub fn retire(
        &self,
        cid: &ConversationId,
        sid: &SessionId,
        connection: &ConnectionId,
    ) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::local(ErrorKind::Protocol))?;
        let keys: Vec<_> = s
            .queues
            .keys()
            .filter(|k| {
                &k.conversation_id == cid && &k.session_id == sid && &k.connection_id == connection
            })
            .cloned()
            .collect();
        for key in keys {
            if let Some(queue) = s.queues.remove(&key) {
                if let Some(handle) = queue.binding.stream.stream_local_id {
                    s.handles.remove(&handle);
                }
                s.packets = s.packets.saturating_sub(queue.items.len());
                let bytes: usize = queue.items.iter().map(|(_, bytes)| *bytes).sum();
                s.bytes = s.bytes.saturating_sub(bytes);
            }
        }
        drop(s);
        self.changed.notify_waiters();
        Ok(())
    }
    pub fn end(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.ended = true;
            s.queues.clear();
            s.handles.clear();
            s.packets = 0;
            s.bytes = 0;
        }
        self.changed.notify_waiters();
    }
    pub async fn receive(&self, key: &StreamKey, options: RequestOptions) -> Result<MediaPacket> {
        options.validate()?;
        let until = tokio::time::Instant::now() + options.timeout;
        loop {
            if options.signal.is_cancelled() {
                return Err(Error::request(ErrorKind::Cancelled, None, false));
            }
            if tokio::time::Instant::now() >= until {
                return Err(Error::request(ErrorKind::Deadline, None, false));
            }
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut s = self
                    .state
                    .lock()
                    .map_err(|_| Error::local(ErrorKind::Protocol))?;
                if s.ended {
                    return Err(Error::request(ErrorKind::Cancelled, None, false));
                }
                let queue = s
                    .queues
                    .get_mut(key)
                    .ok_or_else(|| Error::request(ErrorKind::Unsupported, None, false))?;
                if let Some((packet, bytes)) = queue.items.pop_front() {
                    s.bytes = s.bytes.saturating_sub(bytes);
                    s.packets = s.packets.saturating_sub(1);
                    return Ok(packet);
                }
                if queue.gap > 0 {
                    let dropped_packets = std::mem::take(&mut queue.gap);
                    return Ok(MediaPacket::Gap { dropped_packets });
                }
            }
            tokio::select! {biased;
                _=options.signal.cancelled()=>return Err(Error::request(ErrorKind::Cancelled,None,false)),
                _=tokio::time::sleep_until(until)=>return Err(Error::request(ErrorKind::Deadline,None,false)),
                _=changed=>{}
            }
        }
    }
}
pub(super) fn encode(binding: &Binding, packet: &MediaPacket) -> Result<(Vec<u8>, u32)> {
    let fail = |kind| Error::media(kind, false);
    if !matches!(
        binding.stream.direction,
        Direction::Sendonly | Direction::Sendrecv
    ) {
        return Err(fail(ErrorKind::Validation));
    }
    let handle = binding
        .stream
        .stream_local_id
        .ok_or_else(|| fail(ErrorKind::Unsupported))?;
    match packet {
        MediaPacket::Rtp(packet) => {
            if Some(u64::from(packet.header().payload_type))
                != binding.stream.codec["params"]["payload_type"].as_u64()
            {
                return Err(fail(ErrorKind::Validation));
            }
            Ok((
                streams::frame(handle, packet.datagram_sequence, packet.bytes()),
                packet.datagram_sequence,
            ))
        }
        MediaPacket::Clock {
            datagram_sequence,
            report,
        } if binding.clock => {
            if report.cname.len() > 255 {
                return Err(fail(ErrorKind::Validation));
            }
            let bytes = crate::canonical::pack_sender_clock_datagram(
                &crate::canonical::SenderClockDatagram {
                    stream_local_id: handle,
                    seq: *datagram_sequence,
                    report: report.clone(),
                },
            )
            .map_err(|_| fail(ErrorKind::Validation))?;
            Ok((bytes.to_vec(), *datagram_sequence))
        }
        MediaPacket::Clock { .. } => Err(fail(ErrorKind::Unsupported)),
        MediaPacket::Gap { .. } => Err(fail(ErrorKind::Validation)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn key(index: usize) -> StreamKey {
        StreamKey {
            conversation_id: "room".parse().unwrap(),
            session_id: "session".parse().unwrap(),
            connection_id: "connection".parse().unwrap(),
            stream_id: format!("stream-{index}").parse().unwrap(),
        }
    }
    fn inbox(streams: usize) -> Inbox {
        let inbox = Inbox::default();
        let selected=NativeNegotiation{source_clock:true,stream_revision:StreamRevision::try_from(1).unwrap(),streams:(0..streams).map(|i|NegotiatedStream{id:key(i).stream_id,medium:Medium::Audio,direction:Direction::Sendrecv,codec:json!({"name":"pcmu","params":{"payload_type":0,"sample_rate":8000,"channels":1}}),stream_local_id:Some(i as u16+1)}).collect()};
        let root = key(0);
        inbox
            .install(
                &root.conversation_id,
                &root.session_id,
                &root.connection_id,
                &selected,
            )
            .unwrap();
        inbox
    }
    fn wire(handle: u16, sequence: u32, payload_bytes: usize) -> Vec<u8> {
        let mut rtp = vec![0x80, 0];
        rtp.extend_from_slice(&(sequence as u16).to_be_bytes());
        rtp.extend_from_slice(&sequence.wrapping_mul(160).to_be_bytes());
        rtp.extend_from_slice(&1u32.to_be_bytes());
        rtp.resize(rtp.len() + payload_bytes, 0x7f);
        streams::frame(handle, sequence, &rtp)
    }
    fn options() -> RequestOptions {
        RequestOptions::new(Duration::from_secs(1)).unwrap()
    }
    #[test]
    fn rtp_constructor_preserves_marker_csrc_extension_padding_and_payload_identity() {
        let mut raw = vec![0xb1, 0xe0, 0x12, 0x34, 0x01, 0x02, 0x03, 0x04, 0, 0, 0, 7];
        raw.extend_from_slice(&9u32.to_be_bytes());
        raw.extend_from_slice(&[0xbe, 0xde, 0, 1, 0x10, 0xab, 0, 0]);
        raw.extend_from_slice(&[0x10, 0xaa, 0xbb]);
        raw.extend_from_slice(&[0, 0, 0, 4]);
        let packet = RtpPacket::new(99, raw.clone()).unwrap();
        assert_eq!(packet.bytes(), raw);
        assert_eq!(packet.payload(), [0x10, 0xaa, 0xbb]);
        assert!(packet.header().marker);
        assert_eq!(packet.header().payload_type, 96);
        assert_eq!(packet.header().sequence_number, 0x1234);
        assert_eq!(packet.datagram_sequence, 99);
        let binding = Binding {
            clock: true,
            stream: NegotiatedStream {
                id: key(0).stream_id,
                medium: Medium::Video,
                direction: Direction::Sendonly,
                codec: json!({"params":{"payload_type":96}}),
                stream_local_id: Some(17),
            },
        };
        let (sent, seq) = encode(&binding, &MediaPacket::Rtp(packet)).unwrap();
        assert_eq!(seq, 99);
        assert_eq!(&sent[8..], raw);
        assert_eq!(&sent[2..4], &17u16.to_be_bytes());
        assert!(RtpPacket::new(1, vec![0; 20]).is_err());
    }
    #[tokio::test]
    async fn slow_stream_observes_ordered_local_gap_and_recovers_without_losing_other_queue_or_cancelled_read(
    ) {
        let inbox = inbox(2);
        for sequence in 0..40 {
            inbox.push(wire(1, sequence, 160)).unwrap();
        }
        inbox.push(wire(2, 55, 160)).unwrap();
        let cancelled = options();
        cancelled.signal.cancel();
        assert!(
            matches!(inbox.receive(&key(0),cancelled).await,Err(e) if e.kind==ErrorKind::Cancelled)
        );
        let stats = inbox.stats().unwrap();
        assert_eq!(stats.queued_packets, 33);
        assert_eq!(stats.overflow_drops, 8);
        for sequence in 0..32 {
            let MediaPacket::Rtp(packet) = inbox.receive(&key(0), options()).await.unwrap() else {
                panic!("older RTP must precede gap")
            };
            assert_eq!(packet.datagram_sequence, sequence);
        }
        assert!(matches!(
            inbox.receive(&key(0), options()).await.unwrap(),
            MediaPacket::Gap { dropped_packets: 8 }
        ));
        inbox.push(wire(1, 90, 160)).unwrap();
        assert!(
            matches!(inbox.receive(&key(0),options()).await.unwrap(),MediaPacket::Rtp(p) if p.datagram_sequence==90)
        );
        assert!(
            matches!(inbox.receive(&key(1),options()).await.unwrap(),MediaPacket::Rtp(p) if p.datagram_sequence==55)
        );
        inbox.push(wire(1, 91, 160)).unwrap();
        inbox.end();
        assert_eq!(inbox.stats().unwrap().queued_bytes, 0);
        assert!(
            matches!(inbox.receive(&key(0),options()).await,Err(e) if e.kind==ErrorKind::Cancelled)
        );
    }
    #[tokio::test]
    async fn aggregate_packet_and_byte_caps_apply_across_independent_streams() {
        for payload in [160, 16 * 1024] {
            let inbox = inbox(9);
            let size = wire(1, 0, payload).len();
            let admitted = TOTAL_PACKETS.min(TOTAL_BYTES / size);
            for index in 0..256 {
                inbox
                    .push(wire((index / 32 + 1) as u16, index as u32, payload))
                    .unwrap();
            }
            inbox.push(wire(9, 300, payload)).unwrap();
            let stats = inbox.stats().unwrap();
            assert_eq!(stats.queued_packets, admitted);
            assert!(stats.queued_bytes <= TOTAL_BYTES);
            assert_eq!(stats.overflow_drops, 257 - admitted as u64);
            assert!(matches!(
                inbox.receive(&key(8), options()).await.unwrap(),
                MediaPacket::Gap { dropped_packets: 1 }
            ));
            assert!(
                matches!(inbox.receive(&key(0),options()).await.unwrap(),MediaPacket::Rtp(p) if p.datagram_sequence==0)
            );
        }
    }
    #[tokio::test]
    async fn clock_metadata_wrong_payload_type_and_foreign_handle_stay_distinct() {
        let inbox = inbox(1);
        let mut wrong = wire(1, 1, 160);
        wrong[9] = 96;
        inbox.push(wrong).unwrap();
        inbox.push(wire(7, 2, 160)).unwrap();
        let report = SenderClockReport {
            ssrc: 42,
            ntp: 0x80000000ffffffff,
            rtp: 160,
            packets: 1,
            octets: 160,
            cname: "actual-source".into(),
        };
        inbox
            .push(
                crate::canonical::pack_sender_clock_datagram(
                    &crate::canonical::SenderClockDatagram {
                        stream_local_id: 1,
                        seq: 3,
                        report: report.clone(),
                    },
                )
                .unwrap()
                .to_vec(),
            )
            .unwrap();
        assert!(
            matches!(inbox.receive(&key(0),options()).await.unwrap(),MediaPacket::Clock{datagram_sequence:3,report:r} if r==report)
        );
        let stats = inbox.stats().unwrap();
        assert_eq!(stats.malformed_datagrams, 1);
        assert_eq!(stats.unknown_handles, 1);
        assert_eq!(stats.rtp_packets, 0);
        assert_eq!(stats.clock_reports, 1);
    }
    #[tokio::test]
    async fn retiring_one_connection_releases_queued_bytes_without_consuming_another_connection() {
        let inbox = inbox(2);
        let mut other = key(0);
        other.connection_id = "other-connection".parse().unwrap();
        let selected = NativeNegotiation {
            source_clock: false,
            stream_revision: StreamRevision::try_from(1).unwrap(),
            streams: vec![NegotiatedStream {
                id: other.stream_id.clone(),
                medium: Medium::Audio,
                direction: Direction::Recvonly,
                codec: json!({"name":"pcmu","params":{"payload_type":0}}),
                stream_local_id: Some(3),
            }],
        };
        inbox
            .install(
                &other.conversation_id,
                &other.session_id,
                &other.connection_id,
                &selected,
            )
            .unwrap();
        inbox.push(wire(1, 1, 160)).unwrap();
        inbox.push(wire(2, 2, 160)).unwrap();
        inbox.push(wire(3, 3, 160)).unwrap();
        let root = key(0);
        inbox
            .retire(&root.conversation_id, &root.session_id, &root.connection_id)
            .unwrap();
        assert_eq!(inbox.stats().unwrap().queued_packets, 1);
        assert_eq!(inbox.stats().unwrap().queued_bytes, wire(3, 3, 160).len());
        inbox
            .retire(&root.conversation_id, &root.session_id, &root.connection_id)
            .unwrap();
        inbox.push(wire(1, 4, 160)).unwrap();
        assert_eq!(inbox.stats().unwrap().unknown_handles, 1);
        assert!(
            matches!(inbox.receive(&root,options()).await,Err(e) if e.kind==ErrorKind::Unsupported)
        );
        assert!(
            matches!(inbox.receive(&other,options()).await.unwrap(),MediaPacket::Rtp(p) if p.datagram_sequence==3)
        );
        assert_eq!(inbox.stats().unwrap().queued_packets, 0);
        assert_eq!(inbox.stats().unwrap().queued_bytes, 0);
        inbox.push(wire(3, 5, 160)).unwrap();
        assert!(
            matches!(inbox.receive(&other,options()).await.unwrap(),MediaPacket::Rtp(p) if p.datagram_sequence==5)
        );
    }
    #[tokio::test]
    async fn retiring_connection_wakes_its_pending_packet_reader() {
        let inbox = inbox(1);
        let root = key(0);
        let receive = inbox.receive(&root, options());
        tokio::pin!(receive);
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut receive)
            .await
            .is_err());
        inbox
            .retire(&root.conversation_id, &root.session_id, &root.connection_id)
            .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(100), receive)
            .await
            .expect("retirement wakes pending reader");
        assert!(matches!(result,Err(e) if e.kind==ErrorKind::Unsupported));
    }
}
