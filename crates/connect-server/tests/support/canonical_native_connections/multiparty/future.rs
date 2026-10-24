//! A later real publication follows retained member intent, not exact Stream IDs.
use super::*;

pub(super) async fn verify(peers: &mut [Peer], f: &Fixture, cid: &str) -> Value {
    join_all(peers.iter_mut().map(media::drain)).await;
    let connection = "large-future-publisher";
    let answer = negotiate_av(
        &mut peers[0].client,
        cid,
        "large-future-offer",
        connection,
        json!([{"id":"large-future-voice","kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}]),
    )
    .await;
    wait_native_operation(f, &peers[0].config.credential, cid, "large-future-offer").await;
    let future_handle = handle(&answer);
    let original_handle = peers[0].publish_handle;
    assert_ne!(future_handle, original_handle);
    wait_slots(f, 100).await;
    peers[0].publish_handle = future_handle;
    let retained = (1..N).filter(|i| i % 3 != 0).collect::<BTreeSet<_>>();
    assert_eq!(retained.len(), 33);
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let startup = media::startup(
        peers,
        0,
        &retained,
        &db,
        &[connection.into()],
        "later-publication-retained33",
        ready_deadline,
    )
    .await;
    let source_events = source_events(
        &mut peers[0],
        cid,
        connection,
        future_handle,
        ready_deadline,
    )
    .await;
    let publication = media::probe(peers, 0, &retained, "later-publication-retained33").await;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_connection_flow_facts WHERE connection_id=?1",
            [connection],
            |r| r.get::<_, usize>(0),
        )
        .unwrap(),
        1,
    );
    let original_connection = std::mem::replace(&mut peers[0].connection, connection.into());
    let revision = stream_revision(f, connection);
    for _ in 0..2 {
        events::subscription(
            &mut peers[0],
            cid,
            "large-future-end",
            revision,
            "connection.end",
            json!({"reason_code":200,"reason":"later source withdrawn"}),
        )
        .await;
    }
    peers[0].connection = original_connection;
    peers[0].publish_handle = original_handle;
    wait_slots(f, 99).await;
    // The native end receipt proves actual absence. Its owned original-offer
    // cleanup settles durable retirement independently, without redial/replay.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let state: String = loop {
        let state: String = db
            .query_row(
                "SELECT state FROM connections WHERE id=?1",
                [connection],
                |r| r.get(0),
            )
            .unwrap();
        if state == "retired" {
            break state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "later source remains {state}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let original = media::probe(
        peers,
        0,
        &(1..N).collect(),
        "later-end-preserves-original49",
    )
    .await;
    json!({"answer":answer.original(),"publish_handle":future_handle,"source_events":source_events,"startup":startup,"publication":publication,"after_end":original,"peak_connections":100,"connections_after_end":99,"state_after_end":state})
}

async fn source_events(
    peer: &mut Peer,
    cid: &str,
    connection: &str,
    handle: u16,
    deadline: tokio::time::Instant,
) -> Vec<Value> {
    let mut ready = false;
    let mut opened = false;
    let mut frames = Vec::new();
    while !ready || !opened {
        assert!(
            tokio::time::Instant::now() < deadline,
            "later publication readiness deadline"
        );
        let frame = if let Some(frame) = peer.events.pop_front() {
            frame
        } else {
            tokio::time::timeout_at(deadline, peer.client.receive())
                .await
                .unwrap()
                .unwrap()
        };
        if events::late_self_join(peer, cid, &frame) {
            continue;
        }
        match frame.kind() {
            "connection.ready" => {
                assert_eq!(frame.original()["connid"], connection);
                assert!(!ready);
                ready = true;
            }
            "stream.opened" => {
                assert_eq!(frame.original()["connid"], connection);
                assert!(ready && !opened);
                assert_eq!(
                    frame.original()["payload"]["stream"]["strm_id"],
                    "large-future-voice"
                );
                assert_eq!(
                    frame.original()["payload"]["stream"]["stream_local_id"],
                    handle
                );
                opened = true;
            }
            "connection.quality" | "recording.vcon-ready" => {}
            kind => panic!(
                "unexpected later-publication event{kind}: {:?}",
                frame.original()
            ),
        }
        assert_eq!(frame.original()["cid"], cid);
        assert_eq!(frame.original()["sid"], "native-session");
        frames.push(frame.original().clone());
    }
    frames
}
