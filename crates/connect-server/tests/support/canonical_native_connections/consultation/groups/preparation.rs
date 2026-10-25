//! Actual target-owned signed isolation before any caller handoff exists.
use super::*;
use connect_client::canonical::PreparedHandoffNativeAudio;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};

// All three physical peers are sampled for every marked source. Isolation must
// neither disturb the original call nor leak either direction to preparation.
async fn isolated_probe(
    clients: &mut [CanonicalClient],
    prepared: &mut PreparedHandoffNativeAudio,
    handles: [u16; 2],
    source: usize,
) {
    let allowed = if source < 2 { vec![1 - source] } else { vec![] };
    route_probe(
        clients,
        prepared,
        handles,
        [true, true],
        source,
        &allowed,
        &mut WireClock::new(),
    )
    .await;
}
struct WireClock {
    start: tokio::time::Instant,
    sequence: [u32; 2],
}
impl WireClock {
    fn new() -> Self {
        Self {
            start: tokio::time::Instant::now(),
            sequence: [0, 0],
        }
    }
}
async fn route_probe(
    clients: &mut [CanonicalClient],
    prepared: &mut PreparedHandoffNativeAudio,
    handles: [u16; 2],
    live: [bool; 2],
    source: usize,
    allowed: &[usize],
    wire: &mut WireClock,
) {
    route_probe_many(
        clients,
        &mut [prepared],
        handles,
        live,
        &[true],
        source,
        allowed,
        wire,
    )
    .await;
}
async fn route_probe_many(
    clients: &mut [CanonicalClient],
    prepared: &mut [&mut PreparedHandoffNativeAudio],
    handles: [u16; 2],
    live: [bool; 2],
    prepared_live: &[bool],
    source: usize,
    allowed: &[usize],
    wire: &mut WireClock,
) {
    use futures_util::FutureExt;
    let clock = tokio::time::Instant::now();
    let until = clock + Duration::from_millis(2300);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let marked = (0..160)
        .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
        .collect::<Vec<_>>();
    let mut counts = vec![0usize; 2 + prepared.len()];
    while tokio::time::Instant::now() < until {
        enum Event {
            Tick,
            Packet(usize, Vec<u8>),
        }
        let event = {
            let mut receives = clients
                .iter_mut()
                .enumerate()
                .filter(|(i, _)| live[*i])
                .map(|(i, p)| {
                    async move {
                        let packet = p.receive_native_media_datagram().await.unwrap();
                        (
                            i,
                            unpack_rtp_datagram(&packet).unwrap().rtp.payload.to_vec(),
                        )
                    }
                    .boxed_local()
                })
                .collect::<Vec<_>>();
            receives.extend(
                prepared
                    .iter_mut()
                    .enumerate()
                    .filter(|(i, _)| prepared_live[*i])
                    .map(|(i, p)| {
                        async move { (i + 2, p.receive_pcmu().await.unwrap()) }.boxed_local()
                    }),
            );
            tokio::select! {
                _ = tick.tick() => Event::Tick,
                ((i,p),_,_)=select_all(receives)=>Event::Packet(i,p),
            }
        };
        match event {
            Event::Tick if clock.elapsed() < Duration::from_secs(2) => {
                if source >= 2 {
                    prepared[source - 2].send_pcmu(&marked).unwrap();
                } else {
                    wire.sequence[source] += 1;
                    let sequence = wire.sequence[source];
                    let packet = pack_rtp_datagram(&RtpDatagram {
                        flags: 0,
                        stream_local_id: handles[source],
                        seq: sequence,
                        rtp: RtpMediaPayload {
                            payload: marked.clone().into(),
                            payload_type: 0,
                            sequence_number: sequence as u16,
                            timestamp: (wire.start.elapsed().as_millis() / 20 * 160) as u32,
                            ssrc: 0x4736_9000 + source as u32,
                        },
                    })
                    .unwrap();
                    clients[source].send_native_media_datagram(&packet).unwrap();
                }
            }
            Event::Packet(receiver, bytes) if bytes.iter().any(|b| !matches!(*b, 0xff | 0x7f)) => {
                assert!(
                    allowed.contains(&receiver),
                    "preparation leaked source {source} to receiver {receiver}"
                );
                counts[receiver] += 1;
            }
            _ => {}
        }
    }
    for receiver in allowed {
        assert!(
            counts[*receiver] > 10,
            "too few authorized frames: {counts:?}"
        );
    }
    eprintln!("target preparation strict audio: source={source} marked_frames={counts:?}");
}

#[tokio::test]
async fn production_target_owned_human_ai_and_observer_preparation_stays_muted_and_closed() {
    for (kind, observer) in [
        (IdentityKind::Human, false),
        (IdentityKind::Ai, false),
        (IdentityKind::Human, true),
    ] {
        let codecs::Running {
            f,
            mut clients,
            cid,
            sh,
            th,
        } = codecs::setup().await;
        let target = enroll_kind(
            &f,
            &mut clients[1],
            &cid,
            "prepared-target",
            231,
            false,
            true,
            kind.clone(),
        )
        .await;
        if observer {
            clients[1]
                .session_command(Request {
                    operation_id: "prepare-observer-role".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision(&f)),
                    action: Action::Role {
                        participant_id: target.participant.clone(),
                        role: "observer".into(),
                    },
                })
                .await
                .unwrap();
        }
        let before_role:String = rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&target.participant],|r|r.get(0)).unwrap();
        if observer {
            assert_eq!(before_role, "observer");
        }
        let config = target.config.unwrap();
        if kind == IdentityKind::Human && !observer {
            // The identity's other credential and the caller's broad control
            // authority cannot lend the missing named target permission.
            let denied = party_with_scope(
                &f,
                &config.identity_id,
                232,
                vec!["audio".into()],
                Scope::collaboration(vec!["*".into()]),
            );
            let denied = CanonicalClient::open(denied).await.unwrap();
            let refused = tokio::time::timeout(Duration::from_secs(10), denied.prepare_handoff_native_audio(CommandScope {
                conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("refused-target-preparation".into()),operation_id:Some("refused-target-preparation".into()),expected_revision:Some(0),
            },json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"preparation-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}}))).await.unwrap();
            assert!(refused.is_err());
            let allocated:bool = rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id='refused-target-preparation')",[],|r|r.get(0)).unwrap();
            assert!(!allocated);
        }
        let participant = target.participant;
        let connection = target.id;
        let mut prepared = tokio::time::timeout(Duration::from_secs(15),target.client.prepare_handoff_native_audio(CommandScope {
            conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some(connection.clone()),operation_id:Some("prepare-target".into()),expected_revision:Some(0),
        },json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"preparation-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}],"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}}))).await.unwrap().unwrap();
        assert_eq!(prepared.connection_id(), connection);
        let own_key = f
            .server
            .store
            .authenticate(&config.credential)
            .unwrap()
            .credential_id;
        let record = status(&f, &own_key, "prepare-target").await;
        assert_eq!(record.state, OperationState::Completed);
        assert_eq!(prepared.policy_record_id(), record.record_id);
        assert_eq!(record.effects.len(), 1);
        assert_eq!(record.effects[0].attempt, 1);
        assert_eq!(
            record.result.as_ref().unwrap()["handoff_preparation"],
            json!({"profile":"connect-handoff-preparation-v1","prepared_muted":true})
        );
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let handoffs: u64 = c
            .query_row("SELECT COUNT(*) FROM native_handoffs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(handoffs, 0);
        let role:String = c.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&participant],|r|r.get(0)).unwrap();
        assert_eq!(role, before_role);
        let recipients:Vec<String> = c.prepare("SELECT DISTINCT recipient FROM journal WHERE json_extract(data_json,'$.connection_id')=?1").unwrap().query_map([&connection],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(recipients, vec![config.identity_id.clone()]);
        drop(c);
        for source in 0..3 {
            isolated_probe(&mut clients, &mut prepared, [sh, th], source).await;
        }
        prepared.shutdown().await.unwrap();
        wait_slots(&f, 2).await;
        clients[1]
            .session_command(Request {
                operation_id: "end-preparation-session".into(),
                conversation_id: cid,
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision(&f)),
                action: Action::End {
                    reason_code: 1000,
                    reason: "preparation isolation verified".into(),
                },
            })
            .await
            .unwrap();
        wait_slots(&f, 0).await;
        for client in clients {
            let _ = client.shutdown().await;
        }
        f.server.shutdown().await.unwrap();
        eprintln!("target preparation actual native profile passed: {kind:?} observer={observer}");
    }
}

#[path = "preparation/takeover.rs"]
mod takeover;

#[path = "preparation/chain.rs"]
mod chain;
