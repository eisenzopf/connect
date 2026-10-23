//! Public retained native control after actual movement to a separate SIP account.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_stream_controls_use_actual_route_and_logical_participant_after_movement(
) {
    telephone_case(Stop::StreamControl, true).await;
}
pub(super) async fn controls(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    done: &OperationRecord,
) {
    let stream = done.result.as_ref().unwrap()["readiness"]["streams"][0]
        .as_str()
        .unwrap();
    let pid = peers[1].participant.clone();
    for (operation, selection) in [
        ("telephone-select-named-stream", json!({"strm_id":stream})),
        (
            "telephone-select-logical-participant",
            json!({"from_participant":pid,"kinds":["audio"]}),
        ),
    ] {
        let revision = stream_revision(f, "codec-source");
        stream_control(
            &mut peers[0].client,
            cid,
            "codec-source",
            operation,
            revision,
            "stream.subscribe",
            json!({"subscriptions":[selection]}),
        )
        .await;
        sip_native_duplex(phone, peers, payload, operation).await;
    }
    unsubscribe_and_resume(f, cid, peers, phone, payload, stream).await;
}

async fn unsubscribe_and_resume(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    stream: &str,
) {
    let revision = stream_revision(f, "codec-source");
    stream_control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "telephone-exclude-current-stream",
        revision,
        "stream.unsubscribe",
        json!({"strm_ids":[stream]}),
    )
    .await;
    // The explicit stream exclusion also overrides the retained Participant intent.
    tokio::join!(phone.rtp(payload), private_input_isolation(peers));
    let revision = stream_revision(f, "codec-source");
    stream_control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "telephone-resume-current-stream",
        revision,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":stream}]}),
    )
    .await;
    sip_native_duplex(phone, peers, payload, "telephone-after-exclusion-resume").await;
}
