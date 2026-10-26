//! Two actual disjoint private audiences, using public signed native clients.
use super::*;
use connect_protocol::transitions::{
    EffectState, OperationAction, OperationRecord, OperationState,
};
use futures_util::future::select_all;

const RA: usize = 0;
const PA: usize = 1;
const CA: usize = 2;
const PB: usize = 3;
const RB: usize = 4;
const CB: usize = 5;
struct Peer {
    client: CanonicalClient,
    config: Option<CanonicalClientConfig>,
    identity: String,
    participant: String,
    id: String,
    handle: u16,
    clock: tokio::time::Instant,
    sequence: u32,
    live: bool,
}
struct Group {
    command: TransitionCommand,
    key: connect_store::IssuedCredential,
    record: String,
    primary: usize,
    remote: usize,
    candidate: usize,
}
fn revision(f: &Fixture) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='native-session'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}
fn stream_revision(f: &Fixture, id: &str) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}
fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
fn handle(answer: &Envelope) -> u16 {
    answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16
}
async fn enroll(
    f: &Fixture,
    owner: &mut CanonicalClient,
    cid: &str,
    name: &str,
    index: usize,
    public_media: bool,
) -> Peer {
    enroll_scoped(f, owner, cid, name, index, public_media, false).await
}
async fn enroll_scoped(
    f: &Fixture,
    owner: &mut CanonicalClient,
    cid: &str,
    name: &str,
    index: usize,
    public_media: bool,
    handoff: bool,
) -> Peer {
    enroll_kind(
        f,
        owner,
        cid,
        name,
        index,
        public_media,
        handoff,
        IdentityKind::Human,
    )
    .await
}
async fn enroll_kind(
    f: &Fixture,
    owner: &mut CanonicalClient,
    cid: &str,
    name: &str,
    index: usize,
    public_media: bool,
    handoff: bool,
    kind: IdentityKind,
) -> Peer {
    enroll_kind_with_scope(
        f,
        owner,
        cid,
        name,
        index,
        public_media,
        handoff,
        kind,
        None,
    )
    .await
}
async fn enroll_kind_with_scope(
    f: &Fixture,
    owner: &mut CanonicalClient,
    cid: &str,
    name: &str,
    index: usize,
    public_media: bool,
    handoff: bool,
    kind: IdentityKind,
    scope: Option<Scope>,
) -> Peer {
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Identity {
        identity_id: identity,
        ..
    } = f
        .server
        .store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Identity {
                operation_id: format!("identity-{name}"),
                alias: name.into(),
                kind: kind.clone(),
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let room = owner
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let added = owner
        .participant_change(
            cid,
            &format!("add-{name}"),
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":identity,"kind":kind,"role":"consultant"}),
            ),
        )
        .await
        .unwrap();
    let connect_client::membership::MembershipOutcome::Participant { participant, .. } = added
    else {
        panic!()
    };
    let config = if let Some(scope) = scope {
        party_with_scope(f, &identity, index, vec!["audio".into()], scope)
    } else if handoff {
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.extend([
            "stream.subscribe".into(),
            "stream.unsubscribe".into(),
            "session.handoff".into(),
        ]);
        party_with_scope(f, &identity, index, vec!["audio".into()], scope)
    } else {
        party_with_capabilities(f, &identity, index, true, vec!["audio".into()])
    };
    let mut client = CanonicalClient::open(config.clone()).await.unwrap();
    // Existing invitations preserve their original Session intent/capabilities.
    // Initial SIP fixtures use "call" and a narrower codec set than the older
    // all-native conference fixture; enrollment must not renegotiate either.
    let (intent, invitation_capabilities): (String, String) = rusqlite::Connection::open(
        &f.config.database,
    )
    .unwrap()
    .query_row(
        "SELECT intent,capabilities_json FROM session_control WHERE session_id='native-session'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap();
    let invitation_capabilities: Value = serde_json::from_str(&invitation_capabilities).unwrap();
    owner
        .session_command(Request {
            operation_id: format!("invite-{name}"),
            conversation_id: cid.into(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision(f)),
            action: Action::Invite {
                participants: vec![participant.participant_id.clone()],
                medium: "voice".into(),
                intent,
                capabilities: invitation_capabilities.clone(),
            },
        })
        .await
        .unwrap();
    client
        .session_command(Request {
            operation_id: format!("accept-{name}"),
            conversation_id: cid.into(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision(f)),
            action: Action::Accept {
                capabilities: invitation_capabilities,
            },
        })
        .await
        .unwrap();
    let h = if public_media {
        let op = format!("offer-{name}");
        let answer = negotiate_media(&mut client, cid, &op, name,
            json!([{"id":format!("{name}-voice"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]),capabilities()).await;
        wait_native_operation(f, &config.credential, cid, &op).await;
        handle(&answer)
    } else {
        0
    };
    Peer {
        client,
        config: Some(config),
        identity,
        participant: participant.participant_id,
        // Keep private physical IDs distinct from public Participant aliases,
        // so snapshot assertions detect references rather than matching names.
        id: if public_media {
            name.into()
        } else {
            format!("closed-physical-{index}")
        },
        handle: h,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    }
}
async fn select(f: &Fixture, peers: &mut [Peer], cid: &str, target: usize, source: usize) {
    let source = peers[source].participant.clone();
    let id = peers[target].id.clone();
    stream_control(
        &mut peers[target].client,
        cid,
        &id,
        &format!("select-{id}"),
        stream_revision(f, &id),
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":source,"kinds":["audio"]}]}),
    )
    .await;
}
async fn status(f: &Fixture, key: &str, operation: &str) -> OperationRecord {
    let store = f.server.store.clone();
    let key = key.to_owned();
    let operation = operation.to_owned();
    tokio::task::spawn_blocking(move || {
        store.execute_operation_action(
            &key,
            OperationAction::Get {
                operation_id: operation,
            },
        )
    })
    .await
    .unwrap()
    .unwrap()
}
async fn wait_state(f: &Fixture, key: &str, operation: &str, complete: bool) -> OperationRecord {
    let until = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let record = status(f, key, operation).await;
        if if complete {
            record.state == OperationState::Completed
        } else {
            record
                .effects
                .first()
                .is_some_and(|e| e.state == EffectState::Confirmed)
        } {
            return record;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "{operation} did not settle: {record:?}"
        );
        assert!(
            !matches!(
                record.state,
                OperationState::Failed | OperationState::Cancelled
            ),
            "{record:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn begin(
    f: &Fixture,
    peers: &mut [Peer],
    cid: &str,
    name: &str,
    primary: usize,
    remote: usize,
    candidate: usize,
) -> Group {
    let retained = peers[remote].id.clone();
    let (command, key, record) =
        begin_request(f, peers, cid, name, primary, &retained, candidate).await;
    Group {
        command,
        key,
        record,
        primary,
        remote,
        candidate,
    }
}
async fn begin_request(
    f: &Fixture,
    peers: &mut [Peer],
    cid: &str,
    name: &str,
    primary: usize,
    retained: &str,
    candidate: usize,
) -> (TransitionCommand, connect_store::IssuedCredential, String) {
    let identity = peers[primary].identity.clone();
    let participant = peers[primary].participant.clone();
    let connection = peers[primary].id.clone();
    begin_connection_request(
        f,
        peers,
        cid,
        name,
        &identity,
        &participant,
        &connection,
        retained,
        candidate,
    )
    .await
}
async fn begin_connection_request(
    f: &Fixture,
    peers: &mut [Peer],
    cid: &str,
    name: &str,
    primary_identity: &str,
    primary_participant: &str,
    primary_connection: &str,
    retained: &str,
    candidate: usize,
) -> (TransitionCommand, connect_store::IssuedCredential, String) {
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant.operations.extend([
        "session.consult".into(),
        "session.consult.return".into(),
        "operation.cancel".into(),
    ]);
    let key = f
        .server
        .store
        .issue(
            primary_identity,
            grant,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = key.token.clone();
    let mut controller = CanonicalHttpClient::open(config).await.unwrap();
    let command = TransitionCommand {
        operation_id: name.into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(primary_connection.into()),
        expected_revision: revision(f),
        intent: TransitionIntent::Consult {
            retained_connid: retained.into(),
            private_participants: vec![
                primary_participant.into(),
                peers[candidate].participant.clone(),
            ],
            hold_treatment: HoldTreatment::Silence,
        },
        payload_extensions: Default::default(),
    };
    controller.transition(command.clone()).await.unwrap();
    let origin = f
        .server
        .store
        .native_consultation_origin(&key.id, command.clone())
        .unwrap();
    let cfg = peers[candidate].config.as_ref().unwrap().clone();
    let id = peers[candidate].id.clone();
    let op = format!("offer-{id}");
    let offer = json!({"substrate":"quic","capabilities":capabilities(),
        "private_consultation":{"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()},
        "streams_offered":[{"id":format!("{id}-voice"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let endpoint: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&cfg.device_id],
            |r| r.get(0),
        )
        .unwrap();
    // Trusted invitation registration; actual allocation/confirmation below is
    // signed by the independent current native peer, through the public SDK.
    f.server
        .store
        .register_native_consultation_leg(
            &origin,
            connect_protocol::connection_control::OfferIntent {
                operation_id: op.clone(),
                conversation_id: cid.into(),
                session_id: "native-session".into(),
                connection_id: id.clone(),
                by_participant: None,
                expected_revision: Some(0),
                offer: offer.clone(),
            },
            &endpoint,
        )
        .unwrap();
    let answer = negotiate_media_payload(&mut peers[candidate].client, cid, &op, &id, offer).await;
    wait_native_operation(f, &cfg.credential, cid, &op).await;
    peers[candidate].handle = handle(&answer);
    controller.transition(command.clone()).await.unwrap();
    wait_state(f, &key.id, &command.operation_id, false).await;
    (command, key, origin.record_id().into())
}
async fn restore(f: &Fixture, peers: &mut [Peer], cid: &str, group: &Group) {
    let mut cfg = canonical_http_config(f);
    cfg.credential = group.key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let command = TransitionCommand {
        operation_id: format!("return-{}", group.command.operation_id),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(peers[group.primary].id.clone()),
        expected_revision: revision(f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: group.record.clone(),
            retained_connid: peers[group.remote].id.clone(),
        },
        payload_extensions: Default::default(),
    };
    controller.transition(command.clone()).await.unwrap();
    let done = wait_state(f, &group.key.id, &command.operation_id, true).await;
    let result = done.result.as_ref().unwrap();
    assert_eq!(result["remote_continuing"], true);
    assert_eq!(
        result["retired_connection_ids"],
        json!([peers[group.candidate].id])
    );
    assert_eq!(done.effects.len(), 1);
    assert_eq!(done.effects[0].attempt, 1);
    let replay = controller.transition(command).await.unwrap();
    assert_eq!(replay.result, done.result);
    peers[group.candidate].live = false;
}

// One marked source at a time. Every live receiver is sampled, with strict
// zero non-silent frames outside the expected audience and >10 at its partner.
// The final quiet interval drains only the same authorized source's tail.
async fn probe(peers: &mut [Peer], source: usize, allowed: &[usize], name: &str) {
    use rvoip_uctp::substrate::{
        pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload,
    };
    enum Event {
        Tick,
        Packet(
            usize,
            std::result::Result<Vec<u8>, connect_client::ClientError>,
        ),
    }
    let send_until = tokio::time::Instant::now() + Duration::from_secs(2);
    let until = send_until + Duration::from_millis(300);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut counts = vec![0usize; peers.len()];
    while tokio::time::Instant::now() < until {
        let event = {
            let receives = peers
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| p.live && p.handle != 0)
                .map(|(i, p)| {
                    Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
                })
                .collect::<Vec<_>>();
            tokio::select! {
                _=tick.tick()=>Event::Tick,
                ((i,packet),_,_)=select_all(receives)=>Event::Packet(i,packet),
            }
        };
        match event {
            Event::Tick if tokio::time::Instant::now() < send_until => {
                let p = &mut peers[source];
                p.sequence += 1;
                let packet = pack_rtp_datagram(&RtpDatagram {
                    flags: 0,
                    stream_local_id: p.handle,
                    seq: p.sequence,
                    rtp: RtpMediaPayload {
                        payload: (0..160)
                            .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
                            .collect::<Vec<_>>()
                            .into(),
                        payload_type: 0,
                        sequence_number: p.sequence as u16,
                        timestamp: (p.clock.elapsed().as_micros() * 8 / 1000) as u32,
                        ssrc: 0x4736_8000 + source as u32,
                    },
                })
                .unwrap();
                p.client.send_native_media_datagram(&packet).unwrap();
            }
            Event::Packet(i, bytes) => {
                let packet = unpack_rtp_datagram(&bytes.unwrap()).unwrap();
                assert_eq!(packet.stream_local_id, peers[i].handle);
                if packet
                    .rtp
                    .payload
                    .iter()
                    .any(|b| !matches!(*b, 0xff | 0x7f))
                {
                    assert!(
                        allowed.contains(&i),
                        "{name}: source {} reached forbidden receiver {}",
                        peers[source].id,
                        peers[i].id
                    );
                    counts[i] += 1;
                }
            }
            _ => {}
        }
    }
    eprintln!("private-group audio {name}: source={source} allowed={allowed:?} frames={counts:?}");
    for i in allowed {
        assert!(counts[*i] > 10, "{name}: too few marked frames: {counts:?}");
    }
}

#[tokio::test]
async fn production_disjoint_private_consultations_preserve_each_others_audio_and_return_independently(
) {
    let codecs::Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = codecs::setup().await;
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let pid = |id: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == id)
            .unwrap()
            .participant_id
            .clone()
    };
    let pa = clients.pop().unwrap();
    let ra = clients.pop().unwrap();
    let make = |client, identity: String, participant, id: &str, h| Peer {
        client,
        config: None,
        identity,
        participant,
        id: id.into(),
        handle: h,
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    };
    let mut peers = vec![
        make(
            ra,
            f.peer.identity_id.clone(),
            pid(&f.peer.identity_id),
            "codec-source",
            sh,
        ),
        make(
            pa,
            f.client.identity_id.clone(),
            pid(&f.client.identity_id),
            "codec-target",
            th,
        ),
    ];
    for (index, name, public) in [
        (201, "groups-candidate-a", false),
        (202, "groups-primary-b", true),
        (203, "groups-remote-b", true),
        (204, "groups-candidate-b", false),
    ] {
        let peer = enroll(&f, &mut peers[PA].client, &cid, name, index, public).await;
        peers.push(peer);
    }
    select(&f, &mut peers, &cid, PB, RB).await;
    select(&f, &mut peers, &cid, RB, PB).await;
    probe(&mut peers, RA, &[PA], "original-a-public").await;
    probe(&mut peers, PB, &[RB], "original-b-public").await;
    let a = begin(&f, &mut peers, &cid, "disjoint-consult-a", PA, RA, CA).await;
    probe(&mut peers, PA, &[CA], "a-private-before-b").await;
    probe(&mut peers, CA, &[PA], "a-private-reverse-before-b").await;
    wait_state(&f, &a.key.id, &a.command.operation_id, true).await;
    // Completed private work still owns the held receiver. A competing command
    // must fail before receipt/effect reservation, rather than strand return.
    let mut overlap = a.command.clone();
    overlap.operation_id = "overlapping-held-remote".into();
    overlap.connection_id = Some(peers[RA].id.clone());
    overlap.expected_revision = revision(&f);
    overlap.intent = TransitionIntent::Consult {
        retained_connid: peers[RB].id.clone(),
        private_participants: vec![peers[RA].participant.clone(), peers[CB].participant.clone()],
        hold_treatment: HoldTreatment::Silence,
    };
    let mut cfg = canonical_http_config(&f);
    cfg.credential = a.key.token.clone();
    let mut http = CanonicalHttpClient::open(cfg).await.unwrap();
    let error = http.transition(overlap).await.unwrap_err();
    assert!(
        matches!(error,connect_client::ClientError::Application(ref e) if e.code=="conflict"),
        "{error:?}"
    );
    let n: u64 = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='overlapping-held-remote'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    let b = begin(&f, &mut peers, &cid, "disjoint-consult-b", PB, RB, CB).await;
    for (source, target, name) in [
        (PA, CA, "both-a"),
        (CA, PA, "both-a-reverse"),
        (PB, CB, "both-b"),
        (CB, PB, "both-b-reverse"),
    ] {
        probe(&mut peers, source, &[target], name).await;
    }
    wait_state(&f, &b.key.id, &b.command.operation_id, true).await;
    for (receiver, own_candidate, other_candidate) in [
        (PA, Some(CA), CB),
        (CA, Some(CA), CB),
        (PB, Some(CB), CA),
        (CB, Some(CB), CA),
        (RA, None, CA),
        (RB, None, CB),
    ] {
        let snapshot = peers[receiver]
            .client
            .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        let raw = serde_json::to_string(&snapshot).unwrap();
        assert!(
            !raw.contains(&peers[other_candidate].id),
            "receiver={receiver}: unselected private group appeared in snapshot: {:?}",
            snapshot
                .items
                .iter()
                .filter(|item| item.to_string().contains(&peers[other_candidate].id))
                .collect::<Vec<_>>()
        );
        if let Some(own) = own_candidate {
            assert!(raw.contains(&peers[own].id));
        } else {
            assert!(!raw.contains(&peers[if other_candidate == CA { CB } else { CA }].id));
        }
    }
    for key in [&a.key.id, &b.key.id] {
        assert_eq!(
            f.server
                .store
                .context_for_session(key, "native-session")
                .err()
                .unwrap()
                .code,
            "private-context-required"
        );
    }
    restore(&f, &mut peers, &cid, &a).await;
    for (source, target, name) in [
        (PA, RA, "return-a-public"),
        (RA, PA, "return-a-public-reverse"),
        (PB, CB, "return-a-preserves-b"),
        (CB, PB, "return-a-preserves-b-reverse"),
    ] {
        probe(&mut peers, source, &[target], name).await;
    }
    restore(&f, &mut peers, &cid, &b).await;
    // Returned original routes are reusable; this guard must not become a
    // permanent Session lock. Cancel a new zero-effect intention publicly.
    let mut again = a.command.clone();
    again.operation_id = "reuse-returned-original-routes".into();
    again.expected_revision = revision(&f);
    let mut cfg = canonical_http_config(&f);
    cfg.credential = a.key.token.clone();
    let mut reuse = CanonicalHttpClient::open(cfg).await.unwrap();
    let accepted = reuse.transition(again.clone()).await.unwrap();
    assert_eq!(
        serde_json::to_value(accepted.state).unwrap(),
        json!("accepted")
    );
    assert!(status(&f, &a.key.id, &again.operation_id)
        .await
        .effects
        .is_empty());
    reuse.request_value("operation.cancel",json!({"operation_id":again.operation_id,"reason":"qualified reuse, no new private leg"}),CommandScope {
        conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:again.connection_id.clone(),operation_id:Some("cancel-reused-returned-routes".into()),..Default::default()
    }).await.unwrap().ack().unwrap();
    let cancelled = status(&f, &a.key.id, &again.operation_id).await;
    assert_eq!(cancelled.state, OperationState::Cancelled);
    assert!(cancelled.effects.is_empty());
    for (source, target, name) in [
        (PA, RA, "return-b-preserves-a"),
        (PB, RB, "return-b-public"),
        (RB, PB, "return-b-public-reverse"),
    ] {
        probe(&mut peers, source, &[target], name).await;
    }
    wait_slots(&f, 4).await;
    for g in [&a, &b] {
        let done = status(&f, &g.key.id, &g.command.operation_id).await;
        assert_eq!(done.state, OperationState::Completed);
        assert_eq!(done.effects.len(), 1);
        assert_eq!(done.effects[0].attempt, 1);
    }
    peers[PA]
        .client
        .session_command(Request {
            operation_id: "end-disjoint-private-session".into(),
            conversation_id: cid,
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision(&f)),
            action: Action::End {
                reason_code: 1000,
                reason: "finished disjoint consultations".into(),
            },
        })
        .await
        .unwrap();
    wait_slots(&f, 0).await;
    for p in peers {
        let result = p.client.shutdown().await;
        if p.live {
            result.unwrap();
        }
    }
    f.server.shutdown().await.unwrap();
}

#[path = "groups/handoff.rs"]
mod handoff;

#[path = "groups/preparation.rs"]
mod preparation;
#[path = "groups/replacement.rs"]
mod replacement;

#[path = "groups/sdk_movements.rs"]
mod sdk_movements;
