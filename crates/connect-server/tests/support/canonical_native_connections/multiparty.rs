//! Fifty distinct actual native Participants, never injected presence or flows.
use super::*;
use futures_util::future::{join_all, select_all};
use std::collections::{BTreeSet, VecDeque};
#[path = "multiparty/events.rs"]
mod events;
#[path = "multiparty/future.rs"]
mod future;
#[path = "multiparty/media.rs"]
mod media;
const N: usize = 50;
struct Peer {
    index: usize,
    client: CanonicalClient,
    config: CanonicalClientConfig,
    participant: String,
    connection: String,
    handle: u16,
    publish_handle: u16,
    sequence: u32,
    clock: tokio::time::Instant,
    active: bool,
    readable: bool,
    self_joined_seen: bool,
    late_self_joins: Vec<Value>,
    events: VecDeque<Envelope>,
    receipts: Vec<Value>,
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
fn handle(frame: &Envelope) -> u16 {
    frame.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "bounded fifty-peer capacity lane; run scripts/rehearse-gate6-fifty-participants.py"]
async fn production_fifty_native_participants_preserve_explicit_fanout_handles_and_lifecycle() {
    for algorithm in ["EdDSA", "ES256"] {
        exercise(algorithm).await;
    }
}
async fn exercise(algorithm: &str) {
    let mut f = Fixture::with_native_end_scope(algorithm, 128).await;
    // Explicit isolated capacity profile; production defaults remain unchanged.
    f.server.shutdown().await.unwrap();
    f.config.limits.participants = N;
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    f.client.endpoint = f.server.address;
    f.client.timeout_ms = 10_000;
    ready_endpoint(&f).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut identities = vec![f.client.identity_id.clone()];
    for index in 1..N {
        let kind = match index % 4 {
            0 => IdentityKind::Human,
            1 => IdentityKind::Ai,
            2 => IdentityKind::External,
            _ => IdentityKind::System,
        };
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Identity {
                    operation_id: format!("large-identity-{index}"),
                    alias: format!("large-identity-{index}"),
                    kind,
                    display_name: Some(format!("Large Participant {index}")),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        identities.push(identity_id);
    }
    let mut configs = vec![f.client.clone()];
    for (index, identity) in identities.iter().enumerate().skip(1) {
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.extend([
            "stream.subscribe".into(),
            "stream.unsubscribe".into(),
            "connection.end".into(),
        ]);
        configs.push(party_with_scope(
            &f,
            identity,
            500 + index,
            vec!["audio".into()],
            scope,
        ));
    }
    let mut clients = Vec::new();
    for config in &configs {
        clients.push(CanonicalClient::open(config.clone()).await.unwrap());
    }
    let mut input =
        connect_client::conversations::CreateRequest::persistent(identities[1..].to_vec());
    input.tenant_id = Some("tnt_default".into());
    let room = clients[0]
        .create_conversation("large-room", input)
        .await
        .unwrap();
    assert_eq!(room.participants.len(), N);
    let pids = identities
        .iter()
        .map(|id| {
            room.participants
                .iter()
                .find(|p| &p.identity_id == id)
                .unwrap()
                .participant_id
                .clone()
        })
        .collect::<Vec<_>>();
    clients[0]
        .session_command(Request {
            operation_id: "large-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: pids[1..].to_vec(),
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    for (index, client) in clients.iter_mut().enumerate().skip(1) {
        client
            .session_command(Request {
                operation_id: format!("large-accept-{index}"),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision(&f)),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
    }
    let mut peers = Vec::new();
    let mut answers = Vec::new();
    for (index, (mut client, config)) in clients.into_iter().zip(configs).enumerate() {
        let connection = format!("large-publisher-{index}");
        let answer=negotiate_av(&mut client,&room.id,&format!("large-offer-{index}"),&connection,json!([{"id":format!("large-voice-{index}"),"kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}])).await;
        wait_native_operation(
            &f,
            &config.credential,
            &room.id,
            &format!("large-offer-{index}"),
        )
        .await;
        answers.push(answer.original().clone());
        peers.push(Peer {
            index,
            client,
            config,
            participant: pids[index].clone(),
            connection,
            handle: handle(&answer),
            publish_handle: handle(&answer),
            sequence: 0,
            clock: tokio::time::Instant::now(),
            active: true,
            readable: true,
            self_joined_seen: false,
            late_self_joins: Vec::new(),
            events: VecDeque::new(),
            receipts: Vec::new(),
        });
    }
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_connection_flow_facts",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    join_all(peers.iter_mut().map(media::ready)).await;
    let facts: Vec<String> = db
        .prepare("SELECT connection_id FROM native_connection_flow_facts ORDER BY connection_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    eprintln!("fifty-participant actual directional ready facts: {facts:?}");
    // Datagrams are best effort. Preserve the concurrent startup observation,
    // then recover any missing actual readiness with the same original peer.
    let _ = f.server.service.canonical_runtime_snapshot().unwrap();
    let burst_ready_count = facts.len();
    let mut recovered = Vec::new();
    for (index, peer) in peers.iter_mut().enumerate() {
        if !facts.contains(&peer.connection) {
            media::recover_ready(peer, &db, &f).await;
            recovered.push(index);
        }
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_connection_flow_facts",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        N
    );
    let initial_events = events::joined(&mut peers, &room.id, &pids).await;
    eprintln!("fifty-participant {algorithm}: all publisher lifecycle sets complete");
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        N
    );
    let mut receiver_answers = Vec::new();
    // Publication has actual directional readiness before Session start. Each
    // subscriber now negotiates its own receive-only port on the same peer.
    for index in 1..peers.len() {
        let peer = &mut peers[index];
        let connection = format!("large-receiver-{index}");
        let answer=negotiate_av(&mut peer.client,&room.id,&format!("large-receiver-offer-{index}"),&connection,json!([{"id":format!("large-monitor-{index}"),"kind":"audio","direction":"recvonly","codec_preferences":["g.711-mu"]}])).await;
        wait_native_operation(
            &f,
            &peer.config.credential,
            &room.id,
            &format!("large-receiver-offer-{index}"),
        )
        .await;
        peer.connection = connection;
        peer.handle = handle(&answer);
        receiver_answers.push(answer.original().clone());
        assert_ne!(
            peer.handle, peer.publish_handle,
            "receive and publish bindings cannot alias on one peer"
        );
        if index % 8 == 0 {
            join_all(peers.iter_mut().map(media::drain)).await;
        }
    }
    join_all(peers.iter_mut().map(media::drain)).await;
    assert_ne!(
        peers[0].publish_handle, peers[1].handle,
        "fanout rewrites the source handle to the receiver binding"
    );
    let runtime = f.server.service.canonical_runtime_snapshot().unwrap();
    if runtime.connections != 99 {
        let states: String = db.query_row("SELECT json_group_array(json_object('id',id,'state',state)) FROM connections WHERE session_id='native-session'", [], |r| r.get(0)).unwrap();
        eprintln!(
            "fifty-participant receiver admission snapshot: {runtime:?}; durable states: {states}"
        );
    }
    wait_slots(&f, 99).await;
    eprintln!("fifty-participant {algorithm}: all99 admissions complete");
    let unsubscribed =
        media::probe(&mut peers, 0, &BTreeSet::new(), "no-implicit-subscriptions").await;
    assert!(
        unsubscribed["received_packets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|count| count.as_u64() == Some(0)),
        "unsubscribed receivers must receive no media, including silence: {unsubscribed}"
    );
    let mut commands = Vec::new();
    for index in 1..peers.len() {
        let peer = &mut peers[index];
        let selector = match index % 3 {
            0 => json!({"strm_id":"large-voice-0"}),
            1 => json!({"from_participant":pids[0]}),
            _ => json!({"from_participant":pids[0],"kinds":["audio"]}),
        };
        events::subscription(
            peer,
            &room.id,
            &format!("large-subscribe-{index}"),
            stream_revision(&f, &peer.connection),
            "stream.subscribe",
            json!({"subscriptions":[selector.clone()]}),
        )
        .await;
        commands.push(json!({"connection":peer.connection,"selector":selector}));
        if index % 8 == 0 {
            join_all(peers.iter_mut().map(media::drain)).await;
            eprintln!("fifty-participant {algorithm}: completed subscription {index}/49");
        }
    }
    join_all(peers.iter_mut().map(media::drain)).await;
    let all = (1..N).collect::<BTreeSet<_>>();
    let receiver_connections = peers
        .iter()
        .skip(1)
        .map(|p| p.connection.clone())
        .collect::<Vec<_>>();
    let receiver_ready_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let receiver_startup = media::startup(
        &mut peers,
        0,
        &all,
        &db,
        &receiver_connections,
        "all49-receivers",
        receiver_ready_deadline,
    )
    .await;
    let receiver_events = events::receivers(&mut peers, &room.id, receiver_ready_deadline).await;
    let fanout = media::probe(&mut peers, 0, &all, "all49-subscribers").await;
    let future = future::verify(&mut peers, &f, &room.id).await;
    let removed_revision = stream_revision(&f, &peers[49].connection);
    for (operation, expected) in [
        ("large-unsubscribe", removed_revision),
        ("large-unsubscribe", removed_revision),
        ("large-unsubscribe-again", removed_revision + 1),
    ] {
        let peer = &mut peers[49];
        events::subscription(
            peer,
            &room.id,
            operation,
            expected,
            "stream.unsubscribe",
            json!({"strm_ids":["large-voice-0"]}),
        )
        .await;
    }
    assert_eq!(
        stream_revision(&f, &peers[49].connection),
        removed_revision + 1
    );
    let subscribed = (1..49).collect::<BTreeSet<_>>();
    let after_unsubscribe = media::probe(
        &mut peers,
        0,
        &subscribed,
        "unsubscribe-replay-keeps-other48",
    )
    .await;
    let left_request = Request {
        operation_id: "large-leave".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(revision(&f)),
        action: Action::Leave {
            participant_id: pids[49].clone(),
        },
    };
    peers[49]
        .client
        .session_command(left_request.clone())
        .await
        .unwrap();
    peers[49].active = false;
    let left = events::left(&mut peers, &room.id, &pids[49]).await;
    peers[49]
        .client
        .session_command(left_request)
        .await
        .unwrap();
    wait_slots(&f, 97).await;
    let after_leave = media::probe(&mut peers, 0, &subscribed, "one-leaves-other48-remain").await;
    let snapshot = peers[0]
        .client
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    peers[0].client.participant_change(&room.id,"large-remove",Some(snapshot.revision),payload("conversation.participant.remove",json!({"participant_id":pids[48],"reason":"large Session current membership removal"}))).await.unwrap();
    peers[48].active = false;
    let removed = events::left(&mut peers, &room.id, &pids[48]).await;
    wait_slots(&f, 95).await;
    let after_remove = media::probe(
        &mut peers,
        0,
        &(1..48).collect(),
        "membership-removal-preserves-other47",
    )
    .await;
    let refusal = peers[48]
        .client
        .send_message(
            "large-removed-refusal",
            &room.id,
            "large-removed-refusal-message",
            connect_client::messages::MessageDraft::text_all("removed cannot publish"),
        )
        .await;
    assert!(refusal.is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, usize>(0))
            .unwrap(),
        1
    );
    observe(
        json!({"algorithm":algorithm,"participants":pids,"answers":answers,"initial_events":initial_events,"receiver_answers":receiver_answers,"receiver_startup":receiver_startup,"receiver_events":receiver_events,"future_publication":future,"selectors":commands,"subscription_receipts":peers.iter().enumerate().map(|(i,p)|json!({"peer_index":i,"frames":p.receipts,"late_self_joins":p.late_self_joins})).collect::<Vec<_>>(),"phases":{"unsubscribed":unsubscribed,"fanout":fanout,"after_unsubscribe":after_unsubscribe,"after_leave":after_leave,"after_remove":after_remove},"left_events":left,"removed_events":removed,"peer_handles":peers.iter().map(|p|p.handle).collect::<Vec<_>>(),"burst_ready_count":burst_ready_count,"recovered_ready_indexes":recovered,"configured_participants":f.config.limits.participants,"remaining_connections":f.server.service.canonical_runtime_snapshot().unwrap().connections}),
    );
    for peer in &mut peers {
        peer.client.close();
    }
    f.server.shutdown().await.unwrap();
}
fn observe(value: Value) {
    if let Some(path) = std::env::var_os("CONNECT_FIFTY_PARTICIPANT_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
}
