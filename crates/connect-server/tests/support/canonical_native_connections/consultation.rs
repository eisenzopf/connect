//! Actual native route provenance at the consultation admission boundary.
use super::*;
#[path = "consultation/capabilities.rs"]
mod capabilities;
#[path = "consultation/record_erasure.rs"]
mod record_erasure;
#[path = "consultation/record_links.rs"]
mod record_links;
#[path = "consultation/sdk_controls.rs"]
mod sdk_controls;
use connect_protocol::transitions::{HoldTreatment, TransitionCommand, TransitionIntent};
#[tokio::test]
async fn production_consultation_admits_confirmed_native_ready_routes_without_claiming_media_switch(
) {
    let codecs::Running {
        f,
        mut clients,
        cid,
        sh,
        th,
    } = codecs::setup().await;
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let own = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let consultant = f.server.store.create_identity("consultant").unwrap().id;
    let added = clients[1]
        .participant_change(
            &cid,
            "consult-admit-member",
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":consultant,"kind":"human","role":"consultant"}),
            ),
        )
        .await
        .unwrap();
    let connect_client::membership::MembershipOutcome::Participant { participant, .. } = added
    else {
        panic!()
    };
    let request = TransitionCommand {
        payload_extensions: Default::default(),
        operation_id: "consult-native-ready".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some("codec-target".into()),
        expected_revision: 1,
        intent: TransitionIntent::Consult {
            retained_connid: "codec-source".into(),
            private_participants: vec![own, participant.participant_id],
            hold_treatment: HoldTreatment::Silence,
        },
    };
    let mut config = canonical_http_config(&f);
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant
        .operations
        .extend(["session.consult".into(), "operation.cancel".into()]);
    config.credential = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            grant,
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
        .token;
    let mut controller = CanonicalHttpClient::open(config).await.unwrap();
    let mut missing_permission = request.clone();
    missing_permission.operation_id = "consult-without-grant".into();
    assert!(clients[1].transition(missing_permission).await.is_err());
    let mut includes_remote = request.clone();
    includes_remote.operation_id = "consult-includes-remote".into();
    let TransitionIntent::Consult {
        private_participants,
        ..
    } = &mut includes_remote.intent
    else {
        panic!()
    };
    private_participants.push(
        room.participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone(),
    );
    assert!(controller.transition(includes_remote).await.is_err());
    let original = controller.transition(request.clone()).await.unwrap();
    assert_eq!(
        original.state,
        rvoip_uctp::v2::payloads::OperationState::Accepted
    );
    assert_eq!(
        serde_json::to_value(controller.transition(request.clone()).await.unwrap()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    // Admission is durable intention. It cannot falsely claim a private bridge,
    // hold, new Connection or physical media readiness.
    wait_slots(&f, 2).await;
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let effects:i64=c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.operation_id='consult-native-ready'",[],|r|r.get(0)).unwrap();
    assert_eq!(effects, 0);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operations WHERE operation_id IN ('consult-without-grant','consult-includes-remote')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(c);
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 800).await;
    let cancelled=controller.request(payload("operation.cancel",json!({"operation_id":"consult-native-ready","reason":"cancel before media preparation"})),CommandScope{
        conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("codec-target".into()),operation_id:Some("cancel-consult-native-ready".into()),..Default::default()
    }).await.unwrap();
    cancelled.ack().unwrap();
    clients[0].close();
    wait_slots(&f, 1).await;
    // EOF first removes the actual adapter route. Wait for the independent
    // lifetime worker to journal retirement before testing journal admission.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = serde_json::to_value(
            controller
                .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
                .await
                .unwrap(),
        )
        .unwrap();
        if snapshot["items"].as_array().unwrap().iter().any(|i| {
            i["kind"] == "connection"
                && i["data"]["connection_id"] == "codec-source"
                && i["data"]["state"] == "retired"
        }) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "native EOF retirement must reach the journal"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut after_eof = request;
    after_eof.operation_id = "consult-after-remote-eof".into();
    assert!(controller.transition(after_eof).await.is_err());
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='consult-after-remote-eof'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    for client in clients {
        client.close()
    }
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn production_private_native_candidate_is_muted_and_quarantined_before_any_scene_publication()
{
    private_candidate_case(false, false, false, false, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_switches_actual_audio_without_leaking_to_remote_or_public_context(
) {
    private_candidate_case(true, false, false, false, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_remote_eof_records_interruption_without_inventing_a_continuing_call(
) {
    private_candidate_case(true, true, false, false, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_unconfigured_hold_fails_before_audience_io_and_preserves_public_audio(
) {
    private_candidate_case(true, false, true, false, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_return_restores_original_audio_with_fresh_permission() {
    private_candidate_case(true, false, false, true, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_return_after_remote_eof_settles_without_claiming_a_call() {
    private_candidate_case(true, true, false, true, false, None).await;
}
#[tokio::test]
async fn production_private_consultation_captured_result_survives_recording_outage_without_reapplication(
) {
    private_candidate_case(true, false, false, true, true, None).await;
}
#[tokio::test]
async fn production_private_consultation_record_erasure_enforces_actual_lineage_and_replays_after_restart(
) {
    private_candidate_case_with_records(true, false, false, false, false, None, true).await;
}
#[tokio::test]
async fn production_private_consultation_tls_links_bind_original_viewer_and_invalidate_after_source_erasure(
) {
    private_candidate_case_with_record_links(true, false, false, false, false, None, false, true)
        .await;
}
async fn private_candidate_case(
    apply: bool,
    lose_remote: bool,
    configured_hold: bool,
    return_to_public: bool,
    recording_outage: bool,
    provider: Option<provider::Case>,
) {
    Box::pin(private_candidate_case_with_records(
        apply,
        lose_remote,
        configured_hold,
        return_to_public,
        recording_outage,
        provider,
        false,
    ))
    .await;
}
async fn private_candidate_case_with_records(
    apply: bool,
    lose_remote: bool,
    configured_hold: bool,
    return_to_public: bool,
    recording_outage: bool,
    provider: Option<provider::Case>,
    verify_records: bool,
) {
    Box::pin(private_candidate_case_with_record_links(
        apply,
        lose_remote,
        configured_hold,
        return_to_public,
        recording_outage,
        provider,
        verify_records,
        false,
    ))
    .await;
}
async fn private_candidate_case_with_record_links(
    apply: bool,
    lose_remote: bool,
    configured_hold: bool,
    return_to_public: bool,
    recording_outage: bool,
    provider: Option<provider::Case>,
    verify_records: bool,
    verify_links: bool,
) {
    Box::pin(private_candidate_case_with_auxiliary_checks(
        apply,
        lose_remote,
        configured_hold,
        return_to_public,
        recording_outage,
        provider,
        verify_records,
        verify_links,
        None,
    ))
    .await;
}
#[tokio::test]
async fn production_private_capability_delivery_uses_only_actual_private_connections_and_restores_fresh_public_delivery(
) {
    for algorithm in ["EdDSA", "ES256"] {
        private_candidate_case_with_auxiliary_checks(
            true,
            false,
            false,
            true,
            false,
            None,
            false,
            false,
            Some(Auxiliary::Capability(algorithm)),
        )
        .await;
    }
}
#[derive(Clone, Copy)]
enum Auxiliary {
    Capability(&'static str),
    Dtmf(&'static str),
    Sdk(&'static str),
}
#[tokio::test]
async fn production_private_dtmf_excludes_remote_and_same_participant_public_peer_and_replay_cannot_publish_after_return(
) {
    for algorithm in ["EdDSA", "ES256"] {
        private_candidate_case_with_auxiliary_checks(
            true,
            false,
            false,
            true,
            false,
            None,
            false,
            false,
            Some(Auxiliary::Dtmf(algorithm)),
        )
        .await;
    }
}
async fn private_candidate_case_with_auxiliary_checks(
    apply: bool,
    lose_remote: bool,
    configured_hold: bool,
    return_to_public: bool,
    recording_outage: bool,
    provider: Option<provider::Case>,
    verify_records: bool,
    verify_links: bool,
    auxiliary: Option<Auxiliary>,
) {
    let sdk_algorithm = match auxiliary {
        Some(Auxiliary::Sdk(a)) => Some(a),
        _ => None,
    };
    let capability_algorithm = match auxiliary {
        Some(Auxiliary::Capability(a)) => Some(a),
        _ => None,
    };
    let dtmf_algorithm = match auxiliary {
        Some(Auxiliary::Dtmf(a)) => Some(a),
        _ => None,
    };
    let codecs::Running {
        mut f,
        mut clients,
        cid,
        sh,
        th,
    } = if verify_links {
        let mut f = Fixture::with_native_end_scope("EdDSA", 12).await;
        f.config.record_https = Some(connect_server::canonical::RecordHttpsConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            server_name: "localhost".into(),
            capacity: 8,
            ttl_seconds: 60,
        });
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        codecs::setup_started(f, (), "codec-target-offer", "codec-source-offer")
            .await
            .0
    } else if let Some(algorithm) = dtmf_algorithm {
        codecs::setup_started_dtmf(Fixture::with_native_dtmf_scope(algorithm, 12).await, true).await
    } else if let Some(algorithm) = capability_algorithm.or(sdk_algorithm) {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        codecs::setup_started(f, (), "codec-target-offer", "codec-source-offer")
            .await
            .0
    } else {
        codecs::setup().await
    };
    let room = clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == cid)
        .unwrap();
    let own = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let consultant = if provider.is_some() {
        let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Identity {
                    operation_id: "private-ai-identity".into(),
                    alias: "private-consultant".into(),
                    kind: IdentityKind::Ai,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        identity_id
    } else {
        f.server
            .store
            .create_identity("private-consultant")
            .unwrap()
            .id
    };
    let added = clients[1]
        .participant_change(
            &cid,
            "private-consultant-member",
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":consultant,"kind":if provider.is_some(){"ai"}else{"human"},"role":"consultant"}),
            ),
        )
        .await
        .unwrap();
    let connect_client::membership::MembershipOutcome::Participant { participant, .. } = added
    else {
        panic!()
    };
    let party = |index, caps| {
        if dtmf_algorithm.is_some() {
            let mut scope = super::dtmf::signal_scope();
            scope
                .operations
                .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
            party_with_scope(&f, &consultant, index, caps, scope)
        } else {
            party_with_capabilities(&f, &consultant, index, true, caps)
        }
    };
    let public_config = party(101, vec!["audio".into()]);
    let private_config = party(102, vec!["audio".into(), "video".into(), "data".into()]);
    let mut public = CanonicalClient::open(public_config.clone()).await.unwrap();
    let revision = || -> u64 {
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='native-session'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    clients[1]
        .session_command(Request {
            operation_id: "invite-private-consultant".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision()),
            action: Action::Invite {
                participants: vec![participant.participant_id.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu","g.711-a"]}),
            },
        })
        .await
        .unwrap();
    public
        .session_command(Request {
            operation_id: "accept-private-consultant".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision()),
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu","g.711-a"]}),
            },
        })
        .await
        .unwrap();
    let capabilities = json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"});
    let public_answer=negotiate_media(&mut public,&cid,"public-consultant-offer","public-consultant",json!([{"id":"public-consultant-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]),capabilities.clone()).await;
    wait_native_operation(
        &f,
        &public_config.credential,
        &cid,
        "public-consultant-offer",
    )
    .await;
    if apply || provider.is_some() {
        stream_control(
            &mut public,
            &cid,
            "public-consultant",
            "public-context-receives-primary",
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"from_participant":own,"kinds":["audio"]}]}),
        )
        .await;
    }

    stream_control(&mut clients[1],&cid,"codec-target","private-future-selection",2,"stream.subscribe",json!({"subscriptions":[{"strm_id":"codec-voice"},{"from_participant":participant.participant_id,"kinds":["audio"]}]})).await;
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant
        .operations
        .extend(["session.consult".into(), "operation.cancel".into()]);
    if provider == Some(provider::Case::AutomaticMissingContext) {
        grant.operations.retain(|op| op != "context_view");
    }
    let credential = f
        .server
        .store
        .issue(
            if provider == Some(provider::Case::AutomaticAiCaller) {
                &consultant
            } else {
                &f.client.identity_id
            },
            grant,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(&f);
    config.credential = credential.token.clone();
    let mut controller = CanonicalHttpClient::open(config).await.unwrap();
    let command = TransitionCommand {
        operation_id: "prepare-private-native".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some("codec-target".into()),
        expected_revision: revision(),
        intent: TransitionIntent::Consult {
            retained_connid: "codec-source".into(),
            private_participants: vec![own, participant.participant_id.clone()],
            hold_treatment: if configured_hold {
                HoldTreatment::ConfiguredMedia
            } else {
                HoldTreatment::Silence
            },
        },
        payload_extensions: Default::default(),
    };
    let sdk_probe = if sdk_algorithm.is_some() {
        Some(sdk_controls::submit(&f, &command, &credential.token).await)
    } else {
        None
    };
    let admitted = if provider.is_some_and(provider::Case::automatic) {
        None
    } else {
        if sdk_probe.is_none() {
            controller.transition(command.clone()).await.unwrap();
        }
        let store = f.server.store.clone();
        let key = credential.id.clone();
        let id = command.operation_id.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                store.execute_operation_action(
                    &key,
                    connect_protocol::transitions::OperationAction::Get { operation_id: id },
                )
            })
            .await
            .unwrap()
            .unwrap()
            .record_id,
        )
    };
    if let Some(provider_case) = provider {
        Box::pin(provider::case(
            f,
            clients,
            cid,
            sh,
            th,
            public,
            public_answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
                .as_u64()
                .unwrap() as u16,
            public_config,
            private_config,
            credential,
            command,
            admitted,
            provider_case,
        ))
        .await;
        return;
    }
    let origin = f
        .server
        .store
        .native_consultation_origin(&credential.id, command.clone())
        .unwrap();
    let mut private = CanonicalClient::open(private_config.clone()).await.unwrap();
    let private_offer = json!({"substrate":"quic","capabilities":capabilities,"private_consultation":{"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()},"streams_offered":[{"id":"private-consultant-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let endpoint: String = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [&private_config.device_id],
            |r| r.get(0),
        )
        .unwrap();
    // Trusted coordinator invitation registration; this is not an unsigned
    // substitute for the actual following signed peer/offer/confirmation.
    f.server
        .store
        .register_native_consultation_leg(
            &origin,
            connect_protocol::connection_control::OfferIntent {
                operation_id: "private-consultant-offer".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                connection_id: "private-consultant".into(),
                by_participant: None,
                expected_revision: Some(0),
                offer: private_offer.clone(),
            },
            &endpoint,
        )
        .unwrap();
    // Another physical Device of the same logical Participant cannot reserve
    // this invitation's logical operation ID or substitute its public context.
    assert!(public
        .request_value(
            "connection.offer",
            private_offer.clone(),
            CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some("private-consultant".into()),
                operation_id: Some("private-consultant-offer".into()),
                expected_revision: Some(0)
            }
        )
        .await
        .unwrap()
        .ack()
        .is_err());
    {
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='private-consultant-offer'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM connections WHERE id='private-consultant'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    let answer = negotiate_media_payload(
        &mut private,
        &cid,
        "private-consultant-offer",
        "private-consultant",
        private_offer,
    )
    .await;
    wait_native_operation(
        &f,
        &private_config.credential,
        &cid,
        "private-consultant-offer",
    )
    .await;
    let completed = private
        .operation_get(
            "private-consultant-offer",
            CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(completed).unwrap()["result"]["private_consultation"],
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id(),"prepared_muted":true})
    );
    let durable = f
        .server
        .store
        .execute_operation_action(
            &credential.id,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "prepare-private-native".into(),
            },
        )
        .unwrap();
    assert_eq!(
        durable.state,
        connect_protocol::transitions::OperationState::Accepted
    );
    assert!(durable.effects.is_empty());
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let result: String = c
        .query_row(
            "SELECT result_json FROM operations WHERE operation_id='private-consultant-offer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    // A client must require this signed operation result before initializing or
    // sending private content; the native answer alone is not private readiness.
    assert_eq!(
        result["private_consultation"],
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id(),"prepared_muted":true})
    );
    assert_eq!(result["stream_revision"], 2);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON e.operation_record_id=o.record_id WHERE o.operation_id='private-consultant-offer' AND e.state='confirmed'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(c);
    let remote_snapshot_key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .token;
    for (key, visible) in [
        (&f.client.credential, true),
        (&public_config.credential, true),
        (&remote_snapshot_key, false),
    ] {
        let key = f.server.store.authenticate(key).unwrap().credential_id;
        let snapshot = f
            .server
            .store
            .conversation_snapshot(&key, &cid, Default::default())
            .unwrap();
        let raw = serde_json::to_value(snapshot).unwrap();
        assert_eq!(
            raw["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["kind"] == "connection"
                    && i["data"]["connection_id"] == "private-consultant"),
            visible
        );
    }
    let ph = answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let primary = clients.pop().unwrap();
    let remote = clients.pop().unwrap();
    let mut probes = vec![private, primary, remote, public];
    if apply {
        let public_handle = public_answer.original()["payload"]["substrate_setup"]["streams"][0]
            ["stream_local_id"]
            .as_u64()
            .unwrap() as u16;
        // This same primary was actually audible in the public context.
        probes.swap(0, 1);
        probes.swap(1, 3);
        assert_audio_reaches_other_participant(&mut probes, th, &[public_handle], 82000).await;
        probes.swap(1, 3);
        probes.swap(0, 1);
        if recording_outage {
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            c.execute_batch("CREATE TRIGGER fixture_consultation_recording_outage BEFORE UPDATE OF applied_json ON native_consultation_attempts WHEN NEW.applied_json IS NOT NULL BEGIN SELECT RAISE(ABORT,'fixture captured recording unavailable'); END;").unwrap();
        }
        controller.transition(command.clone()).await.unwrap();
        if recording_outage {
            // Outlast the bounded Core interval and captured-recording retries.
            // Until the captured result is durable, do not infer media readiness.
            tokio::time::sleep(Duration::from_secs(12)).await;
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let (started,applied,attempt):(bool,Option<String>,u64)=c.query_row("SELECT n.started,n.applied_json,e.attempt FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert!(started);
            assert!(applied.is_none());
            assert_eq!(attempt, 1);
            c.execute_batch("DROP TRIGGER fixture_consultation_recording_outage")
                .unwrap();
            drop(c);
            controller.transition(command.clone()).await.unwrap();
        }
        if configured_hold {
            // Origin admission and refusal recording are separate bounded
            // metadata phases. Neither can submit an audience effect.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let record = f
                    .server
                    .store
                    .execute_operation_action(
                        &credential.id,
                        connect_protocol::transitions::OperationAction::Get {
                            operation_id: command.operation_id.clone(),
                        },
                    )
                    .unwrap();
                if record.state == connect_protocol::transitions::OperationState::Failed {
                    assert!(record.effects.is_empty());
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "unsupported hold remained pending"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            wait_slots(&f, 3).await;
            probes.swap(0, 2);
            assert_audio_reaches_other_participant(&mut probes, sh, &[th], 85000).await;
            probes[1]
                .session_command(Request {
                    operation_id: "end-unsupported-private-hold".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision()),
                    action: Action::End {
                        reason_code: 0,
                        reason: "explicit test engagement completion".into(),
                    },
                })
                .await
                .unwrap();
            wait_slots(&f, 0).await;
            for client in probes {
                client.shutdown().await.unwrap();
            }
            if let Some(probe) = sdk_probe {
                probe.client.close(sdk_controls::options()).await.unwrap();
            }
            f.server.shutdown().await.unwrap();
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let record = f
                .server
                .store
                .execute_operation_action(
                    &credential.id,
                    connect_protocol::transitions::OperationAction::Get {
                        operation_id: command.operation_id.clone(),
                    },
                )
                .unwrap();
            if record
                .effects
                .first()
                .is_some_and(|e| e.state == connect_protocol::transitions::EffectState::Confirmed)
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "private audience did not apply: {:?}",
                serde_json::to_value(record).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if lose_remote {
            probes[2].close();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let record = f
                    .server
                    .store
                    .execute_operation_action(
                        &credential.id,
                        connect_protocol::transitions::OperationAction::Get {
                            operation_id: command.operation_id.clone(),
                        },
                    )
                    .unwrap();
                if record.state == connect_protocol::transitions::OperationState::Unknown {
                    assert_eq!(record.effects.len(), 1);
                    assert_eq!(
                        record.effects[0].state,
                        connect_protocol::transitions::EffectState::Confirmed
                    );
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "remote EOF did not fence the engagement"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            wait_slots(&f, 2).await;
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let (failure,decoded):(String,bool)=c.query_row("SELECT media_failure,decoded_audio_observed FROM native_consultation_attempts WHERE operation_record_id=?1",[origin.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(failure, "remote-ended");
            assert!(!decoded);
            drop(c);
            if return_to_public {
                let operator = f.server.store.authenticate(&f.admin).unwrap();
                f.server.store.revoke(&operator, &credential.id).unwrap();
                perform_return_with_sdk(
                    &f,
                    &cid,
                    origin.record_id(),
                    revision(),
                    false,
                    vec!["codec-source", "private-consultant"],
                    sdk_probe.as_ref().map(|p| p.reference.clone()),
                )
                .await;
                let c = rusqlite::Connection::open(&f.config.database).unwrap();
                assert_eq!(
                    c.query_row(
                        "SELECT state FROM operations WHERE record_id=?1",
                        [origin.record_id()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "cancelled"
                );
            }

            probes[1]
                .session_command(Request {
                    operation_id: "end-lost-private-remote".into(),
                    conversation_id: cid.clone(),
                    session_id: "native-session".into(),
                    by_participant: None,
                    expected_revision: Some(revision()),
                    action: Action::End {
                        reason_code: 0,
                        reason: "explicit settlement after remote EOF".into(),
                    },
                })
                .await
                .unwrap();
            wait_slots(&f, 0).await;
            for client in probes {
                client.shutdown().await.unwrap();
            }
            if let Some(probe) = sdk_probe {
                probe.client.close(sdk_controls::options()).await.unwrap();
            }
            f.server.shutdown().await.unwrap();
            return;
        }
        if let Some(algorithm) = capability_algorithm {
            capabilities::private(&f, &mut probes, &cid, algorithm).await;
        }
        let private_signal = if dtmf_algorithm.is_some() {
            Some(super::dtmf::private(&f, &mut probes, &cid).await)
        } else {
            None
        };
        // Candidate -> primary, then primary -> candidate are real decoded/mixed IO.
        assert_audio_reaches_other_participant(&mut probes, ph, &[th], 83000).await;
        probes.swap(0, 1);
        assert_audio_reaches_other_participant(&mut probes, th, &[ph], 84000).await;
        // Primary cannot publish private speech to remote or the old public context.
        probes.swap(1, 2);
        assert_audio_is_silent_after_unsubscribe(&mut probes, th, sh).await;
        probes.swap(1, 3);
        assert_audio_is_silent_after_unsubscribe(&mut probes, th, public_handle).await;
        // Candidate cannot publish to retained remote, and retained remote is held.
        probes.swap(0, 2);
        probes.swap(1, 3);
        assert_audio_is_silent_after_unsubscribe(&mut probes, ph, sh).await;
        probes.swap(0, 1);
        probes.swap(1, 2);
        assert_audio_is_silent_after_unsubscribe(&mut probes, sh, th).await;
        let record = f
            .server
            .store
            .execute_operation_action(
                &credential.id,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: command.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            record.state,
            connect_protocol::transitions::OperationState::Completed
        );
        let completion: connect_protocol::transitions::TransitionCompletion =
            serde_json::from_value(record.result.clone().unwrap()).unwrap();
        assert_eq!(completion.admitted_connection_ids, ["private-consultant"]);
        assert_eq!(completion.session_revision, command.expected_revision + 1);
        if let Some(probe) = &sdk_probe {
            sdk_controls::completed(&f, probe, completion.session_revision).await;
        }
        let mut competing = command.clone();
        competing.operation_id = "second-private-context".into();
        competing.expected_revision = completion.session_revision;
        assert!(controller.transition(competing).await.is_err());
        assert_eq!(
            rusqlite::Connection::open(&f.config.database)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM operations WHERE operation_id='second-private-context'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
        assert!(rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT decoded_audio_observed FROM native_consultation_attempts WHERE operation_record_id=?1",[origin.record_id()],|r|r.get::<_,bool>(0)).unwrap());
        assert_eq!(record.effects.len(), 1);
        assert_eq!(record.effects[0].attempt, 1);
        let before = serde_json::to_value(&record.effects).unwrap();
        controller.transition(command.clone()).await.unwrap();
        let after = f
            .server
            .store
            .execute_operation_action(
                &credential.id,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: command.operation_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(serde_json::to_value(after.effects).unwrap(), before);
        // The completed context uses current physical media grants. Revoking
        // the original execution credential cannot revoke those separate peers
        // or authorize another audience attempt.
        let operator = f.server.store.authenticate(&f.admin).unwrap();
        f.server.store.revoke(&operator, &credential.id).unwrap();
        assert!(controller.transition(command.clone()).await.is_err());
        // Current ordering is [remote, primary, candidate, old public context].
        probes[3].close();
        wait_slots(&f, 3).await;
        probes.swap(0, 2);
        assert_audio_reaches_other_participant(&mut probes, ph, &[th], 86000).await;
        probes.swap(0, 1);
        assert_audio_reaches_other_participant(&mut probes, th, &[ph], 87000).await;
        probes.swap(0, 1);
        probes.swap(0, 2);
        if return_to_public {
            let _returned = perform_return_with_sdk(
                &f,
                &cid,
                origin.record_id(),
                revision(),
                true,
                vec!["private-consultant"],
                sdk_probe.as_ref().map(|p| p.reference.clone()),
            )
            .await;
            wait_slots(&f, 2).await;
            // Current order is remote, primary, retired candidate, retired public context.
            if let Some(algorithm) = capability_algorithm {
                capabilities::returned(&f, &mut probes, &cid, algorithm).await;
            }
            if let Some(original) = private_signal {
                super::dtmf::returned(&f, &mut probes, &cid, original).await;
            }
            assert_audio_reaches_other_participant(&mut probes, sh, &[th], 88000).await;
            probes.swap(0, 1);
            assert_audio_reaches_other_participant(&mut probes, th, &[sh], 89000).await;
            probes.swap(0, 1);
        }
        probes[1]
            .session_command(Request {
                operation_id: "end-private-audience-session".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision()),
                action: Action::End {
                    reason_code: 0,
                    reason: "explicit private engagement termination".into(),
                },
            })
            .await
            .unwrap();
        wait_slots(&f, 0).await;
        if verify_links {
            record_links::verify(
                &f,
                &cid,
                &consultant,
                &private_config.credential,
                &remote_snapshot_key,
            )
            .await;
        }
        if let Some(probe) = sdk_probe {
            probe.client.close(sdk_controls::options()).await.unwrap();
        }
        let replay = if verify_records {
            Some(
                record_erasure::verify(
                    &f,
                    &cid,
                    &consultant,
                    &participant.participant_id,
                    &private_config.credential,
                    &remote_snapshot_key,
                )
                .await,
            )
        } else {
            None
        };
        for client in probes {
            client.shutdown().await.unwrap();
        }
        if let Some(replay) = replay {
            let config = f.config.clone();
            f.server.shutdown().await.unwrap();
            f.server = RunningCanonicalServer::start(config).await.unwrap();
            record_erasure::after_restart(&f, &cid, replay).await;
        }
        f.server.shutdown().await.unwrap();
        return;
    }

    assert_audio_is_silent_after_unsubscribe(&mut probes, ph, th).await;
    probes.swap(1, 2);
    assert_audio_is_silent_after_unsubscribe(&mut probes, ph, sh).await;
    probes.swap(0, 1);
    assert_audio_is_silent_after_unsubscribe(&mut probes, sh, ph).await;
    // Private preparation leaves the retained public engagement usable.
    probes.swap(1, 2);
    assert_audio_reaches_other_participant(&mut probes, sh, &[th], 80000).await;
    controller.request_value("operation.cancel",json!({"operation_id":"prepare-private-native","reason":"cancel quarantined preparation"}),CommandScope {conversation_id:Some(cid.clone()),session_id:Some("native-session".into()),connection_id:Some("codec-target".into()),operation_id:Some("cancel-private-preparation".into()),..Default::default()}).await.unwrap().ack().unwrap();
    wait_slots(&f, 3).await;
    assert_audio_reaches_other_participant(&mut probes, sh, &[th], 81000).await;
    probes[1]
        .session_command(Request {
            operation_id: "end-after-cancelled-private-preparation".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(revision()),
            action: Action::End {
                reason_code: 0,
                reason: "explicit completion after original audio verification".into(),
            },
        })
        .await
        .unwrap();
    wait_slots(&f, 0).await;

    for client in probes {
        client.shutdown().await.unwrap();
    }
    f.server.shutdown().await.unwrap();
}

async fn perform_return(
    f: &Fixture,
    cid: &str,
    parent: &str,
    expected_revision: u64,
    continuing: bool,
    retired: Vec<&str>,
) -> serde_json::Value {
    perform_return_with_sdk(f, cid, parent, expected_revision, continuing, retired, None).await
}

async fn perform_return_with_sdk(
    f: &Fixture,
    cid: &str,
    parent: &str,
    expected_revision: u64,
    continuing: bool,
    retired: Vec<&str>,
    sdk_reference: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut grant = Scope::collaboration(vec!["*".into()]);
    grant.operations.push("session.consult.return".into());
    let store = f.server.store.clone();
    let identity = f.client.identity_id.clone();
    let return_key = tokio::task::spawn_blocking(move || {
        store.issue(&identity, grant, chrono::Utc::now().timestamp() + 600)
    })
    .await
    .unwrap()
    .unwrap();
    let sdk_return = if let Some(reference) = sdk_reference {
        Some(
            sdk_controls::restore(
                f,
                &return_key.token,
                reference,
                expected_revision,
                continuing,
                &retired,
            )
            .await,
        )
    } else {
        None
    };
    let mut cfg = canonical_http_config(f);
    cfg.credential = return_key.token.clone();
    let mut return_client = CanonicalHttpClient::open(cfg).await.unwrap();
    let return_command = TransitionCommand {
        operation_id: "return-private-native".into(),
        conversation_id: cid.to_owned(),
        session_id: "native-session".into(),
        connection_id: Some("codec-target".into()),
        expected_revision,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: parent.into(),
            retained_connid: "codec-source".into(),
        },
        payload_extensions: Default::default(),
    };
    if sdk_return.is_none() {
        return_client
            .transition(return_command.clone())
            .await
            .unwrap();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
    let returned = loop {
        let store = f.server.store.clone();
        let key = return_key.id.clone();
        let status = tokio::task::spawn_blocking(move || {
            store.execute_operation_action(
                &key,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: "return-private-native".into(),
                },
            )
        })
        .await
        .unwrap()
        .unwrap();
        if status.state == connect_protocol::transitions::OperationState::Completed {
            break serde_json::to_value(status).unwrap();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "native return did not complete: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(returned["result"]["remote_continuing"], continuing);
    assert_eq!(returned["result"]["retired_connection_ids"], json!(retired));
    let before = returned["effects"].clone();
    let replay = return_client
        .transition(return_command.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay.result).unwrap(),
        returned["result"]
    );
    let store = f.server.store.clone();
    let key = return_key.id.clone();
    let recorded = tokio::task::spawn_blocking(move || {
        store.execute_operation_action(
            &key,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "return-private-native".into(),
            },
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(serde_json::to_value(recorded.effects).unwrap(), before);
    let store = f.server.store.clone();
    let key = return_key.id.clone();
    let admin = f.admin.clone();
    tokio::task::spawn_blocking(move || store.revoke(&store.authenticate(&admin)?, &key))
        .await
        .unwrap()
        .unwrap();
    assert!(return_client.transition(return_command).await.is_err());
    if let Some((client, operation)) = sdk_return {
        assert!(
            matches!(operation.status(sdk_controls::options()).await,Err(e) if e.certainty==connect_client::sdk::Certainty::Refused)
        );
        client.close(sdk_controls::options()).await.unwrap();
    }
    returned
}

#[path = "consultation/groups.rs"]
mod groups;

#[path = "consultation/provider.rs"]
mod provider;
#[tokio::test]
async fn production_private_ai_automatic_signed_consult_uses_public_snapshot_reference_and_returns()
{
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticSignedReturn),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_jobs_close_provider_before_server_authority_retires() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticServerShutdown),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_http_consult_initializes_once_and_returns_without_manual_bridge(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticReturn),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_invitation_uses_independent_consent_and_preserves_original_intent(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticJoin),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_context_denial_has_no_provider_io_or_owner_fallback() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticMissingContext),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_cancel_during_export_drains_ack_and_preserves_original_audio(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticCancelExport),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_lost_creation_response_stays_unknown_without_recreating_or_exporting(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticLostCreationResponse),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_cancel_during_creation_records_late_call_and_closes_without_export(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticCancelCreation),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_native_endpoint_revocation_closes_provider_and_returns_original_audio(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticNativeEndpointRevoked),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_refused_native_begin_releases_primary_floor_and_preserves_original_audio(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::BeginRefused),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_primary_eof_restores_survivors_without_reviving_or_inventing_a_primary(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticPrimaryEof),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_creation_record_outage_retries_observation_and_closes_after_cancel_without_export(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticCancelCreationRecordOutage),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_provider_close_record_outage_retries_observation_without_another_end(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticProviderCloseRecordOutage),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_ai_caller_uses_own_context_grant_and_independent_owner_return(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticAiCaller),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_automatic_remote_eof_closes_provider_and_return_does_not_invent_a_call(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticRemoteEof),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_uses_fresh_provider_context_and_actual_bidirectional_audio_then_returns(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::Return),
    )
    .await;
}
#[tokio::test]
async fn production_private_ai_missing_context_ack_does_not_apply_audience_or_repeat_creation() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::MissingAck),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_context_revocation_closes_provider_and_preserves_primary_for_return()
{
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::ContextRevoked),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_erasure_stops_memory_and_preserves_return() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::RecordDeleted),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_permission_has_no_execution_owner_fallback() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::RecordNoPermission),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_listener_share_revocation_stops_original_memory() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::RecordListenerShareRevoked),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_requires_actual_listener_original_audience() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::RecordListenerDenied),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_cancel_pending_export_retains_erasure_custody() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticCancelRecordExport),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn production_private_ai_record_context_actual_socket_write_retains_erasure_and_cancel_custody(
) {
    if provider::write_profile("production_private_ai_record_context_actual_socket_write_retains_erasure_and_cancel_custody").await {return;}
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticRecordWriteCancel),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn production_private_ai_record_context_source_expiry_retires_partial_socket_write_and_preserves_public_audio(
) {
    if provider::write_profile("production_private_ai_record_context_source_expiry_retires_partial_socket_write_and_preserves_public_audio").await {return;}
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::AutomaticRecordWriteExpiry),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_record_context_retention_stops_memory_and_preserves_return() {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::RecordExpired),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_wrong_context_ack_preserves_public_audio_and_closes_only_reserved_call(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::WrongAck),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_provider_end_records_actual_cleanup_without_repeating_end_and_returns(
) {
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::ProviderEnded),
    )
    .await;
}

#[tokio::test]
async fn production_private_ai_socket_loss_preserves_cleanup_uncertainty_and_returns_original_call()
{
    private_candidate_case(
        false,
        false,
        false,
        false,
        false,
        Some(provider::Case::SocketLost),
    )
    .await;
}
