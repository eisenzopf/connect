//! Durable Session role changes on the actual independent SIP source.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_role_observer_speaker_observer_then_native_move_preserves_role_and_receive(
) {
    telephone_case(Stop::ReturnObserver, true).await;
}
pub(super) async fn cycle(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    controller: &mut CanonicalHttpClient,
) {
    let participant = peers[1].participant.clone();
    for (index, role) in ["observer", "speaker", "observer"].into_iter().enumerate() {
        let before = f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections;
        controller
            .request_value(
                "session.update",
                json!({"kind":"role-change","details":{"participant_id":participant,"role":role}}),
                CommandScope {
                    conversation_id: Some(cid.into()),
                    session_id: Some("native-session".into()),
                    operation_id: Some(format!("telephone-role-{index}")),
                    expected_revision: Some(revision(f)),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .ack()
            .unwrap();
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        while f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .role_projections
            == before
        {
            assert!(
                tokio::time::Instant::now() < end,
                "telephone durable {role} never reached the actual Core participant role"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if peers.len() > 2 {
            probe(
                peers,
                2,
                if role == "observer" { &[] } else { &[0] },
                "telephone-role-sibling-respects-shared-role",
            )
            .await;
        }
        if role == "observer" {
            tokio::join!(phone.rtp(payload), private_input_isolation(peers));
            let (_, marked) = tokio::join!(
                probe(
                    peers,
                    0,
                    &[],
                    "telephone-observer-can-receive-retained-native"
                ),
                sip_audio(phone, payload, Duration::from_millis(2600))
            );
            assert!(
                marked > 10,
                "Observer telephone lost current receive permission"
            );
        } else {
            sip_native_duplex(
                phone,
                peers,
                payload,
                "telephone-speaker-restored-by-durable-role",
            )
            .await;
        }
    }
}

pub(super) async fn sibling(f: &Fixture, cid: &str, peers: &mut Vec<Peer>) {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("stream.subscribe".into());
    let config = party_with_scope(f, &f.client.identity_id, 392, vec!["audio".into()], scope);
    let mut client = CanonicalClient::open(config.clone()).await.unwrap();
    let answer=negotiate_media(&mut client,cid,"telephone-role-sibling-offer","telephone-role-sibling",json!([{"id":"telephone-role-sibling-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]),capabilities()).await;
    wait_native_operation(f, &config.credential, cid, "telephone-role-sibling-offer").await;
    peers.push(Peer {
        client,
        config: Some(config),
        identity: f.client.identity_id.clone(),
        participant: peers[1].participant.clone(),
        id: "telephone-role-sibling".into(),
        handle: handle(&answer),
        clock: tokio::time::Instant::now(),
        sequence: 1,
        live: true,
    });
    let participant = peers[1].participant.clone();
    stream_control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "telephone-role-logical-member-audience",
        stream_revision(f, "codec-source"),
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":participant,"kinds":["audio"]}]}),
    )
    .await;
}
