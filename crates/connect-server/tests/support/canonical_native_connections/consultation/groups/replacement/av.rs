//! Signed native AV resources and explicit post-commit video gates, not quality qualification.
use super::*;
#[path = "av/transfer.rs"]
pub(super) mod transfer;

#[tokio::test]
async fn production_replacement_av_retains_resources_and_opens_video_only_after_known_audio_commit()
{
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        true,
        ActivationAudit::None,
    )
    .await;
}

pub(super) async fn setup() -> (codecs::Running, u16) {
    let f = Fixture::with_native_end_scope("EdDSA", 16).await;
    ready_endpoint(&f).await;
    let config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        99,
        true,
        vec!["audio".into(), "video".into()],
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"]}),
    )
    .await;
    let t = negotiate_av(&mut target, &cid, "codec-target-offer", "codec-target", json!([
        {"id":"codec-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},
        {"id":"codec-camera-target","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}
    ])).await;
    let s = negotiate_av(&mut source, &cid, "codec-source-offer", "codec-source", json!([
        {"id":"codec-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},
        {"id":"codec-camera-source","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]}
    ])).await;
    wait_native_operation(&f, &f.client.credential, &cid, "codec-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "codec-source-offer").await;
    stream_control(
        &mut target,
        &cid,
        "codec-target",
        "codec-select-voice",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"}]}),
    )
    .await;
    stream_control(
        &mut source,
        &cid,
        "codec-source",
        "codec-select-return",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-monitor"}]}),
    )
    .await;
    (
        codecs::Running {
            f,
            clients: vec![source, target],
            cid,
            sh: stream_handle(&s, "codec-voice"),
            th: stream_handle(&t, "codec-monitor"),
        },
        stream_handle(&s, "codec-camera-source"),
    )
}

pub(super) fn stream_handle(frame: &Envelope, id: &str) -> u16 {
    let streams = frame.original()["payload"]["substrate_setup"]["streams"]
        .as_array()
        .unwrap();
    let matching = streams
        .iter()
        .filter(|s| s["strm_id"] == id)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    u16::try_from(matching[0]["stream_local_id"].as_u64().unwrap()).unwrap()
}

async fn control(
    client: &mut CanonicalClient,
    cid: &str,
    connection: &str,
    operation: &str,
    revision: u64,
    kind: &str,
    input: Value,
) -> (Value, Option<Envelope>) {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(connection.into()),
        operation_id: Some(operation.into()),
        expected_revision: Some(revision),
    };
    let request = client.send_value(kind, input, scope.clone()).await.unwrap();
    let mut callback = None;
    let mut result = None;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(8), client.receive_control())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            frame.scope(connect_client::canonical::Scope::Conversation),
            Some(cid)
        );
        assert_eq!(
            frame.scope(connect_client::canonical::Scope::Session),
            Some("native-session")
        );
        assert_eq!(
            frame.scope(connect_client::canonical::Scope::Connection),
            Some(connection)
        );
        assert_eq!(
            frame.scope(connect_client::canonical::Scope::Operation),
            Some(operation)
        );
        match frame.kind() {
            "connection.offer" => {
                let p = &frame.original()["payload"];
                let streams = p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s| {
                    assert!(matches!(s["kind"].as_str(), Some("audio" | "video")));
                    json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){
                        "sendonly"=>"recvonly", "recvonly"=>"sendonly", _=>"sendrecv"}, "codec":s["codec"]})
                }).collect::<Vec<_>>();
                client.answer_connection_offer(&frame, serde_json::from_value(json!({
                    "by_participant":p["by_participant"], "substrate":"quic", "capabilities":av_capabilities(),
                    "streams_answered":streams, "substrate_setup":p["substrate_setup"]})).unwrap(), &scope).await.unwrap();
                callback = Some(frame);
            }
            "operation.updated" => {
                assert_eq!(frame.original()["in_reply_to"], request);
                assert_eq!(
                    frame.original()["payload"]["state"],
                    "completed",
                    "{:?}",
                    frame.original()
                );
                assert!(result
                    .replace(frame.original()["payload"].clone())
                    .is_none());
            }
            "ack" => {
                assert_eq!(frame.original()["in_reply_to"], request);
                return (result.unwrap(), callback);
            }
            _ => panic!(
                "unexpected signed AV control result: {:?}",
                frame.original()
            ),
        }
    }
}

async fn silent_video(
    source: &CanonicalClient,
    target: &mut CanonicalClient,
    source_handle: u16,
    target_handle: u16,
) {
    use rvoip_uctp::substrate::{observe_rtp_datagram, pack, MediaDatagram};
    let frame = include_bytes!("../../../../../fixtures/gate6-vp8-black-640x360.vp8");
    let sending = async {
        for sequence in 1u16..=5 {
            let mut rtp = vec![0x80, 0x80 | 96];
            rtp.extend(sequence.to_be_bytes());
            rtp.extend((u32::from(sequence) * 9000).to_be_bytes());
            rtp.extend(0x6476_7001u32.to_be_bytes());
            rtp.push(0x10);
            rtp.extend(frame);
            source
                .send_native_media_datagram(&pack(&MediaDatagram {
                    flags: 0,
                    stream_local_id: source_handle,
                    seq: u32::from(sequence),
                    payload: rtp.into(),
                }))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    };
    let receiving = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            let bytes = target.receive_native_media_datagram().await.unwrap();
            let packet = observe_rtp_datagram(&bytes).unwrap();
            assert_ne!(
                packet.stream_local_id, target_handle,
                "muted post-commit video became public"
            );
        }
    });
    let (_, silence) = tokio::join!(sending, receiving);
    assert!(silence.is_err());
}

pub(super) async fn after_commit(
    f: &Fixture,
    peers: &mut [Peer],
    cid: &str,
    candidate: &str,
    source_video_handle: u16,
    target_video_handle: u16,
) {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let text: String = c
        .query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [candidate],
            |r| r.get(0),
        )
        .unwrap();
    let ready: Value = serde_json::from_str(&text).unwrap();
    let mut streams = ready["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    streams.sort();
    assert_eq!(streams, vec!["replacement-camera", "replacement-voice"]);
    drop(c);
    let (_, callback) = control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "av-select-replacement",
        stream_revision(f, "codec-source"),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"replacement-voice"},{"strm_id":"replacement-camera"}]}),
    )
    .await;
    let receive_camera = stream_handle(&callback.unwrap(), "replacement-camera");
    let (retained, rest) = peers.split_at_mut(1);
    silent_video(
        &rest[1].client,
        &mut retained[0].client,
        target_video_handle,
        receive_camera,
    )
    .await;
    let revision = stream_revision(f, candidate);
    let input = json!({"action":"unmute","streams":["replacement-camera"]});
    let (opened, _) = control(
        &mut peers[2].client,
        cid,
        candidate,
        "av-open-replacement-camera",
        revision,
        "connection.update",
        input.clone(),
    )
    .await;
    assert_eq!(opened["result"]["resources_changed"], false);
    assert_eq!(opened["result"]["gate_streams"][0]["source_muted"], false);
    let (replay, _) = control(
        &mut peers[2].client,
        cid,
        candidate,
        "av-open-replacement-camera",
        revision,
        "connection.update",
        input,
    )
    .await;
    assert_eq!(replay, opened);
    let (retained, rest) = peers.split_at_mut(1);
    assert_vp8_routes_from(
        &rest[1].client,
        &mut retained[0].client,
        target_video_handle,
        receive_camera,
        10,
    )
    .await;
    let (_, callback) = control(
        &mut peers[2].client,
        cid,
        candidate,
        "av-select-retained-camera",
        stream_revision(f, candidate),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"},{"strm_id":"codec-camera-source"}]}),
    )
    .await;
    let receive_remote = stream_handle(&callback.unwrap(), "codec-camera-source");
    let (retained, rest) = peers.split_at_mut(1);
    assert_vp8_routes_from(
        &retained[0].client,
        &mut rest[1].client,
        source_video_handle,
        receive_remote,
        50,
    )
    .await;
    // Existing later probes deliberately accept only their audio hop. Retire
    // these separately exercised video subscriptions and demultiplex their
    // remaining original datagrams before returning to that stricter probe.
    control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "av-return-retained-audio-only",
        stream_revision(f, "codec-source"),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"replacement-voice"}]}),
    )
    .await;
    control(
        &mut peers[2].client,
        cid,
        candidate,
        "av-return-target-audio-only",
        stream_revision(f, candidate),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"}]}),
    )
    .await;
    drain_exercised_video(&mut peers[0], receive_camera).await;
    drain_exercised_video(&mut peers[2], receive_remote).await;
    eprintln!("signed AV replacement: exact resources, video muted through commit, original explicit unmute replay, VP8 both directions; no full quality qualification");
}

async fn drain_exercised_video(peer: &mut Peer, video_handle: u16) {
    use rvoip_uctp::substrate::observe_rtp_datagram;
    let until = tokio::time::Instant::now() + Duration::from_millis(300);
    let mut observed = 0;
    loop {
        let bytes =
            match tokio::time::timeout_at(until, peer.client.receive_native_media_datagram()).await
            {
                Ok(bytes) => bytes.unwrap(),
                Err(_) => break,
            };
        observed += 1;
        assert!(observed <= 256, "bounded AV-to-audio fixture drain");
        let packet = observe_rtp_datagram(&bytes).unwrap();
        assert!(
            packet.stream_local_id == peer.handle || packet.stream_local_id == video_handle,
            "an unexercised stream crossed the signed AV fixture"
        );
    }
}
