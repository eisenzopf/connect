//! Two independent answered SIP dialogs receive the same frozen native signal.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_two_initial_sip_recipients_overlap_long_schedules_without_replay() {
    for algorithm in ["EdDSA", "ES256"] {
        exercise(algorithm, false).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_dtmf_two_initial_sip_recipients_settle_partial_refusal_after_other_finishes(
) {
    for algorithm in ["EdDSA", "ES256"] {
        exercise(algorithm, true).await;
    }
}

async fn second_phone(mut configured: ConfiguredInitial) -> (ConfiguredInitial, PeerSip, String) {
    let peer = PeerSip::with_audio_for_user(Some(0), "+14155550124").await;
    *peer.ledger.lock().unwrap() = Some(configured.f.config.database.clone());
    let store = &configured.f.server.store;
    let admin = store.authenticate(&configured.f.admin).unwrap();
    let ProvisioningOutcome::Identity { identity_id, .. } = store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::Identity {
                operation_id: "fanout-second-phone".into(),
                alias: "fanout-second-phone".into(),
                kind: IdentityKind::System,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let key = store
        .issue(
            &identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let original = configured.f.config.telephone.as_ref().unwrap().initial[0].clone();
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
        .execute_provisioning(
            &original.route_credential_id,
            ProvisioningCommand::Endpoint {
                operation_id: "fanout-second-endpoint".into(),
                identity_id: identity_id.clone(),
                device: DeviceSpec {
                    id: "fanout-second-device".into(),
                    kind: "telephone".into(),
                    platform: "isolated-canonical-fixture".into(),
                },
                transport: EndpointTransport::Telephone,
                address: "+14155550124".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("config_second_callback".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .execute_provisioning(
            &original.route_credential_id,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "fanout-verify-second".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "isolated-fixture".into(),
            },
        )
        .unwrap();
    configured
        .f
        .config
        .telephone
        .as_mut()
        .unwrap()
        .initial
        .push(connect_store::InitialTelephonePolicy {
            endpoint_id,
            endpoint_credential_id: key.id,
            route_credential_id: original.route_credential_id,
            route_reference: "config_second_callback".into(),
            sip_authority: peer.address.to_string(),
            timeout_seconds: 30,
        });
    configured.f.server.shutdown().await.unwrap();
    configured.f.server = RunningCanonicalServer::start(configured.f.config.clone())
        .await
        .unwrap();
    configured.f.client.endpoint = configured.f.server.address;
    (configured, peer, identity_id)
}
async fn exercise(algorithm: &str, partial: bool) {
    let (configured, second, second_identity) = second_phone(
        configured_initial_profile(
            algorithm,
            Some(0),
            30,
            EndpointTransport::Telephone,
            true,
            true,
        )
        .await,
    )
    .await;
    let ConfiguredInitial { f, peer: first, .. } = configured;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input = connect_client::conversations::CreateRequest::persistent(vec![
        f.peer.identity_id.clone(),
        second_identity,
    ]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("fanout-room", input)
        .await
        .unwrap();
    client
        .session_command(connect_protocol::session_control::Request {
            operation_id: "fanout-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: None,
            action: connect_protocol::session_control::Action::Invite {
                participants: room
                    .participants
                    .iter()
                    .filter(|p| p.identity_id != f.client.identity_id)
                    .map(|p| p.participant_id.clone())
                    .collect(),
                medium: "voice".into(),
                intent: "call".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    until(|| db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE accepted=1 AND connected_at IS NOT NULL AND retired_at IS NULL", [], |r|r.get::<_,u64>(0)).unwrap()==2).await;
    assert_eq!((first.unique(), second.unique()), (1, 1));
    ready_endpoint(&f).await;
    negotiate_named(
        &mut client,
        &room.id,
        "fanout-native-offer",
        "initial-native",
        "initial-native-audio",
    )
    .await;
    wait_native_operation(&f, &f.client.credential, &room.id, "fanout-native-offer").await;
    let operation = if partial {
        "fanout-partial"
    } else {
        "fanout-long"
    };
    let scope = CommandScope {
        conversation_id: Some(room.id.clone()),
        session_id: Some("native-session".into()),
        connection_id: Some("initial-native".into()),
        operation_id: Some(operation.into()),
        expected_revision: None,
    };
    first
        .reject_info
        .store(partial, std::sync::atomic::Ordering::Release);
    let (method, duration) = if partial {
        ("info", 1000)
    } else {
        ("rfc4733", 6000)
    };
    let started = tokio::time::Instant::now();
    client
        .send_native_dtmf(scope.clone(), "58", duration, method)
        .await
        .unwrap();
    let events = if partial {
        None
    } else {
        let (a, b) = tokio::join!(
            collect_events(&first, started),
            collect_events(&second, started)
        );
        assert!(
            a.0.abs_diff(b.0) < 1000,
            "recipients started serially: {a:?} {b:?}"
        );
        Some((a, b))
    };
    let deadline = started + Duration::from_secs(18);
    let completed = loop {
        let record = client
            .operation_get(
                operation,
                CommandScope {
                    conversation_id: Some(room.id.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if !matches!(
            record.state,
            rvoip_uctp::v2::payloads::OperationState::Preparing
        ) {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "fanout did not settle: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let signal = if partial {
        assert_eq!(
            completed.state,
            rvoip_uctp::v2::payloads::OperationState::Unknown
        );
        &completed.problem.as_ref().unwrap()["signal"]
    } else {
        assert_eq!(
            completed.state,
            rvoip_uctp::v2::payloads::OperationState::Completed
        );
        completed.result.as_ref().unwrap()
    };
    let mut dispositions = signal["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["disposition"].as_str().unwrap())
        .collect::<Vec<_>>();
    dispositions.sort();
    assert_eq!(
        dispositions,
        if partial {
            vec!["local_submitted", "outcome_unknown"]
        } else {
            vec!["local_submitted", "local_submitted"]
        }
    );
    assert_eq!(db.query_row("SELECT count(*) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1 AND g.attempted=1 AND g.frame_id IS NOT NULL AND g.disposition IS NOT NULL",[operation],|r|r.get::<_,u64>(0)).unwrap(),2);
    assert_eq!(db.query_row("SELECT count(DISTINCT g.frame_id) FROM native_dtmf_gateways g JOIN operations o ON o.record_id=g.operation_record_id WHERE o.operation_id=?1",[operation],|r|r.get::<_,u64>(0)).unwrap(),2);
    if partial {
        assert_info(&first, &["5"]);
        assert_info(&second, &["5", "8"]);
        assert!(
            started.elapsed() >= Duration::from_millis(1070),
            "partial outcome settled before the healthy suffix was submitted"
        );
    }
    let counts = (
        first.info.lock().unwrap().len(),
        second.info.lock().unwrap().len(),
    );
    let replay = client
        .send_native_dtmf(scope, "58", duration, method)
        .await
        .unwrap();
    assert_eq!(
        (replay.state, replay.result, replay.problem),
        (
            completed.state,
            completed.result.clone(),
            completed.problem.clone()
        )
    );
    tokio::join!(no_more_events(&first), no_more_events(&second));
    assert_eq!(
        (
            first.info.lock().unwrap().len(),
            second.info.lock().unwrap().len()
        ),
        counts
    );
    assert_eq!((first.unique(), second.unique()), (1, 1));
    if let Some(path) = std::env::var_os("CONNECT_SIP_DTMF_FANOUT_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"partial":partial,"method":method,"duration_ms":duration,"elapsed_ms":started.elapsed().as_millis(),"events":events,"completed":completed,"calls":[first.unique(),second.unique()],"info_packets":counts})).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
    client.close();
    f.server.shutdown().await.unwrap();
    assert!(first.methods.lock().unwrap().contains(&Method::Bye));
    assert!(second.methods.lock().unwrap().contains(&Method::Bye));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM initial_telephone_answers WHERE retired_at IS NOT NULL",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}
fn assert_info(peer: &PeerSip, expected: &[&str]) {
    let mut sequences = std::collections::BTreeMap::new();
    for raw in peer.info.lock().unwrap().iter() {
        let Message::Request(request) = parse_message(raw).unwrap() else {
            panic!()
        };
        let body = std::str::from_utf8(&request.body).unwrap().to_owned();
        sequences
            .entry(request.raw_header_value(&HeaderName::CSeq).unwrap())
            .or_insert(body);
    }
    assert_eq!(
        sequences
            .values()
            .map(|body| body
                .lines()
                .find_map(|line| line.strip_prefix("Signal="))
                .unwrap())
            .collect::<Vec<_>>(),
        expected
    );
}
async fn collect_events(peer: &PeerSip, started: tokio::time::Instant) -> (u128, usize) {
    let deadline = started + Duration::from_secs(18);
    let mut bytes = [0; 4096];
    let mut first = None;
    let mut events = std::collections::BTreeMap::<u32, (u8, usize)>::new();
    while events.len() < 2 || events.values().any(|(_, n)| *n < 3) {
        let (n, _) = tokio::time::timeout_at(deadline, peer.media.recv_from(&mut bytes))
            .await
            .expect("both complete schedules must fit concurrent budget")
            .unwrap();
        if n > 1 && (200..=211).contains(&bytes[1]) {
            continue;
        }
        let packet = rvoip_rtp_core::packet::RtpPacket::parse(&bytes[..n]).unwrap();
        if packet.header.payload_type != 101 {
            continue;
        }
        first.get_or_insert(started.elapsed().as_millis());
        assert_eq!(packet.payload.len(), 4);
        if packet.payload[1] & 0x80 != 0 {
            assert_eq!(
                u16::from_be_bytes([packet.payload[2], packet.payload[3]]),
                48000
            );
            let e = events
                .entry(packet.header.timestamp)
                .or_insert((packet.payload[0], 0));
            assert_eq!(e.0, packet.payload[0]);
            e.1 += 1;
        }
    }
    assert_eq!(
        events
            .values()
            .map(|(e, _)| *e)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([5, 8])
    );
    (first.unwrap(), events.len())
}
async fn no_more_events(peer: &PeerSip) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut bytes = [0; 4096];
    while let Ok(Ok((n, _))) =
        tokio::time::timeout_at(deadline, peer.media.recv_from(&mut bytes)).await
    {
        if n > 1 && (200..=211).contains(&bytes[1]) {
            continue;
        }
        let packet = rvoip_rtp_core::packet::RtpPacket::parse(&bytes[..n]).unwrap();
        assert_ne!(
            packet.header.payload_type, 101,
            "replay submitted another telephone event"
        );
    }
}
