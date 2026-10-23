//! Actual signed public Session invite, configured initial agent and UDP SIP answer.
use super::*;
#[path = "initial/activation.rs"]
mod activation;
use rusqlite::OptionalExtension;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_public_invite_records_real_answer_and_guarded_admission_once()
{
    initial_case(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_public_end_confirms_original_cleanup_after_generation_fence()
{
    initial_case(true).await;
}
struct ConfiguredInitial {
    f: Fixture,
    peer: PeerSip,
    phone_credential: String,
}
async fn configured_initial(payload: Option<u8>, timeout_seconds: u32) -> ConfiguredInitial {
    configured_initial_for("EdDSA", payload, timeout_seconds).await
}
async fn configured_initial_for(
    algorithm: &str,
    payload: Option<u8>,
    timeout_seconds: u32,
) -> ConfiguredInitial {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_service=debug")
        .with_test_writer()
        .try_init();
    let f = Fixture::with_native_end_scope(algorithm, 12).await;
    let peer = PeerSip::with_audio(payload).await;
    *peer.ledger.lock().unwrap() = Some(f.config.database.clone());
    let store = &f.server.store;
    let admin = store.authenticate(&f.admin).unwrap();
    let ProvisioningOutcome::Identity {
        identity_id: route_owner,
        ..
    } = store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::Identity {
                operation_id: "initial-route-owner".into(),
                alias: "initial-route-owner".into(),
                kind: IdentityKind::System,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let route_key = store
        .issue(
            &route_owner,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let phone_key = store
        .issue(
            &f.peer.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
        .execute_provisioning(
            &route_key.id,
            ProvisioningCommand::Endpoint {
                operation_id: "initial-phone-endpoint".into(),
                identity_id: f.peer.identity_id.clone(),
                device: DeviceSpec {
                    id: "initial-phone-device".into(),
                    kind: "telephone".into(),
                    platform: "isolated-canonical-fixture".into(),
                },
                transport: EndpointTransport::Telephone,
                address: "+14155550123".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("config_initial_callback".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .execute_provisioning(
            &route_key.id,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-initial-phone".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "isolated-fixture".into(),
            },
        )
        .unwrap();
    let policy = connect_store::InitialTelephonePolicy {
        endpoint_id: endpoint_id.clone(),
        endpoint_credential_id: phone_key.id.clone(),
        route_credential_id: route_key.id.clone(),
        route_reference: "config_initial_callback".into(),
        sip_authority: peer.address.to_string(),
        timeout_seconds,
    };
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer: peer_config,
        admin,
        key_revision,
    } = f;
    server.shutdown().await.unwrap();
    config.telephone = Some(
        connect_server::canonical_telephone::CanonicalTelephoneConfig {
            bind: std::net::UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap(),
            media_port_start: 20200,
            media_port_end: 20263,
            outbound: None,
            routes: vec![],
            initial: vec![policy],
        },
    );
    let path = temp.path().join("initial-canonical.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    config = CanonicalServerConfig::load(&path).unwrap();
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    let f = Fixture {
        temp,
        config,
        server,
        client,
        peer: peer_config,
        admin,
        key_revision,
    };
    ConfiguredInitial {
        f,
        peer,
        phone_credential: phone_key.id,
    }
}
async fn initial_case(public_end: bool) {
    let ConfiguredInitial { f, peer, .. } = configured_initial(Some(0), 30).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut room_input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    room_input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("initial-room", room_input)
        .await
        .unwrap();
    let caller = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let target = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let request = connect_protocol::session_control::Request {
        operation_id: "public-initial-invite".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: Some(caller),
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![target.clone()],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    client.session_command(request.clone()).await.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let admission_deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        let state:Option<String>=db.query_row("SELECT o.state FROM operations o JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id",[],|r|r.get(0)).optional().unwrap();
        if state.as_deref() == Some("completed") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < admission_deadline,
            "actual initial admission did not complete: {state:?}"
        );
        assert!(
            !matches!(state.as_deref(), Some("failed" | "unknown" | "cancelled")),
            "initial call failed: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(peer.unique(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE accepted=1 AND connected_at IS NOT NULL AND retired_at IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT state FROM session_participants WHERE session_id='native-session' AND participant_id=?1",[&target],|r|r.get::<_,String>(0)).unwrap(),"present");
    client.session_command(request).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(peer.unique(), 1);
    assert!(peer.methods.lock().unwrap().contains(&Method::Ack));
    // The same canonical Core now admits an actual signed native endpoint and
    // routes marked PCMU through the guarded initial telephone controller.
    ready_endpoint(&f).await;
    let answer = negotiate_named(
        &mut client,
        &room.id,
        "initial-native-offer",
        "initial-native",
        "initial-native-audio",
    )
    .await;
    wait_native_operation(&f, &f.client.credential, &room.id, "initial-native-offer").await;
    assert_eq!(db.query_row("SELECT c.state FROM connections c JOIN operations o ON o.connection_id=c.id JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id",[],|r|r.get::<_,String>(0)).unwrap(),"ready");
    stream_control(
        &mut client,
        &room.id,
        "initial-native",
        "initial-select-phone",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"telephone-audio"}]}),
    )
    .await;
    let handle = answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16;
    let mut native = [Peer {
        client,
        config: Some(f.client.clone()),
        identity: f.client.identity_id.clone(),
        participant: room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone(),
        id: "initial-native".into(),
        handle,
        clock: tokio::time::Instant::now(),
        sequence: 0,
        live: true,
    }];
    sip_native_duplex(&peer, &mut native, 0, "initial-public-native-duplex").await;
    if public_end {
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let session = f
            .server
            .store
            .session_view(&actor.credential_id, &room.id, "native-session")
            .unwrap();
        native[0]
            .client
            .session_command(connect_protocol::session_control::Request {
                operation_id: "public-initial-end".into(),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(session.topology_revision),
                action: connect_protocol::session_control::Action::End {
                    reason_code: 1000,
                    reason: "complete".into(),
                },
            })
            .await
            .unwrap();
        until(||db.query_row("SELECT count(*) FROM connections c JOIN operations o ON o.connection_id=c.id JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id WHERE c.state='retired' AND c.generation=2",[],|r|r.get::<_,u64>(0)).unwrap()==1).await;
    }
    native[0].client.close();

    f.server.shutdown().await.unwrap();
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE retired_at IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    BeforeStart,
    RingingEnd,
    RingingRevoke,
    Deadline,
    Codec,
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_refusal_before_allocation_does_not_poison_drain() {
    refusal_case(Refusal::BeforeStart).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_public_end_while_ringing_cancels_exact_dialog() {
    refusal_case(Refusal::RingingEnd).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_revocation_while_ringing_preserves_uncertainty_and_cleans_up()
{
    refusal_case(Refusal::RingingRevoke).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_no_answer_deadline_replay_never_redials() {
    refusal_case(Refusal::Deadline).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_telephone_codec_refusal_preserves_answer_without_acceptance() {
    refusal_case(Refusal::Codec).await;
}
async fn refusal_case(mode: Refusal) {
    let ConfiguredInitial {
        f,
        peer,
        phone_credential,
    } = configured_initial(
        if mode == Refusal::Codec {
            Some(8)
        } else {
            None
        },
        if mode == Refusal::Deadline { 3 } else { 5 },
    )
    .await;
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    if mode == Refusal::BeforeStart {
        f.server.store.revoke(&admin, &phone_credential).unwrap();
    }
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("refusal-room", input)
        .await
        .unwrap();
    let target = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let request = connect_protocol::session_control::Request {
        operation_id: "public-initial-refusal".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        by_participant: None,
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![target.clone()],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    client.session_command(request.clone()).await.unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    if mode == Refusal::BeforeStart {
        // The host has already been proven by the positive cases. Let this
        // ordinary queued notification execute before closing its intake.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(peer.unique(), 0);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM initial_telephone_operations",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        client.session_command(request).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.close();
        f.server
            .shutdown()
            .await
            .expect("a pre-allocation policy refusal owns no uncertain Core work");
        assert_eq!(peer.unique(), 0);
        return;
    }
    until(|| peer.unique() == 1).await;
    let binding: (String, String) = db
        .query_row(
            "SELECT runtime_connection_id,sip_call_id FROM initial_telephone_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    if mode == Refusal::RingingEnd {
        let actor = f.server.store.authenticate(&f.client.credential).unwrap();
        let view = f
            .server
            .store
            .session_view(&actor.credential_id, &room.id, "native-session")
            .unwrap();
        client
            .session_command(connect_protocol::session_control::Request {
                operation_id: "end-ringing".into(),
                conversation_id: room.id.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(view.topology_revision),
                action: connect_protocol::session_control::Action::End {
                    reason_code: 1000,
                    reason: "cancel ringing".into(),
                },
            })
            .await
            .unwrap();
    } else if mode == Refusal::RingingRevoke {
        f.server.store.revoke(&admin, &phone_credential).unwrap();
    }
    until(||db.query_row("SELECT count(*) FROM connections c JOIN operations o ON o.connection_id=c.id JOIN initial_telephone_operations i ON i.operation_record_id=o.record_id WHERE c.state='retired'",[],|r|r.get::<_,u64>(0)).unwrap()==1).await;
    assert_eq!(db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE accepted=1 OR connected_at IS NOT NULL",[],|r|r.get::<_,u64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    if mode == Refusal::Codec {
        assert_eq!(db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE accepted=0 AND retired_at IS NOT NULL AND codec='g.711-a'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row(
                "SELECT state FROM operation_effects WHERE compensates_effect_id IS NULL",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "confirmed"
        );
        assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    } else {
        assert_eq!(
            db.query_row("SELECT count(*) FROM initial_telephone_answers", [], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM operation_effects WHERE compensates_effect_id IS NULL",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "unknown"
        );
        assert!(peer.methods.lock().unwrap().contains(&Method::Cancel));
        assert!(!peer.methods.lock().unwrap().contains(&Method::Bye));
    }
    assert_eq!(db.query_row("SELECT count(*) FROM operation_effects WHERE compensates_effect_id IS NOT NULL AND state='confirmed'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    // Replay the original public invitation. Ended Sessions may refuse current
    // access; either response must retain the original once-only attempt.
    let _ = client.session_command(request).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let after: (String, String) = db
        .query_row(
            "SELECT runtime_connection_id,sip_call_id FROM initial_telephone_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(binding, after);
    assert_eq!(peer.unique(), 1);
    client.close();
    f.server
        .shutdown()
        .await
        .expect("settled original resources and terminal replay drain cleanly");
}
