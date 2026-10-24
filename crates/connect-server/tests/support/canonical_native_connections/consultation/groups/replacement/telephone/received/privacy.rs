//! Actual committed SIP source outside a live native private pair.
use super::*;
use rvoip_uctp::v2::{payloads::OperationState, Envelope, Scope as WireScope};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_moved_sip_received_private_audience_and_return_keep_actual_lifetime() {
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::DtmfPrivacy, true, algorithm).await;
    }
}
pub(in super::super) fn signal_scope() -> Scope {
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "dtmf.send".into(),
        "dtmf.receive".into(),
        "operation.get".into(),
    ]);
    scope
}
pub(in super::super) async fn signal(client: &mut CanonicalClient) -> Option<Envelope> {
    for _ in 0..256 {
        let Ok(frame) = client.receive().await else {
            return None;
        };
        if frame.kind() == "dtmf.received" {
            return Some(frame);
        }
    }
    panic!("private received signal missing behind lifecycle frames")
}
pub(in super::super) async fn settled(
    http: &mut CanonicalHttpClient,
    operation: &str,
    cid: &str,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        let row = http
            .operation_get(
                operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    session_id: Some("native-session".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if matches!(
            row.state,
            OperationState::Completed
                | OperationState::Failed
                | OperationState::Cancelled
                | OperationState::Unknown
        ) {
            return serde_json::to_value(row).unwrap();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "received private source did not settle: {row:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn summary(row: &Value) -> &Value {
    if row["result"].is_null() {
        &row["problem"]["signal"]
    } else {
        &row["result"]
    }
}
pub(crate) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut Vec<Peer>,
    cid: &str,
    algorithm: &str,
    moved: &OperationRecord,
) {
    let mut peers = vec![native.remove(0)];
    // Original source/controller credentials are already revoked. Invite with
    // a fresh manager credential, not the independent native primary's grant.
    let manager_config = party_with_scope(
        f,
        &f.client.identity_id,
        154,
        vec!["audio".into()],
        Scope::operator(),
    );
    let mut manager = CanonicalClient::open(manager_config).await.unwrap();
    let remote = enroll(&f, &mut manager, cid, "received-private-remote", 151, true).await;
    let candidate = enroll_kind_with_scope(
        &f,
        &mut manager,
        cid,
        "received-private-candidate",
        152,
        false,
        false,
        IdentityKind::Human,
        Some(signal_scope()),
    )
    .await;
    let public_config = party_with_scope(
        &f,
        &candidate.identity,
        153,
        vec!["audio".into()],
        signal_scope(),
    );
    let mut public = CanonicalClient::open(public_config.clone()).await.unwrap();
    let public_answer = negotiate_named(
        &mut public,
        cid,
        "received-public-offer",
        "received-public-sibling",
        "received-public-audio",
    )
    .await;
    wait_native_operation(&f, &public_config.credential, cid, "received-public-offer").await;
    stream_control(
        &mut public,
        cid,
        "received-public-sibling",
        "received-public-select",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":native[0].participant,"kinds":["audio"]}]}),
    )
    .await;
    peers.push(remote);
    peers.push(candidate);
    peers.push(Peer {
        client: public,
        config: Some(public_config),
        identity: peers[2].identity.clone(),
        participant: peers[2].participant.clone(),
        id: "received-public-sibling".into(),
        handle: handle(&public_answer),
        clock: tokio::time::Instant::now(),
        sequence: 0,
        live: true,
    });
    // Consultation planning requires observed live media incarnations for the
    // retained peer and existing outside peers, beyond negotiation metadata.
    probe(&mut peers, 1, &[], "received-public-retained-ready").await;
    probe(&mut peers, 3, &[], "received-public-sibling-ready").await;
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(&f);
    cfg.credential = key.token;
    let mut http = CanonicalHttpClient::open(cfg).await.unwrap();
    send_info(&peer).await;
    let (primary, rest) = peers.split_at_mut(1);
    let (baseline_primary, baseline_public) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(5), signal(&mut primary[0].client)),
        tokio::time::timeout(Duration::from_secs(5), signal(&mut rest[2].client))
    );
    let baseline_primary = baseline_primary.unwrap().unwrap();
    let baseline_public = baseline_public.unwrap().unwrap();
    assert_eq!(
        baseline_primary.scope(WireScope::Operation),
        baseline_public.scope(WireScope::Operation)
    );
    let baseline = settled(
        &mut http,
        baseline_primary.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(baseline["state"], "completed");
    assert_eq!(
        summary(&baseline)["recipients"].as_array().unwrap().len(),
        2
    );
    let group = begin(&f, &mut peers, cid, "received-native-private", 0, 1, 2).await;
    let outside:String=db.query_row("SELECT json_extract(selection_json,'$.outside_sip') FROM native_consultation_attempts WHERE operation_record_id=?1",[&group.record],|r|r.get(0)).unwrap();
    let outside: Value = serde_json::from_str(&outside).unwrap();
    assert_eq!(outside.as_array().unwrap().len(), 1);
    assert_eq!(outside[0]["source_kind"], "established-sip");
    assert_eq!(outside[0]["record_id"], moved.record_id);
    assert_eq!(outside[0]["generation"], 2);
    assert!(outside[0].get("endpoint_credential_id").is_none());
    // Prove both actual private endpoints and their strict native audio boundary
    // before observing SIP input; no fabricated active private registration.
    let _ = sip_audio(peer, 0, Duration::from_millis(500)).await;
    let (_, primary_leak) = tokio::join!(
        probe(&mut peers, 0, &[2], "received-private-primary"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert_eq!(
        primary_leak, 0,
        "private primary audio reached outside moved SIP call"
    );
    let (_, candidate_leak) = tokio::join!(
        probe(&mut peers, 2, &[0], "received-private-candidate"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert_eq!(
        candidate_leak, 0,
        "private candidate audio reached outside moved SIP call"
    );
    let (same_participant, same_identity): (String, String) = db
        .query_row(
            "SELECT participant_id,identity_id FROM connections WHERE id=?1",
            [&peers[2].id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(same_participant, peers[3].participant);
    assert_eq!(same_identity, peers[3].identity);
    let primary_id = peers[0].id.clone();
    let remote_id = peers[1].id.clone();
    let candidate_id = peers[2].id.clone();
    let public_id = peers[3].id.clone();
    let (packet, address) = send_info(&peer).await;
    let (primary, others) = peers.split_at_mut(1);
    let (_, candidate_public) = others.split_at_mut(1);
    let (candidate, public) = candidate_public.split_at_mut(1);
    let mut private_primary = Box::pin(signal(&mut primary[0].client));
    let mut private_candidate = Box::pin(signal(&mut candidate[0].client));
    let public_frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut public[0].client))
        .await
        .unwrap()
        .unwrap();
    let private_operation = public_frame.scope(WireScope::Operation).unwrap().to_owned();
    let private_status = settled(&mut http, &private_operation, cid).await;
    assert_eq!(private_status["state"], "completed");
    let recipients = summary(&private_status)["recipients"].as_array().unwrap();
    assert_eq!(
        recipients.len(),
        1,
        "unexpected physical recipients: {private_status}"
    );
    // Both private physical slots are refused before recipient freeze. A
    // public sibling of the candidate keeps only its own public authority.
    assert!(!recipients
        .iter()
        .any(|r| r["connection_id"] == candidate_id || r["connection_id"] == primary_id));
    let recipient = recipients
        .iter()
        .find(|r| r["connection_id"] == public_id)
        .unwrap();
    assert_eq!(recipient["disposition"], "local_submitted");
    assert_eq!(recipient["frame_id"], public_frame.id());
    assert!(
        tokio::time::timeout(Duration::from_millis(200), private_primary.as_mut())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), private_candidate.as_mut())
            .await
            .is_err()
    );
    let mut public_next = Box::pin(signal(&mut public[0].client));
    let mut cfg = canonical_http_config(&f);
    cfg.credential = group.key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let command = TransitionCommand {
        operation_id: "return-received-native-private".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(primary_id.clone()),
        expected_revision: revision(&f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: group.record.clone(),
            retained_connid: remote_id,
        },
        payload_extensions: Default::default(),
    };
    controller.transition(command.clone()).await.unwrap();
    let returned = wait_state(&f, &group.key.id, &command.operation_id, true).await;
    assert_eq!(
        returned.result.as_ref().unwrap()["retired_connection_ids"],
        json!([candidate_id])
    );
    assert_eq!(returned.result.as_ref().unwrap()["remote_continuing"], true);
    assert_eq!(returned.effects.len(), 1);
    assert_eq!(returned.effects[0].attempt, 1);
    assert_eq!(
        controller.transition(command).await.unwrap().result,
        returned.result
    );
    peer.socket
        .send_to(packet.as_bytes(), address)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), private_primary.as_mut())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), public_next.as_mut())
            .await
            .is_err()
    );
    send_rfc(&peer).await;
    let (recovered_primary, recovered_public) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(5), private_primary.as_mut()),
        tokio::time::timeout(Duration::from_secs(5), public_next.as_mut())
    );
    let recovered_primary = recovered_primary.unwrap().unwrap();
    let recovered_public = recovered_public.unwrap().unwrap();
    drop(private_primary);
    drop(public_next);
    assert_eq!(
        recovered_primary.scope(WireScope::Operation),
        recovered_public.scope(WireScope::Operation)
    );
    assert_ne!(
        recovered_primary.scope(WireScope::Operation),
        Some(private_operation.as_str())
    );
    let recovered = settled(
        &mut http,
        recovered_primary.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(recovered["state"], "completed");
    assert_eq!(
        summary(&recovered)["recipients"].as_array().unwrap().len(),
        2
    );
    // Logical retirement may leave the physical control peer open. Check its
    // pending reader for any leaked signal through return/recovery, then end
    // that reader only after the private Connection is durably retired.
    assert!(!matches!(
        tokio::time::timeout(Duration::from_millis(200), private_candidate.as_mut()).await,
        Ok(Some(_))
    ));
    drop(private_candidate);
    assert_eq!(
        settled(&mut http, &private_operation, cid).await,
        private_status
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        3
    );
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_MOVED_DTMF_PRIVATE_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"baseline":baseline,"baseline_frames":[baseline_primary.original(),baseline_public.original()],"private":private_status,"private_frame":public_frame.original(),"recovered":recovered,"recovered_frames":[recovered_primary.original(),recovered_public.original()],"private_primary":primary_id,"private_candidate":candidate_id,"public_sibling":public_id,"same_participant":same_participant,"calls":peer.unique(),"moved_record":moved.record_id,"outside":outside,"private_audio_marked":[primary_leak,candidate_leak]})).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
    // Retain every live client through the host's explicit shutdown below.
    native.extend(peers);
}
