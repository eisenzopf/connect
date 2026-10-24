//! Signed live multicast on every actual peer, without consuming journal hints.
use super::*;
pub(super) fn late_self_join(peer: &mut Peer, cid: &str, frame: &Envelope) -> bool {
    if frame.kind() != "session.participant.joined" {
        return false;
    }
    assert_eq!(frame.original()["cid"], cid);
    assert_eq!(frame.original()["sid"], "native-session");
    assert_eq!(
        frame.original()["payload"]["participant"]["participant_id"],
        peer.participant
    );
    assert_eq!(
        frame.original()["payload"]["via_connection"],
        format!("large-publisher-{}", peer.index)
    );
    assert!(
        !peer.self_joined_seen,
        "duplicate late self joined at peer{}",
        peer.index
    );
    peer.self_joined_seen = true;
    peer.late_self_joins.push(frame.original().clone());
    true
}
pub(super) async fn joined(peers: &mut [Peer], cid: &str, pids: &[String]) -> Vec<Value> {
    let all = pids.iter().cloned().collect::<BTreeSet<_>>();
    let mut observed = Vec::new();
    for (index, peer) in peers.iter_mut().enumerate() {
        let expected = all
            .iter()
            .filter(|id| *id != &peer.participant)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut joined = BTreeSet::new();
        let mut started = false;
        let mut ready = false;
        let mut opened = false;
        let mut frames = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !started || !ready || !opened || joined != expected {
            let frame = if let Some(frame) = peer.events.pop_front() {
                frame
            } else {
                tokio::time::timeout_at(deadline, peer.client.receive()).await.unwrap_or_else(|_|panic!("large peer{index}: started={started} ready={ready} opened={opened} joined={}/49",joined.len())).unwrap()
            };
            assert_eq!(frame.original()["cid"], cid);
            assert_eq!(frame.original()["sid"], "native-session");
            let body = &frame.original()["payload"];
            match frame.kind() {
                "session.started" => {
                    assert!(!started);
                    started = true;
                    assert_eq!(
                        body["participants_present"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap().to_owned())
                            .collect::<BTreeSet<_>>(),
                        all
                    );
                    assert_eq!(body["active_connections"].as_array().unwrap().len(), N);
                }
                "session.participant.joined" => {
                    let id = body["participant"]["participant_id"].as_str().unwrap();
                    assert!(all.contains(id));
                    if id == peer.participant {
                        assert!(
                            !peer.self_joined_seen,
                            "duplicate self joined at peer{index}"
                        );
                        peer.self_joined_seen = true;
                        frames.push(frame.original().clone());
                        continue;
                    }
                    assert!(
                        joined.insert(id.to_owned()),
                        "large peer{index}: duplicate joined{id}"
                    );
                }
                "connection.ready" => {
                    assert_eq!(frame.original()["connid"], peer.connection);
                    assert!(!ready);
                    ready = true;
                }
                "stream.opened" => {
                    assert!(ready && !opened);
                    assert_eq!(body["stream"]["stream_local_id"], peer.handle);
                    assert_eq!(body["stream"]["strm_id"], format!("large-voice-{index}"));
                    opened = true;
                }
                "connection.quality" | "recording.vcon-ready" => {}
                kind => panic!(
                    "unexpected large Session event{kind}: {:?}",
                    frame.original()
                ),
            }
            frames.push(frame.original().clone());
        }
        observed.push(json!({"peer_index":index,"participant_id":peer.participant,"joined":joined,"frames":frames}));
    }
    observed
}
pub(super) async fn left(peers: &mut [Peer], cid: &str, departed: &str) -> Vec<Value> {
    let mut frames = Vec::new();
    for (index, peer) in peers.iter_mut().enumerate().filter(|(_, p)| p.active) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            let frame = if let Some(frame) = peer.events.pop_front() {
                frame
            } else {
                tokio::time::timeout_at(deadline, peer.client.receive())
                    .await
                    .unwrap()
                    .unwrap()
            };
            if late_self_join(peer, cid, &frame) {
                continue;
            }
            if frame.kind() == "session.participant.left" {
                assert_eq!(frame.original()["cid"], cid);
                assert_eq!(frame.original()["sid"], "native-session");
                assert_eq!(frame.original()["payload"]["participant_id"], departed);
                frames.push(json!({"peer_index":index,"frame":frame.original()}));
                break;
            }
            assert!(
                matches!(
                    frame.kind(),
                    "connection.quality" | "stream.closed" | "recording.vcon-ready"
                ),
                "unexpected large departure frame: {:?}",
                frame.original()
            );
        }
    }
    frames
}

pub(super) async fn receivers(
    peers: &mut [Peer],
    cid: &str,
    deadline: tokio::time::Instant,
) -> Vec<Value> {
    let mut observed = Vec::new();
    for (index, peer) in peers.iter_mut().enumerate().skip(1) {
        let mut ready = false;
        let mut opened = false;
        let mut frames = Vec::new();
        while !ready || !opened {
            assert!(
                tokio::time::Instant::now() < deadline,
                "receiver{index}: readiness deadline"
            );
            let frame = if let Some(frame) = peer.events.pop_front() {
                frame
            } else {
                tokio::time::timeout_at(deadline, peer.client.receive())
                    .await
                    .unwrap()
                    .unwrap()
            };
            if late_self_join(peer, cid, &frame) {
                continue;
            }
            match frame.kind() {
                "connection.ready" => {
                    assert_eq!(frame.original()["connid"], peer.connection);
                    assert!(!ready);
                    ready = true;
                }
                "stream.opened" => {
                    assert!(ready && !opened);
                    assert_eq!(
                        frame.original()["payload"]["stream"]["stream_local_id"],
                        peer.handle
                    );
                    opened = true;
                }
                "connection.quality" | "recording.vcon-ready" => {}
                kind => panic!(
                    "unexpected receiver{index} lifecycle frame{kind}: {:?}",
                    frame.original()
                ),
            }
            assert_eq!(frame.original()["cid"], cid);
            frames.push(frame.original().clone());
        }
        observed.push(json!({"peer_index":index,"frames":frames}));
    }
    observed
}

pub(super) async fn subscription(
    peer: &mut Peer,
    cid: &str,
    operation: &str,
    revision: u64,
    kind: &str,
    input: Value,
) {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(peer.connection.clone()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    };
    let request = peer.client.send_value(kind, input, scope).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut completed = false;
    loop {
        let frame = tokio::time::timeout_at(deadline, peer.client.receive())
            .await
            .unwrap()
            .unwrap();
        if late_self_join(peer, cid, &frame) {
            continue;
        }
        match frame.kind() {
            "ack" => {
                assert_eq!(frame.original()["in_reply_to"], request);
                assert!(
                    completed,
                    "{operation}: ACK arrived without completed operation"
                );
                peer.receipts.push(frame.original().clone());
                return;
            }
            "operation.updated" => {
                assert_eq!(frame.original()["in_reply_to"], request);
                assert_eq!(frame.original()["operation_id"], operation);
                assert!(!completed, "{operation}: duplicate completion");
                assert_eq!(
                    frame.original()["payload"]["state"],
                    "completed",
                    "{:?}",
                    frame.original()
                );
                completed = true;
                peer.receipts.push(frame.original().clone());
            }
            "connection.ready"
            | "stream.opened"
            | "stream.closed"
            | "connection.quality"
            | "session.started"
            | "session.participant.joined"
            | "session.participant.left"
            | "recording.vcon-ready" => peer.events.push_back(frame),
            kind => panic!(
                "unexpected large subscription frame{kind}: {:?}",
                frame.original()
            ),
        }
    }
}
