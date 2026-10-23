//! Session preferences affect real initial negotiation, without granting codecs.
use super::*;
#[tokio::test]
async fn production_session_codec_preferences_select_current_policy_preserve_fallback_and_original_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        for mode in ["preferred", "latest", "accepted-fallback"] {
            scenario(algorithm, mode).await;
        }
    }
}
async fn scenario(algorithm: &str, mode: &str) {
    let f = Fixture::with_native_stream_scope(algorithm, 12).await;
    ready_endpoint(&f).await;
    let config = party_with_streams(&f, &f.peer.identity_id, 105, true);
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut source = CanonicalClient::open(config.clone()).await.unwrap();
    let both = json!({"audio":["g.711-mu","g.711-a"]});
    let cid = session_with_acceptance(
        &f,
        &mut target,
        "voice",
        both.clone(),
        if mode == "accepted-fallback" {
            json!({"audio":["g.711-mu"]})
        } else {
            both
        },
    )
    .await;
    let first = Request {
        operation_id: "session-prefer-pcma".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: Some(1),
        action: Action::Renegotiate {
            codec_preferences: vec!["g.711-a".into()],
        },
    };
    let original = target.session_command(first.clone()).await.unwrap();
    if mode == "latest" {
        target
            .session_command(Request {
                operation_id: "session-prefer-pcmu".into(),
                expected_revision: Some(2),
                action: Action::Renegotiate {
                    codec_preferences: vec!["g.711-mu".into()],
                },
                ..first.clone()
            })
            .await
            .unwrap();
        assert_eq!(
            target.session_command(first.clone()).await.unwrap(),
            original
        );
    }
    let caps = json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}},{"name":"pcma","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
    let preferences = if mode == "latest" {
        json!(["g.711-a", "g.711-mu"])
    } else {
        json!(["g.711-mu", "g.711-a"])
    };
    let t=negotiate_media(&mut target,&cid,"policy-target-offer","policy-target",json!([{"id":"policy-monitor","kind":"audio","direction":"sendrecv","codec_preferences":preferences}]),caps.clone()).await;
    let s=negotiate_media(&mut source,&cid,"policy-source-offer","policy-source",json!([{"id":"policy-voice","kind":"audio","direction":"sendrecv","codec_preferences":preferences}]),caps).await;
    wait_native_operation(&f, &f.client.credential, &cid, "policy-target-offer").await;
    wait_native_operation(&f, &config.credential, &cid, "policy-source-offer").await;
    let target_codec = if mode == "latest" { "pcmu" } else { "pcma" };
    let source_codec = if mode == "preferred" { "pcma" } else { "pcmu" };
    for (reply, codec) in [(&t, target_codec), (&s, source_codec)] {
        assert_eq!(
            reply.original()["payload"]["streams_answered"][0]["codec"]["name"],
            codec,
            "{algorithm}/{mode}"
        );
    }
    // A preference never narrows or rewrites the advertised capability grant.
    assert_eq!(
        t.original()["payload"]["capabilities"]["audio_codecs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let th = t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let sh = s.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    stream_control(
        &mut target,
        &cid,
        "policy-target",
        "policy-select-source",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"policy-voice"}]}),
    )
    .await;
    stream_control(
        &mut source,
        &cid,
        "policy-source",
        "policy-select-target",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"policy-monitor"}]}),
    )
    .await;
    let mut clients = vec![source, target];
    let sp = if source_codec == "pcma" { 8 } else { 0 };
    let tp = if target_codec == "pcma" { 8 } else { 0 };
    let (left, right) = clients.split_at_mut(1);
    codecs::decoded_audio(&left[0], &mut right[0], sh, sp, th, tp, 33000).await;
    let (left, right) = clients.split_at_mut(1);
    codecs::decoded_audio(&right[0], &mut left[0], th, tp, sh, sp, 33200).await;
    assert_eq!(
        clients[1].session_command(first.clone()).await.unwrap(),
        original
    );
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let (current,revision):(String,u64)=c.query_row("SELECT sc.codec_preferences_json,s.topology_revision FROM session_control sc JOIN sessions s ON s.id=sc.session_id WHERE s.id='native-session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mut query = c.prepare("SELECT offer_json FROM connection_offers WHERE connection_id IN ('policy-target','policy-source')").unwrap();
    let retained = query
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 2);
    for payload in retained {
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap()["streams_offered"][0]
                ["codec_preferences"],
            preferences
        );
    }
    drop(query);
    assert_eq!(
        serde_json::from_str::<Value>(&current).unwrap(),
        if mode == "latest" {
            json!(["g.711-mu"])
        } else {
            json!(["g.711-a"])
        }
    );
    drop(c);
    // Live Session-wide switching remains explicit unfinished coordinator work.
    let denied = clients[1]
        .session_command(Request {
            operation_id: "live-session-codec-unavailable".into(),
            expected_revision: Some(revision),
            action: Action::Renegotiate {
                codec_preferences: vec!["g.711-mu".into()],
            },
            ..first
        })
        .await
        .unwrap_err();
    let ClientError::Application(error) = denied else {
        panic!("unexpected refusal: {denied:?}")
    };
    assert_eq!(error.code, "unsupported-command");
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let after:(String,u64)=c.query_row("SELECT sc.codec_preferences_json,s.topology_revision FROM session_control sc JOIN sessions s ON s.id=sc.session_id WHERE s.id='native-session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(after, (current.clone(), revision));
    drop(c);
    let (left, right) = clients.split_at_mut(1);
    codecs::decoded_audio(&left[0], &mut right[0], sh, sp, th, tp, 33400).await;
    if let Some(path) = std::env::var_os("CONNECT_G6_SESSION_CODECS_SAMPLES") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file,"{}",json!({"algorithm":algorithm,"mode":mode,"target":t.original(),"source":s.original(),"offered_preferences":preferences,"current_preferences":serde_json::from_str::<Value>(&current).unwrap()})).unwrap();
    }
    for client in clients {
        client.close();
    }
    f.server.shutdown().await.unwrap();
}
