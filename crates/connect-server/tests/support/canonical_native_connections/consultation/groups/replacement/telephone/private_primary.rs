//! Actual selected SIP-primary media and signal custody through public clients.
use super::received::privacy::{settled, signal, signal_scope};
use super::received::{send_info, send_rfc};
use super::retained::RetainedCall;
use super::*;
use rvoip_uctp::v2::Scope as WireScope;

pub(super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    mut peers: Vec<Peer>,
    cid: &str,
    algorithm: &str,
    call: RetainedCall<'_>,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (phone, kind, generation) = match call {
        RetainedCall::Initial => (db.query_row("SELECT o.connection_id FROM operations o JOIN initial_telephone_operations n ON n.operation_record_id=o.record_id",[],|r|r.get::<_,String>(0)).unwrap(),"initial-sip",1),
        RetainedCall::Established(moved) => {
            let id=moved.result.as_ref().unwrap()["connection_id"].as_str().unwrap();
            assert_ne!(Some(id),moved.connection_id.as_deref());
            (id.into(),"established-sip",2)
        }
    };
    let prior_move = if let RetainedCall::Established(moved) = call {
        let (controller_revoked,expires):(bool,i64)=db.query_row("SELECT k.revoked,n.expires_at FROM native_replacements n JOIN credentials k ON k.id=n.credential_id WHERE n.operation_record_id=?1",[&moved.record_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let source_credential_refused = matches!(f.server.store.authenticate(&f.client.credential),Err(e) if e.code=="unauthorized");
        let preparation_expired = chrono::Utc::now().timestamp() > expires;
        let lifetime_current:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN connections c ON c.id=l.connection_id AND c.generation=l.generation AND c.observed_runtime_id=l.runtime_epoch WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.generation=2 AND l.retired=0 AND c.state IN ('ready','active'))",rusqlite::params![moved.record_id,phone],|r|r.get(0)).unwrap();
        assert!(
            controller_revoked
                && source_credential_refused
                && preparation_expired
                && lifetime_current
        );
        json!({"record_id":moved.record_id,"controller_revoked":controller_revoked,"source_credential_refused":source_credential_refused,"preparation_expired":preparation_expired,"lifetime_current":lifetime_current})
    } else {
        Value::Null
    };
    let (identity, participant): (String, String) = db
        .query_row(
            "SELECT identity_id,participant_id FROM connections WHERE id=?1",
            [&phone],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let key = f
        .server
        .store
        .issue(
            &identity,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = key.token;
    let mut source_status = CanonicalHttpClient::open(cfg).await.unwrap();
    send_info(peer).await;
    let baseline = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
        .await
        .unwrap()
        .unwrap();
    let baseline_status = settled(
        &mut source_status,
        baseline.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(baseline_status["state"], "completed");
    let mut manager = if matches!(call, RetainedCall::Established(_)) {
        Some(
            CanonicalClient::open(party_with_scope(
                f,
                &f.client.identity_id,
                172,
                vec!["audio".into()],
                Scope::operator(),
            ))
            .await
            .unwrap(),
        )
    } else {
        None
    };
    let candidate = enroll_kind_with_scope(
        f,
        manager.as_mut().unwrap_or(&mut peers[0].client),
        cid,
        "sip-primary-consultant",
        171,
        false,
        false,
        IdentityKind::Human,
        Some(signal_scope()),
    )
    .await;
    peers.push(candidate);
    let remote = peers[0].id.clone();
    let (consult, controller_key, record) = begin_connection_request(
        f,
        &mut peers,
        cid,
        "consult-selected-sip-primary",
        &identity,
        &participant,
        &phone,
        &remote,
        1,
    )
    .await;
    for p in &mut peers {
        p.client.start_signaling_reader().unwrap();
    }
    let selected: String = db
        .query_row(
            "SELECT selection_json FROM native_consultation_attempts WHERE operation_record_id=?1",
            [&record],
            |r| r.get(0),
        )
        .unwrap();
    let selected: Value = serde_json::from_str(&selected).unwrap();
    let selected_phone = selected["private"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["connection_id"] == phone)
        .unwrap();
    assert_eq!(selected_phone["source_kind"], kind);
    assert_eq!(selected_phone["generation"], generation);
    assert!(
        selected_phone.get("key_id").is_none() && selected_phone.get("credential_id").is_none()
    );
    assert_eq!(selected["remote"]["connection_id"], remote);
    // Sample every native physical receiver. The held original peer must stay
    // silent while the actual SIP primary publishes to its own consultant.
    let receive = async {
        let until = tokio::time::Instant::now() + Duration::from_millis(900);
        let mut counts = vec![0usize; peers.len()];
        loop {
            let reads = peers
                .iter_mut()
                .enumerate()
                .map(|(i, p)| {
                    Box::pin(async move { (i, p.client.receive_native_rtp_datagram().await) })
                })
                .collect::<Vec<_>>();
            let Ok(((i, bytes), _, _)) = tokio::time::timeout_at(until, select_all(reads)).await
            else {
                break;
            };
            let packet = rvoip_uctp::substrate::unpack_rtp_datagram(&bytes.unwrap()).unwrap();
            assert_eq!(packet.stream_local_id, peers[i].handle);
            if packet
                .rtp
                .payload
                .iter()
                .any(|b| !matches!(*b, 0xff | 0x7f))
            {
                counts[i] += 1;
            }
        }
        counts
    };
    let (_, counts) = tokio::join!(peer.rtp(0), receive);
    assert_eq!(
        counts[0], 0,
        "private SIP source leaked to held native remote"
    );
    assert!(
        counts[1] > 10,
        "private SIP input did not reach registered consultant: {counts:?}"
    );
    let (_, marked) = tokio::join!(
        probe(&mut peers, 1, &[], "sip-primary-private-native-source"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert!(
        marked > 10,
        "consultant input did not reach actual SIP primary: {marked}"
    );
    wait_state(f, &controller_key.id, &consult.operation_id, true).await;
    let mut private_frames = vec![];
    let mut private_statuses = vec![];
    let mut old_info = None;
    for source in ["sip-info", "rfc4733"] {
        if source == "sip-info" {
            old_info = Some(send_info(peer).await);
        } else {
            send_rfc(peer).await;
        }
        let frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[1].client))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            frame.scope(WireScope::Connection),
            Some(peers[1].id.as_str())
        );
        assert_eq!(frame.original()["payload"]["source"], source);
        assert_eq!(frame.original()["payload"]["digits"], "5");
        let status = settled(
            &mut source_status,
            frame.scope(WireScope::Operation).unwrap(),
            cid,
        )
        .await;
        assert_eq!(status["state"], "completed");
        assert_eq!(status["result"]["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(
            status["result"]["recipients"][0]["connection_id"],
            peers[1].id
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[0].client))
                .await
                .is_err(),
            "private SIP digit leaked to held native remote"
        );
        private_frames.push(frame.original().clone());
        private_statuses.push(status);
    }
    let info_before = peer.info.lock().unwrap().len();
    let candidate_id = peers[1].id.clone();
    peers[1]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(candidate_id.clone()),
                operation_id: Some("private-consultant-to-sip-primary".into()),
                ..Default::default()
            },
            "9",
            100,
            "info",
        )
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if peer.info.lock().unwrap().len() == info_before + 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "private consultant digit did not reach actual SIP primary"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        String::from_utf8_lossy(peer.info.lock().unwrap().last().unwrap()).contains("Signal=9")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[0].client))
            .await
            .is_err()
    );
    let mut native_cfg = canonical_http_config(f);
    native_cfg.credential = peers[1].config.as_ref().unwrap().credential.clone();
    let mut native_http = CanonicalHttpClient::open(native_cfg).await.unwrap();
    let private_native_status =
        settled(&mut native_http, "private-consultant-to-sip-primary", cid).await;
    assert_eq!(private_native_status["state"], "completed");
    assert_eq!(
        private_native_status["result"]["recipients"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        private_native_status["result"]["recipients"][0]["connection_id"],
        phone
    );
    assert_eq!(
        private_native_status["result"]["recipients"][0]["disposition"],
        "local_submitted"
    );
    let mut cfg = canonical_http_config(f);
    cfg.credential = controller_key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let original = controller.transition(consult.clone()).await.unwrap();
    let returning = TransitionCommand {
        operation_id: "return-selected-sip-primary".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(phone.clone()),
        expected_revision: revision(f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: record.clone(),
            retained_connid: remote.clone(),
        },
        payload_extensions: Default::default(),
    };
    controller.transition(returning.clone()).await.unwrap();
    let returned = wait_state(f, &controller_key.id, &returning.operation_id, true).await;
    assert_eq!(returned.result.as_ref().unwrap()["remote_continuing"], true);
    assert_eq!(
        returned.result.as_ref().unwrap()["retired_connection_ids"],
        json!([candidate_id])
    );
    assert_eq!(returned.effects.len(), 1);
    assert_eq!(returned.effects[0].attempt, 1);
    assert_eq!(
        controller.transition(returning).await.unwrap().result,
        returned.result
    );
    assert_eq!(
        controller.transition(consult).await.unwrap().result,
        original.result
    );
    peers[1].live = false;
    sip_native_duplex(
        peer,
        &mut peers[..1],
        0,
        "selected-sip-primary-original-return",
    )
    .await;
    let (packet, address) = old_info.unwrap();
    peer.socket
        .send_to(packet.as_bytes(), address)
        .await
        .unwrap();
    send_rfc(peer).await; // Exact old event retransmission must keep its old audience.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[0].client))
            .await
            .is_err(),
        "old private input replay expanded into restored audience"
    );
    super::received::send_next_rfc(peer).await;
    let restored = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.original()["payload"]["source"], "rfc4733");
    assert!(private_frames
        .iter()
        .all(|p| p["operation_id"] != restored.original()["operation_id"]));
    let restored_status = settled(
        &mut source_status,
        restored.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(restored_status["state"], "completed");
    for (frame, status) in private_frames.iter().zip(&private_statuses) {
        let operation = frame["operation_id"].as_str().unwrap();
        assert_eq!(&settled(&mut source_status, operation, cid).await, status);
    }
    let source_operation_count: u64 = db
        .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        source_operation_count, 4,
        "replayed private SIP input created a new logical operation"
    );
    assert_eq!(peer.unique(), 1);
    if let Some(path) = std::env::var_os("CONNECT_PRIVATE_SIP_PRIMARY_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let row = json!({"algorithm":algorithm,"source_kind":kind,"prior_move":prior_move,"source_operation_count":source_operation_count,"private_native_status":private_native_status,"phone":phone,"retained":remote,"selection":selected,"baseline":baseline_status,"baseline_frame":baseline.original(),"private_frames":private_frames,"private_statuses":private_statuses,"returned":returned,"restored":restored_status,"restored_frame":restored.original(),"private_sip_to_native_marked":counts,"private_native_to_sip_marked":marked,"native_private_info_count":peer.info.lock().unwrap().len()-info_before,"calls":peer.unique()});
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap(),
            "{row}"
        )
        .unwrap();
    }
    for p in &mut peers {
        p.client.close();
    }
    if let Some(manager) = &mut manager {
        manager.close();
    }
}
