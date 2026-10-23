//! Actual public native -> independently owned SIP -> new signed native movement.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_source_returns_to_signed_native_generation_three_without_redial() {
    telephone_case(Stop::ReturnNative, true).await;
}
pub(super) async fn run(
    f: &Fixture,
    cid: &str,
    peers: &mut Vec<Peer>,
    phone: &PeerSip,
    payload: u8,
    controller: &mut CanonicalHttpClient,
    key: &str,
    route: &str,
    first: &TransitionCommand,
    first_done: &OperationRecord,
    observer: bool,
) {
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let remote: (u64,String)=db.query_row("SELECT generation,runtime_connection_id FROM connections JOIN connection_offers ON connection_id=connections.id WHERE connections.id='codec-source'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let original_native = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    f.server.store.revoke(&admin, &original_native).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("stream.subscribe".into());
    let target = party_with_scope(f, &f.client.identity_id, 391, vec!["audio".into()], scope);
    let endpoint: String = db
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&target.device_id],
            |r| r.get(0),
        )
        .unwrap();
    let source = first_done.result.as_ref().unwrap()["connection_id"]
        .as_str()
        .unwrap();
    let command = TransitionCommand {
        operation_id: "telephone-return-native".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(source.into()),
        expected_revision: revision(f),
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            retained_connid: "codec-source".into(),
        },
        payload_extensions: Default::default(),
    };
    let root = controller.transition(command.clone()).await.unwrap();
    let prepared = wait_prepared(f, key, &command.operation_id).await;
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM native_replacements WHERE operation_record_id=?1",
            [&prepared.record_id],
            |r| r.get(0),
        )
        .unwrap();
    let plan: Value = serde_json::from_str(&plan).unwrap();
    assert_eq!(plan["generation"], 3);
    assert_eq!(plan["source_generation"], 2);
    let candidate = plan["candidate_connection_id"].as_str().unwrap().to_owned();
    let offer = json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"telephone-return-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":prepared.record_id}});
    let mut client = CanonicalClient::open(target.clone()).await.unwrap();
    let allocation = client
        .allocate_native_replacement_target(
            &command,
            &root,
            serde_json::from_value(offer.clone()).unwrap(),
        )
        .await;
    if let Err(error) = &allocation {
        eprintln!(
            "telephone return allocation refused: {error:?}; current {:?}",
            status(f, key, &command.operation_id).await
        );
    }
    allocation.unwrap();
    let (request, scope) = client
        .begin_native_replacement_media(&command, &root, serde_json::from_value(offer).unwrap())
        .await
        .unwrap();
    let answer = client.receive_control().await.unwrap();
    assert_eq!(
        answer.kind(),
        "connection.answer",
        "{:?}",
        answer.original()
    );
    assert_eq!(answer.original()["in_reply_to"], request);
    let native_handle = handle(&answer);
    let callback = client.receive_control().await.unwrap();
    assert_eq!(
        callback.kind(),
        "connection.offer",
        "{:?}",
        callback.original()
    );
    let p = &callback.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|v|json!({"id":v["strm_id"],"kind":v["kind"],"direction":match v["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":v["codec"]})).collect::<Vec<_>>();
    client.answer_connection_offer(&callback,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap(),&scope).await.unwrap();
    let staged_until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let staged = status(f, key, &command.operation_id).await;
        if staged.effects.len() == 5
            && staged
                .effects
                .iter()
                .all(|e| e.state == EffectState::Confirmed)
        {
            break;
        }
        assert_eq!(
            staged.state,
            OperationState::Preparing,
            "telephone return private staging: {staged:?}"
        );
        assert!(
            tokio::time::Instant::now() < staged_until,
            "telephone return private staging did not settle: {staged:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // While the new device has no actual input, the established phone keeps
    // both real directions; an answered native offer is not movement readiness.
    assert_eq!(payload, 0);
    old_duplex(phone, &mut peers[0], &mut client, native_handle, observer).await;
    let waiting = status(f, key, &command.operation_id).await;
    assert_eq!(
        waiting.state,
        OperationState::Preparing,
        "telephone return after resource start: {waiting:?}"
    );
    let participant = peers[1].participant.clone();
    let candidate_index = peers.len();
    peers.push(Peer {
        client,
        config: Some(target),
        identity: f.client.identity_id.clone(),
        participant: participant.clone(),
        id: candidate.clone(),
        handle: native_handle,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    });
    send_readiness(&mut peers[candidate_index]).await;
    let end = tokio::time::Instant::now() + Duration::from_secs(10);
    let done = loop {
        let state = status(f, key, &command.operation_id).await;
        if state.state == OperationState::Completed {
            break state;
        }
        assert!(
            !matches!(
                state.state,
                OperationState::Failed | OperationState::Unknown | OperationState::Cancelled
            ),
            "telephone return stopped: {state:?}"
        );
        assert!(
            tokio::time::Instant::now() < end,
            "telephone return did not complete: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(done.result.as_ref().unwrap()["generation"], 3);
    assert_eq!(done.result.as_ref().unwrap()["participant_id"], participant);
    assert_eq!(done.result.as_ref().unwrap()["connection_id"], candidate);
    assert_eq!(done.effects.len(), 7);
    assert!(done
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed && e.attempt == 1));
    until(|| phone.methods.lock().unwrap().contains(&Method::Bye)).await;
    until(|| {
        db.query_row(
            "SELECT retired=1 FROM telephone_connection_lifetimes WHERE operation_record_id=?1",
            [&first_done.record_id],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    })
    .await;
    assert_eq!(remote,db.query_row("SELECT generation,runtime_connection_id FROM connections JOIN connection_offers ON connection_id=connections.id WHERE connections.id='codec-source'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap());
    probe(
        peers,
        0,
        &[candidate_index],
        "telephone-return-retained-native-to-generation-three",
    )
    .await;
    probe(
        peers,
        candidate_index,
        if observer { &[] } else { &[0] },
        "telephone-return-generation-three-to-retained-native",
    )
    .await;
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    f.server.store.revoke(&admin, route).unwrap();
    // Late old SIP signaling/packets cannot reenter either native programme.
    phone.digit("5").await;
    phone.digit("5").await;
    phone.digit("#").await;
    tokio::join!(phone.rtp(payload), private_input_isolation(peers));
    probe(
        peers,
        0,
        &[candidate_index],
        "telephone-return-survives-old-route-revocation",
    )
    .await;
    probe(
        peers,
        candidate_index,
        if observer { &[] } else { &[0] },
        "telephone-return-current-native-owner-still-duplex",
    )
    .await;
    controller.transition(first.clone()).await.unwrap();
    controller.transition(command.clone()).await.unwrap();
    assert_eq!(
        status(f, key, &first.operation_id).await.result,
        first_done.result
    );
    assert_eq!(
        status(f, key, &command.operation_id).await.result,
        done.result
    );
    assert_eq!(
        phone.unique(),
        1,
        "return/replay redialed the old telephone route"
    );
    for peer in peers.drain(..) {
        let _ = peer.client.shutdown().await;
    }
    eprintln!("actual telephone/native return passed: same logical member, retained native unchanged, generation3 actual duplex, one SIP call, exact old route absence and replay");
}

// Distinct, orthogonal current markers identify both directions even when the
// established phone's earlier authorized 500 Hz tail remains in a playout queue.
// The phone sends 1 kHz, the retained native peer sends 2 kHz, and the unready
// candidate must receive only its private silence. Neither endpoint may hear
// its own current marker. This also distinguishes an actual echo from a tail.
async fn old_duplex(
    phone: &PeerSip,
    native: &mut Peer,
    candidate: &mut CanonicalClient,
    private_handle: u16,
    observer: bool,
) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    fn amplitude(bytes: &[u8], width: usize) -> i64 {
        let samples = bytes
            .iter()
            .map(|b| {
                let u = !*b;
                let value = (((i32::from(u & 15) << 3) + 132) << ((u >> 4) & 7)) - 132;
                if u & 128 != 0 {
                    -value
                } else {
                    value
                }
            })
            .collect::<Vec<_>>();
        (0..width * 2)
            .map(|phase| {
                samples
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        i64::from(*x)
                            * if (i + phase) % (width * 2) < width {
                                1
                            } else {
                                -1
                            }
                    })
                    .sum::<i64>()
                    .abs()
                    / samples.len().max(1) as i64
            })
            .max()
            .unwrap()
    }
    enum Event {
        Tick,
        Native(Vec<u8>),
        Phone(Vec<u8>),
        Private(Vec<u8>),
    }
    let mut send_until = tokio::time::Instant::now() + Duration::from_secs(12);
    let mut until = send_until + Duration::from_millis(400);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buffer = [0; 4096];
    let mut heard = [0usize; 2];
    while tokio::time::Instant::now() < until {
        let event = tokio::select! {
            _=tick.tick()=>Event::Tick,
            packet=native.client.receive_native_rtp_datagram()=>Event::Native(packet.unwrap()),
            packet=candidate.receive_native_rtp_datagram()=>Event::Private(packet.unwrap()),
            packet=phone.media.recv_from(&mut buffer)=>{let(n,_)=packet.unwrap();Event::Phone(buffer[..n].to_vec())},
        };
        match event {
            Event::Tick if tokio::time::Instant::now() < send_until => {
                native.sequence += 1;
                let packet = pack_rtp_datagram(&RtpDatagram {
                    flags: 0,
                    stream_local_id: native.handle,
                    seq: native.sequence,
                    rtp: RtpMediaPayload {
                        payload: (0..160)
                            .map(|i| if i % 4 < 2 { 0x80 } else { 0x00 })
                            .collect::<Vec<_>>()
                            .into(),
                        payload_type: 0,
                        sequence_number: native.sequence as u16,
                        timestamp: (native.clock.elapsed().as_micros() * 8 / 1000) as u32,
                        ssrc: 0x47368000,
                    },
                })
                .unwrap();
                native.client.send_native_rtp_datagram(&packet).unwrap();
                let seq = phone
                    .rtp_sequence
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut packet = vec![0x80, 0];
                packet.extend_from_slice(&seq.to_be_bytes());
                packet.extend_from_slice(
                    &((phone.rtp_clock.elapsed().as_micros() * 8 / 1000) as u32).to_be_bytes(),
                );
                packet.extend_from_slice(&0x19374621u32.to_be_bytes());
                packet.extend((0..160).map(|i| if i % 8 < 4 { 0x80 } else { 0x00 }));
                let remote = phone.media_target.lock().unwrap().unwrap();
                phone.media.send_to(&packet, remote).await.unwrap();
            }
            Event::Native(packet) => {
                let packet = unpack_rtp_datagram(&packet).unwrap();
                assert_eq!(packet.stream_local_id, native.handle);
                assert!(
                    amplitude(&packet.rtp.payload, 2) < 1000,
                    "retained native heard its own current 2 kHz marker"
                );
                let phone_marker = amplitude(&packet.rtp.payload, 4);
                if observer {
                    assert!(
                        phone_marker < 1000,
                        "Observer telephone published its current marker"
                    );
                } else if phone_marker > 4000 {
                    heard[0] += 1;
                }
            }
            Event::Phone(packet) => {
                if packet.len() < 12 || packet[0] >> 6 != 2 || (200..=211).contains(&packet[1]) {
                    continue;
                }
                assert_eq!(packet[1] & 127, 0);
                let mut header = 12 + usize::from(packet[0] & 15) * 4;
                if packet[0] & 16 != 0 {
                    header += 4 + 4 * usize::from(u16::from_be_bytes([
                        packet[header + 2],
                        packet[header + 3],
                    ]));
                }
                let end = if packet[0] & 32 != 0 {
                    packet.len() - usize::from(*packet.last().unwrap())
                } else {
                    packet.len()
                };
                assert!(header <= end);
                let payload = &packet[header..end];
                assert!(
                    amplitude(payload, 4) < 1000,
                    "phone heard its own current 1 kHz marker"
                );
                if amplitude(payload, 2) > 4000 {
                    heard[1] += 1;
                }
            }
            Event::Private(packet) => {
                let packet = unpack_rtp_datagram(&packet).unwrap();
                assert_eq!(packet.stream_local_id, private_handle);
                assert!(
                    packet.rtp.payload.iter().all(|b| matches!(*b, 0xff | 0x7f)),
                    "established call audio leaked into private candidate"
                );
            }
            _ => {}
        }
        if (observer || heard[0] >= 16)
            && heard[1] >= 16
            && tokio::time::Instant::now() < send_until
        {
            send_until = tokio::time::Instant::now();
            until = send_until + Duration::from_millis(400);
        }
    }
    assert!(
        (observer || heard[0] > 10) && heard[1] > 10,
        "old telephone/native duplex lost before new native readiness: {heard:?}"
    );
    eprintln!("distinct telephone/native old duplex: phone_1khz_to_native={} native_2khz_to_phone={}, own markers absent and private candidate silent observer={observer}",heard[0],heard[1]);
}
