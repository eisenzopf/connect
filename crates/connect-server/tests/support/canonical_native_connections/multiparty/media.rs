//! Real per-peer native datagrams and independent G.711 decoding of egress.
use super::*;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};
fn packet(peer: &mut Peer, mark: bool) -> Vec<u8> {
    peer.sequence += 1;
    pack_rtp_datagram(&RtpDatagram {
        flags: 0,
        stream_local_id: peer.publish_handle,
        seq: peer.sequence,
        rtp: RtpMediaPayload {
            payload: (0..160)
                .map(|i| {
                    if mark {
                        if i % 16 < 8 {
                            0x80
                        } else {
                            0x00
                        }
                    } else {
                        0xff
                    }
                })
                .collect::<Vec<_>>()
                .into(),
            payload_type: 0,
            sequence_number: peer.sequence as u16,
            timestamp: (peer.clock.elapsed().as_micros() * 8 / 1000) as u32,
            ssrc: 0x50500001,
        },
    })
    .unwrap()
    .to_vec()
}
pub(super) async fn ready(peer: &mut Peer) {
    for index in 0..240 {
        let bytes = packet(peer, false);
        peer.client.send_native_media_datagram(&bytes).unwrap();
        if index % 20 == 19 {
            // A real application services the reliable stream while publishing.
            // Preserve every verified hint; do not leave its actual writer
            // blocked behind an unread multicast window during media startup.
            drain(peer).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
pub(super) async fn drain(peer: &mut Peer) {
    let id = peer.client.keepalive().await.unwrap();
    loop {
        let frame = peer.client.receive().await.unwrap();
        if frame.kind() == "ack" {
            assert_eq!(frame.original()["in_reply_to"], id);
            break;
        }
        peer.events.push_back(frame);
    }
}
fn pcmu(byte: u8) -> i32 {
    let u = !byte;
    let magnitude = ((i32::from(u & 15) << 3) + 132) << ((u & 0x70) >> 4);
    let sample = magnitude - 132;
    if u & 0x80 != 0 {
        -sample
    } else {
        sample
    }
}
pub(super) async fn probe(
    peers: &mut [Peer],
    source: usize,
    allowed: &BTreeSet<usize>,
    name: &str,
) -> Value {
    enum Event {
        Tick,
        Packet(usize, connect_client::Result<Vec<u8>>),
    }
    let start = tokio::time::Instant::now();
    let send_until = start + Duration::from_secs(2);
    let until = send_until + Duration::from_millis(300);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut counts = vec![0usize; peers.len()];
    let mut received = vec![0usize; peers.len()];
    let mut observed = vec![BTreeSet::new(); peers.len()];
    while tokio::time::Instant::now() < until {
        let event = {
            let receives = peers
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| p.readable)
                .map(|(i, p)| {
                    Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
                })
                .collect::<Vec<_>>();
            tokio::select! {_=tick.tick()=>Event::Tick,((i,packet),_,_)=select_all(receives)=>Event::Packet(i,packet)}
        };
        match event {
            Event::Tick if tokio::time::Instant::now() < send_until => {
                let bytes = packet(&mut peers[source], true);
                peers[source]
                    .client
                    .send_native_media_datagram(&bytes)
                    .unwrap();
            }
            Event::Packet(i, Ok(bytes)) => {
                received[i] += 1;
                let p = unpack_rtp_datagram(&bytes).unwrap();
                assert_eq!(
                    p.stream_local_id, peers[i].handle,
                    "{name}: wrong physical peer binding at{i}"
                );
                assert_eq!(p.rtp.payload_type, 0);
                observed[i].insert(p.stream_local_id);
                if start.elapsed() > Duration::from_millis(300)
                    && p.rtp.payload.iter().any(|b| pcmu(*b).abs() > 4000)
                {
                    assert!(
                        allowed.contains(&i),
                        "{name}: publisher{source} reached unsubscribed/retired peer{i}"
                    );
                    counts[i] += 1;
                }
            }
            Event::Packet(i, Err(error)) => {
                assert!(!peers[i].active, "{name}: active peer{i} closed: {error:?}");
                peers[i].readable = false;
            }
            _ => {}
        }
    }
    for i in allowed {
        assert!(
            counts[*i] > 10,
            "{name}: subscribed peer{i} received too few decoded frames: {counts:?}; received_packets={received:?}"
        );
    }
    eprintln!("fifty-participant media {name}: decoded marked counts={counts:?}");
    json!({"name":name,"source_index":source,"allowed":allowed,"decoded_marked":counts,"received_packets":received,"egress_handles":observed})
}

/// First actual delivery establishes receiver readiness. Require decoded wire
/// media and durable flow facts before measuring the unchanged steady window.
pub(super) async fn startup(
    peers: &mut [Peer],
    source: usize,
    allowed: &BTreeSet<usize>,
    db: &rusqlite::Connection,
    connections: &[String],
    name: &str,
    end: tokio::time::Instant,
) -> Value {
    enum Event {
        Tick,
        Packet(usize, connect_client::Result<Vec<u8>>),
    }
    let start = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut counts = vec![0usize; peers.len()];
    let mut flow_facts = 0usize;
    let mut check_at = start;
    let ids = serde_json::to_string(connections).unwrap();
    while tokio::time::Instant::now() < end {
        if tokio::time::Instant::now() >= check_at {
            flow_facts = db.query_row(
                "SELECT count(*) FROM native_connection_flow_facts WHERE connection_id IN (SELECT value FROM json_each(?1))",
                [&ids], |r| r.get(0),
            ).unwrap();
            if flow_facts == connections.len() && allowed.iter().all(|i| counts[*i] > 0) {
                break;
            }
            check_at = tokio::time::Instant::now() + Duration::from_millis(100);
        }
        let event = {
            let receives = peers
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| p.readable)
                .map(|(i, p)| {
                    Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
                })
                .collect::<Vec<_>>();
            tokio::select! { _=tick.tick()=>Event::Tick,((i,packet),_,_)=select_all(receives)=>Event::Packet(i,packet) }
        };
        match event {
            Event::Tick => {
                let bytes = packet(&mut peers[source], true);
                peers[source]
                    .client
                    .send_native_media_datagram(&bytes)
                    .unwrap();
            }
            Event::Packet(i, Ok(bytes)) => {
                let p = unpack_rtp_datagram(&bytes).unwrap();
                assert_eq!(
                    p.stream_local_id, peers[i].handle,
                    "{name}: wrong physical binding at{i}"
                );
                assert_eq!(p.rtp.payload_type, 0);
                // The prior phase drains its tail; this also excludes any
                // remaining old-source packet from the new startup witness.
                if start.elapsed() > Duration::from_millis(300)
                    && p.rtp.payload.iter().any(|b| pcmu(*b).abs() > 4000)
                {
                    assert!(allowed.contains(&i), "{name}: reached excluded peer{i}");
                    counts[i] += 1;
                }
            }
            Event::Packet(i, Err(error)) => {
                panic!("{name}: peer{i} closed during actual startup: {error:?}")
            }
        }
    }
    assert_eq!(
        flow_facts,
        connections.len(),
        "{name}: missing actual flow facts"
    );
    assert!(
        allowed.iter().all(|i| counts[*i] > 0),
        "{name}: missing decoded first delivery: {counts:?}"
    );
    assert!(
        tokio::time::Instant::now() < end,
        "{name}: exceeded original eight-second readiness bound"
    );
    eprintln!(
        "fifty-participant startup {name}: {}ms, flow_facts={flow_facts}, decoded={counts:?}",
        start.elapsed().as_millis()
    );
    json!({"name":name,"source_index":source,"allowed":allowed,"decoded_marked":counts,"flow_connection_ids":connections,"flow_facts":flow_facts,"elapsed_ms":start.elapsed().as_millis() as u64,"deadline_ms":8000})
}

pub(super) async fn recover_ready(peer: &mut Peer, db: &rusqlite::Connection, f: &Fixture) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let seen: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_connection_flow_facts WHERE connection_id=?1)",
                [&peer.connection],
                |r| r.get(0),
            )
            .unwrap();
        if seen {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = f.server.service.canonical_runtime_snapshot().unwrap();
            let facts:String=db.query_row("SELECT json_group_array(json_object('id',c.id,'state',c.state,'generation',c.generation,'role',sp.role,'member_active',p.active,'kind',i.kind,'endpoint',ep.verification_state,'runtime',c.observed_runtime_id,'core_id',(SELECT runtime_connection_id FROM connection_offers WHERE connection_id=c.id))) FROM connections c JOIN session_participants sp ON sp.session_id=c.session_id AND sp.participant_id=c.participant_id JOIN participants p ON p.participant_id=c.participant_id JOIN identities i ON i.id=c.identity_id JOIN endpoints ep ON ep.id=c.endpoint_id WHERE c.id=?1",[&peer.connection],|r|r.get(0)).unwrap();
            eprintln!("unready publisher metadata: {facts}");
            panic!(
                "actual publisher {} never became ready during original-peer recovery",
                peer.connection
            );
        }
        let bytes = packet(peer, false);
        peer.client.send_native_media_datagram(&bytes).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
