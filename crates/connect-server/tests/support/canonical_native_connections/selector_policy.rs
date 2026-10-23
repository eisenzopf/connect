//! Signed production refusals and heterogeneous wildcard routing.
use super::*;
#[tokio::test]
async fn production_native_heterogeneous_wildcard_preserves_audio_and_definitive_refusals() {
    for algorithm in ["EdDSA", "ES256"] {
        heterogeneous(algorithm).await;
    }
}
async fn refused(
    client: &mut CanonicalClient,
    cid: &str,
    operation: &str,
    selectors: Value,
    code: u64,
) -> Value {
    let request = client
        .send_value(
            "stream.subscribe",
            json!({"subscriptions":selectors}),
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some("hetero-target".into()),
                operation_id: Some(operation.into()),
                expected_revision: Some(1),
            },
        )
        .await
        .unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(8), client.receive_control())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        changed.kind(),
        "operation.updated",
        "{:?}",
        changed.original()
    );
    assert_eq!(changed.original()["in_reply_to"], request);
    assert_eq!(changed.original()["payload"]["state"], "failed");
    let error = client.receive_control().await.unwrap();
    assert_eq!(error.kind(), "error", "{:?}", error.original());
    assert_eq!(error.original()["in_reply_to"], request);
    assert_eq!(error.original()["payload"]["code"], code);
    if let Some(path) = std::env::var_os("CONNECT_G6_SUBSCRIPTION_SAMPLES") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file,"{}",json!({"operation_id":operation,"request_id":request,"operation":changed.original(),"error":error.original()})).unwrap();
    }
    changed.original()["payload"].clone()
}
async fn heterogeneous(algorithm: &str) {
    let f = Fixture::with_native_stream_scope(algorithm, 12).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "video".into()]).await;
    let config = party_with_capabilities(&f, &f.peer.identity_id, 103, false, vec!["audio".into()]);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_acceptance(
        &f,
        &mut target,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"]}),
        json!({"audio":["g.711-mu"]}),
    )
    .await;
    let t=negotiate_av(&mut target,&cid,"hetero-target-offer","hetero-target",json!([{"id":"hetero-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}])).await;
    let s = negotiate_named(
        &mut source,
        &cid,
        "hetero-source-offer",
        "hetero-source",
        "hetero-voice",
    )
    .await;
    wait_native_operation(&f, &f.client.credential, &cid, "hetero-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "hetero-source-offer").await;
    let pid = s.original()["payload"]["by_participant"].clone();
    for (id, selectors, code) in [
        (
            "hetero-missing-stream",
            json!([{"strm_id":"hetero-voice"},{"strm_id":"missing-source"}]),
            404,
        ),
        (
            "hetero-missing-participant",
            json!([{"from_participant":"missing-participant"}]),
            404,
        ),
        (
            "hetero-incompatible-video",
            json!([{"from_participant":pid,"kinds":["video"]}]),
            488,
        ),
    ] {
        let original = refused(&mut target, &cid, id, selectors.clone(), code).await;
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let observed = http
            .operation_get(
                id,
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            observed.state,
            rvoip_uctp::v2::payloads::OperationState::Failed
        );
        assert_eq!(observed.problem.as_ref(), Some(&original["problem"]));
        assert_eq!(
            observed.record_id.as_ref().unwrap().as_str(),
            original["record_id"].as_str().unwrap()
        );

        assert_eq!(
            refused(&mut target, &cid, id, selectors, code).await,
            original,
            "failed replay retains the same durable observation"
        );
    }
    let sh = s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let th = t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut clients = vec![source, target];
    assert_audio_is_silent_after_unsubscribe(&mut clients, sh, th).await;
    assert!(stream_control(
        &mut clients[1],
        &cid,
        "hetero-target",
        "hetero-wildcard",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":pid}]})
    )
    .await
    .is_none());
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 31000).await;
    // Replaying a refused selector after a later successful revision cannot
    // install the old request or disturb the current physical audio route.
    refused(
        &mut clients[1],
        &cid,
        "hetero-incompatible-video",
        json!([{"from_participant":pid,"kinds":["video"]}]),
        488,
    )
    .await;
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 31200).await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let effects:u64=c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id IN ('hetero-missing-stream','hetero-missing-participant','hetero-incompatible-video')",[],|r|r.get(0)).unwrap();
    assert_eq!(effects, 0);
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE operation_id='hetero-wildcard'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "completed");
    drop(c);
    for client in clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_native_disjoint_data_formats_refuse_488_without_allocating_a_route() {
    let f = Fixture::with_native_stream_scope("EdDSA", 12).await;
    ready_endpoint_with_capabilities(&f, vec!["data".into()]).await;
    let config = party_with_capabilities(&f, &f.peer.identity_id, 104, false, vec!["data".into()]);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "text-chat",
        json!({"data":["reliable-ordered"]}),
    )
    .await;
    let caps = |protocol: &str| json!({"data_protocols":[protocol],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
    negotiate_media(&mut target,&cid,"hetero-target-offer","hetero-target",json!([{"id":"hetero-text","kind":"data","direction":"recvonly","codec_preferences":["text"]}]),caps("text")).await;
    negotiate_media(&mut source,&cid,"hetero-source-offer","hetero-source",json!([{"id":"hetero-json","kind":"data","direction":"sendonly","codec_preferences":["json"]}]),caps("json")).await;
    wait_native_operation(&f, &f.client.credential, &cid, "hetero-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "hetero-source-offer").await;
    refused(
        &mut target,
        &cid,
        "hetero-no-common-format",
        json!([{"strm_id":"hetero-json"}]),
        488,
    )
    .await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let effects:u64=c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='hetero-no-common-format'",[],|r|r.get(0)).unwrap();
    assert_eq!(effects, 0);
    drop(c);
    source.close();
    target.close();
    f.server.shutdown().await.unwrap();
}
