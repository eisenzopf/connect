//! Actual initial and live Session codec policy, signed peer confirmation,
//! retained explicit routes, independent decoding and immutable replay.
use super::*;
#[tokio::test]
async fn production_session_codec_preferences_select_current_policy_switch_live_peers_preserve_fallback_routes_and_replay(
) {
    for algorithm in ["EdDSA", "ES256"] {
        for mode in ["preferred", "latest", "accepted-fallback"] {
            scenario(algorithm, mode).await;
        }
    }
}
#[tokio::test]
async fn production_session_codec_captured_success_retries_storage_without_another_offer() {
    for algorithm in ["EdDSA", "ES256"] {
        scenario(algorithm, "recording-recovery").await;
    }
}
#[tokio::test]
async fn production_session_codec_bad_confirmation_retains_partial_unknown_and_never_resubmits() {
    for algorithm in ["EdDSA", "ES256"] {
        scenario(algorithm, "bad-confirmation").await;
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
    let source_codec = if matches!(
        mode,
        "preferred" | "recording-recovery" | "bad-confirmation"
    ) {
        "pcma"
    } else {
        "pcmu"
    };
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
    let (current,mut revision):(String,u64)=c.query_row("SELECT sc.codec_preferences_json,s.topology_revision FROM session_control sc JOIN sessions s ON s.id=sc.session_id WHERE s.id='native-session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
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
    if mode == "latest" {
        let participant = t.original()["payload"]["by_participant"]
            .as_str()
            .unwrap()
            .to_owned();
        let observer = Request {
            operation_id: "session-observer".into(),
            expected_revision: Some(revision),
            action: Action::Role {
                participant_id: participant.clone(),
                role: "observer".into(),
            },
            ..first.clone()
        };
        let old_role = clients[1].session_command(observer.clone()).await.unwrap();
        clients[1]
            .session_command(Request {
                operation_id: "session-agent".into(),
                expected_revision: Some(revision + 1),
                action: Action::Role {
                    participant_id: participant.clone(),
                    role: "agent".into(),
                },
                ..first.clone()
            })
            .await
            .unwrap();
        assert_eq!(
            clients[1].session_command(observer).await.unwrap(),
            old_role
        );
        revision += 2;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let role:String=db.query_row("SELECT role FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&participant],|r|r.get(0)).unwrap();
        assert_eq!(role, "agent");
        drop(db);
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let next = f
            .server
            .store
            .issue(
                &actor.identity_id,
                actor.scope.clone(),
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        dtmf::bind_fixture_successor(&f, &f.client, &next.id, "bind-session-codec-successor");
        let authentication = clients[1].refresh_credential(next.token).await.unwrap();
        assert_eq!(authentication.identity_id, actor.identity_id);
    }
    // Switch the policy while both physical Connections and their independent
    // explicit subscriptions are live. A peer with an accepted PCMU-only
    // ceiling keeps its compatible binding and emits no codec reoffer.
    let desired = if target_codec == "pcma" {
        "pcmu"
    } else {
        "pcma"
    };
    if mode == "recording-recovery" {
        rusqlite::Connection::open(&f.config.database).unwrap().execute_batch("CREATE TRIGGER reject_live_codec_recording BEFORE UPDATE OF state ON session_codec_targets WHEN NEW.state='updated' BEGIN SELECT RAISE(ABORT,'isolated recording failure'); END;").unwrap();
    }
    let live = Request {
        operation_id: "live-session-codec-switch".into(),
        expected_revision: Some(revision),
        action: Action::Renegotiate {
            codec_preferences: vec![if desired == "pcma" {
                "g.711-a"
            } else {
                "g.711-mu"
            }
            .into()],
        },
        ..first.clone()
    };
    let decision = if mode == "bad-confirmation" {
        // A separately scoped HTTP manager gives this batch a short concrete
        // deadline without expiring either physical peer or borrowing its IO
        // permission. An invalid signed reply must not complete the waiter.
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let manager = f
            .server
            .store
            .issue(
                &actor.identity_id,
                Scope {
                    operations: vec!["session.update".into(), "operation.get".into()],
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 6,
            )
            .unwrap();
        let mut config = canonical_http_config(&f);
        config.credential = manager.token;
        let mut manager = CanonicalHttpClient::open(config).await.unwrap();
        manager.session_command(live.clone()).await.unwrap()
    } else {
        clients[1].session_command(live.clone()).await.unwrap()
    };
    let Outcome::SessionControl(policy) = &decision else {
        panic!("wrong policy receipt")
    };
    let reference = policy
        .codec_reconciliation
        .as_ref()
        .expect("live reconciliation reference");
    let mut offers = Vec::new();
    let mut handles = [sh, th];
    let mut changed = 0;
    for (index, old_codec) in [source_codec, target_codec].into_iter().enumerate() {
        if old_codec == desired {
            continue;
        }
        let offer = tokio::time::timeout(Duration::from_secs(8), clients[index].receive_control())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(offer.kind(), "connection.offer", "{:?}", offer.original());
        assert!(offer
            .scope(rvoip_uctp::v2::Scope::Operation)
            .unwrap()
            .starts_with("scodec_child_"));
        offers.push(offer.original().clone());
        let stream = &offer.original()["payload"]["substrate_setup"]["streams"][0];
        assert_eq!(stream["codec"]["name"], desired, "{algorithm}/{mode}");
        let handle = stream["stream_local_id"].as_u64().unwrap() as u16;
        assert_ne!(handle, handles[index]);
        handles[index] = handle;
        answer(
            &mut clients[index],
            &offer,
            if index == 0 {
                s.original()["payload"]["capabilities"].clone()
            } else {
                t.original()["payload"]["capabilities"].clone()
            },
            mode == "bad-confirmation" && index == 0,
        )
        .await;
        changed += 1;
    }
    if mode == "bad-confirmation" {
        let partial = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let db = rusqlite::Connection::open(&f.config.database).unwrap();
                let facts:(u64,u64,u64) = db.query_row("SELECT (SELECT COUNT(*) FROM session_codec_targets WHERE state='updated'),(SELECT COUNT(*) FROM session_codec_targets WHERE state='unknown'),(SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id LIKE 'scodec_child_%')", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                drop(db);
                if facts.0 == 1 && facts.1 == 1 { break facts; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.unwrap();
        assert_eq!(partial, (1, 1, 2));
        let observed = clients[1]
            .operation_get(
                &reference.operation_id,
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    session_id: Some("native-session".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(observed.state).unwrap(),
            json!("unknown")
        );
        assert!(observed.result.is_none());
        write_worker_sample(
            algorithm,
            mode,
            &offers,
            &decision,
            serde_json::to_value(&observed).unwrap(),
        );
        assert_eq!(clients[1].session_command(live).await.unwrap(), decision);
        assert_eq!(clients[1].session_command(first).await.unwrap(), original);
        tokio::time::sleep(Duration::from_millis(250)).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let effects:u64=db.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id LIKE 'scodec_child_%'", [], |r|r.get(0)).unwrap();
        assert_eq!(effects, 2);
        drop(db);
        for client in clients {
            client.close();
        }
        f.server.shutdown().await.unwrap();
        return;
    }
    if mode == "recording-recovery" {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let (pending, effects):(u64,u64) = db.query_row("SELECT (SELECT COUNT(*) FROM session_codec_targets WHERE state='preparing'),(SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id LIKE 'scodec_child_%')", [], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((pending, effects), (2, 2));
        let observed = clients[1]
            .operation_get(
                &reference.operation_id,
                CommandScope {
                    conversation_id: Some(cid.clone()),
                    session_id: Some("native-session".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(observed.state).unwrap(),
            json!("preparing")
        );
        assert!(observed.result.is_none());
        let (left, right) = clients.split_at_mut(1);
        codecs::decoded_audio(&left[0], &mut right[0], handles[0], 0, handles[1], 0, 33300).await;
        db.execute_batch("DROP TRIGGER reject_live_codec_recording")
            .unwrap();
    }
    let settled = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let result = clients[1]
                .operation_get(
                    &reference.operation_id,
                    CommandScope {
                        conversation_id: Some(cid.clone()),
                        session_id: Some("native-session".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if serde_json::to_value(result.state).unwrap() == json!("completed") {
                break result;
            }
            assert!(result.problem.is_none(), "{result:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    write_worker_sample(
        algorithm,
        mode,
        &offers,
        &decision,
        serde_json::to_value(&settled).unwrap(),
    );
    let counts: connect_protocol::session_codecs::ReconciliationOutcome =
        serde_json::from_value(settled.result.unwrap()).unwrap();
    assert_eq!(counts.counts.targets, 2);
    assert_eq!(counts.counts.updated, changed);
    assert_eq!(counts.counts.unchanged, 2 - changed);
    assert_eq!(
        counts.counts.refused + counts.counts.unknown + counts.counts.unavailable,
        0
    );
    assert_eq!(clients[1].session_command(live).await.unwrap(), decision);
    assert_eq!(
        clients[1].session_command(first.clone()).await.unwrap(),
        original
    );
    let pt = if desired == "pcma" { 8 } else { 0 };
    let (left, right) = clients.split_at_mut(1);
    codecs::decoded_audio(
        &left[0],
        &mut right[0],
        handles[0],
        pt,
        handles[1],
        pt,
        33400,
    )
    .await;
    let (left, right) = clients.split_at_mut(1);
    codecs::decoded_audio(
        &right[0],
        &mut left[0],
        handles[1],
        pt,
        handles[0],
        pt,
        33600,
    )
    .await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let actual: String = c
        .query_row(
            "SELECT codec_preferences_json FROM session_control WHERE session_id='native-session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&actual).unwrap(),
        json!([if desired == "pcma" {
            "g.711-a"
        } else {
            "g.711-mu"
        }])
    );
    let effects:u64 = c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id LIKE 'scodec_child_%'", [], |r|r.get(0)).unwrap();
    assert_eq!(
        effects, 2,
        "one admitted attempt per Connection, including no-change"
    );
    drop(c);
    if let Some(path) = std::env::var_os("CONNECT_G6_SESSION_CODECS_SAMPLES")
        .filter(|_| matches!(mode, "preferred" | "latest" | "accepted-fallback"))
    {
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

fn write_worker_sample(
    algorithm: &str,
    mode: &str,
    offers: &[Value],
    decision: &Outcome,
    observed: Value,
) {
    if let Some(path) = std::env::var_os("CONNECT_G6_SESSION_CODEC_WORKER_SAMPLES") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file,"{}",json!({"algorithm":algorithm,"mode":mode,"offers":offers,"decision":decision,"observed":observed})).unwrap();
    }
}

async fn answer(client: &mut CanonicalClient, offer: &Envelope, capabilities: Value, stale: bool) {
    let p = &offer.original()["payload"];
    let streams=p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
    let mut payload = json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities,"streams_answered":streams,"substrate_setup":p["substrate_setup"]});
    if stale {
        payload["substrate_setup"]["revision"] = json!(999);
    }
    client
        .answer_connection_offer(
            offer,
            serde_json::from_value(payload).unwrap(),
            &CommandScope {
                conversation_id: offer
                    .scope(rvoip_uctp::v2::Scope::Conversation)
                    .map(str::to_owned),
                session_id: offer
                    .scope(rvoip_uctp::v2::Scope::Session)
                    .map(str::to_owned),
                connection_id: offer
                    .scope(rvoip_uctp::v2::Scope::Connection)
                    .map(str::to_owned),
                operation_id: offer
                    .scope(rvoip_uctp::v2::Scope::Operation)
                    .map(str::to_owned),
                expected_revision: offer.expected_revision(),
            },
        )
        .await
        .unwrap();
}
