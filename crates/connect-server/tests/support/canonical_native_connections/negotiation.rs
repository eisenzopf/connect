//! Original codec preference is constrained by current accepted capability.
use super::*;

#[tokio::test]
async fn production_native_initial_codec_selection_uses_offered_fallback_within_current_capabilities(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_stream_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let source_config = party_with_streams(&f, &f.peer.identity_id, 97, true);
        let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut source = CanonicalClient::open(source_config.clone()).await.unwrap();
        let cid = session_with_media(&f, &mut target, "voice", json!({"audio":["g.711-mu"]})).await;
        let room = target
            .list_conversations(Default::default())
            .await
            .unwrap()
            .conversations
            .into_iter()
            .find(|r| r.id == cid)
            .unwrap();
        let root = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let peer = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let caps = json!({"audio_codecs":[{"name":"pcma","params":{"sample_rate":8000,"channels":1}},{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
        let target_offer = json!({"by_participant":root,"substrate":"quic","capabilities":caps,"streams_offered":[{"id":"fallback-monitor","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-a","g.711-mu"]}]});
        let source_offer = json!({"by_participant":peer,"substrate":"quic","capabilities":caps,"streams_offered":[{"id":"fallback-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-a","g.711-mu"]}]});
        let t = negotiate_media_payload(
            &mut target,
            &cid,
            "fallback-target-offer",
            "fallback-target",
            target_offer.clone(),
        )
        .await;
        let p = negotiate_media_payload(
            &mut source,
            &cid,
            "fallback-source-offer",
            "fallback-source",
            source_offer.clone(),
        )
        .await;
        wait_native_operation(&f, &f.client.credential, &cid, "fallback-target-offer").await;
        wait_native_operation(&f, &source_config.credential, &cid, "fallback-source-offer").await;
        for (offer, answer) in [(&target_offer, &t), (&source_offer, &p)] {
            assert_eq!(
                answer.original()["payload"]["by_participant"],
                offer["by_participant"]
            );
            assert_eq!(answer.original()["payload"]["substrate"], "quic");
            assert_eq!(
                answer.original()["payload"]["streams_answered"][0]["codec"]["name"],
                "pcmu"
            );
            assert_eq!(
                answer.original()["payload"]["streams_answered"][0]["direction"],
                "sendrecv"
            );
            assert_eq!(
                answer.original()["payload"]["capabilities"]["audio_codecs"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                answer.original()["payload"]["capabilities"]["audio_codecs"][0]["name"],
                "pcmu"
            );
        }
        let th = t.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        let sh = p.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        stream_control(
            &mut target,
            &cid,
            "fallback-target",
            "fallback-subscribe",
            1,
            "stream.subscribe",
            json!({"by_participant":root,"subscriptions":[{"strm_id":"fallback-voice"}]}),
        )
        .await;
        stream_control(
            &mut source,
            &cid,
            "fallback-source",
            "fallback-return",
            1,
            "stream.subscribe",
            json!({"by_participant":peer,"subscriptions":[{"strm_id":"fallback-monitor"}]}),
        )
        .await;
        // A PCMA-only offer must not silently borrow the usable PCMU format.
        let id = target.send_value("connection.offer", json!({"by_participant":root,"substrate":"quic","capabilities":caps,"streams_offered":[{"id":"unsupported-only","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-a"]}]}), CommandScope { conversation_id: Some(cid.clone()), session_id: Some("native-session".into()), connection_id: Some("fallback-refused".into()), operation_id: Some("fallback-refused".into()), expected_revision: Some(0) }).await.unwrap();
        let failed = target.receive_control().await.unwrap();
        assert_eq!(failed.original()["in_reply_to"], id);
        assert_eq!(failed.kind(), "operation.updated");
        assert_eq!(failed.original()["payload"]["state"], "failed");
        let ack = target.receive_control().await.unwrap();
        assert_eq!(ack.kind(), "ack");
        assert_eq!(ack.original()["in_reply_to"], id);
        assert_eq!(rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='fallback-refused'", [], |r| r.get::<_,u64>(0)).unwrap(), 0);
        let mut authority_refusals = Vec::new();
        for (case, field, value, code, category) in [
            (
                "foreign-participant",
                "by_participant",
                json!(peer),
                403,
                "auth",
            ),
            (
                "foreign-substrate",
                "substrate",
                json!("webtransport"),
                501,
                "unsupported",
            ),
        ] {
            let mut offered = target_offer.clone();
            offered[field] = value;
            let operation = format!("fallback-{case}");
            let id = target
                .send_value(
                    "connection.offer",
                    offered.clone(),
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        connection_id: Some(operation.clone()),
                        operation_id: Some(operation.clone()),
                        expected_revision: Some(0),
                    },
                )
                .await
                .unwrap();
            let refusal = target.receive_control().await.unwrap();
            assert_eq!(refusal.original()["in_reply_to"], id);
            assert_eq!(refusal.kind(), "error", "{case}: {:?}", refusal.original());
            assert_eq!(refusal.original()["payload"]["code"], code);
            assert_eq!(refusal.original()["payload"]["category"], category);
            let barrier = target.keepalive().await.unwrap();
            loop {
                let frame = target.receive_control().await.unwrap();
                assert_eq!(frame.kind(), "ack", "{case}: {:?}", frame.original());
                if frame.original()["in_reply_to"] == barrier {
                    break;
                }
                assert_eq!(frame.original()["in_reply_to"], id);
            }
            let effects: u64 = rusqlite::Connection::open(&f.config.database).unwrap()
                .query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id=?1", [&operation], |r| r.get(0)).unwrap();
            assert_eq!(effects, 0, "{case} must fail before an effect");
            authority_refusals.push(json!({"case":case,"request_id":id,"request_payload":offered,"refusal":refusal.original(),"effects":effects}));
        }
        let (mut source_side, mut target_side) = (source, target);
        codecs::decoded_audio(&source_side, &mut target_side, sh, 0, th, 0, 40000).await;
        codecs::decoded_audio(&target_side, &mut source_side, th, 0, sh, 0, 40200).await;
        assert_eq!(
            f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .connections,
            2
        );
        if let Some(directory) = std::env::var_os("CONNECT_G6_NEGOTIATION_SAMPLES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{algorithm}.json")), serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"current_session_audio":["g.711-mu"],"target_offer":target_offer,"source_offer":source_offer,"target_answer":t.original(),"source_answer":p.original(),"refused":failed.original(),"authority_refusals":authority_refusals,"bidirectional_decoded_audio":true})).unwrap()).unwrap();
        }
        source_side.close();
        target_side.close();
        f.server.shutdown().await.unwrap();
    }
}
