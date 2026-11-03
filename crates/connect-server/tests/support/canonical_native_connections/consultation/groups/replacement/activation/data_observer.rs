//! Reject an actual pending Data observer's scope after durable physical capture.
//! The shared failure audit requires known movement, no Ready, exact target
//! retirement, retained binding preservation and unchanged original replay.
use super::*;

#[tokio::test]
async fn production_native_data_observer_scope_failure_stops_known_move_without_ready_or_replay() {
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
        false,
        ActivationAudit::RefuseDataObserver,
    )
    .await;
}

pub(in super::super) fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"data_protocols":["json"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}

pub(in super::super) async fn setup() -> codecs::Running {
    let f = Fixture::with_native_end_scope("EdDSA", 16).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "data".into()]).await;
    let config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        99,
        true,
        vec!["audio".into(), "data".into()],
    );
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice",
        json!({"audio":["g.711-mu"],"data":["reliable-ordered"]}),
    )
    .await;
    let t = negotiate_media(&mut target, &cid, "codec-target-offer", "codec-target", json!([{"id":"codec-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]), capabilities()).await;
    let s = negotiate_media(&mut source, &cid, "codec-source-offer", "codec-source", json!([{"id":"codec-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]), capabilities()).await;
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
    codecs::Running {
        f,
        clients: vec![source, target],
        cid,
        sh: av::stream_handle(&s, "codec-voice"),
        th: av::stream_handle(&t, "codec-monitor"),
    }
}
