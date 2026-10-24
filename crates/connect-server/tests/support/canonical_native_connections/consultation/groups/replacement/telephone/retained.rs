//! The same actual retained-call assertions for initial and committed SIP.
use super::received::privacy::{settled, signal, signal_scope};
use super::received::{send_info, send_rfc};
use super::*;
use rusqlite::params;
use rvoip_uctp::v2::Scope as WireScope;

#[derive(Clone, Copy)]
pub(super) enum RetainedCall<'a> {
    Initial,
    Established(&'a OperationRecord),
}

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
    let (phone, source_kind, selected_kind, generation, consult_operation, return_operation, moved_record) = match call {
        RetainedCall::Initial => (
            db.query_row("SELECT o.connection_id FROM operations o JOIN initial_telephone_operations n ON n.operation_record_id=o.record_id",[],|r|r.get::<_,String>(0)).unwrap(),
            "sip-initial", "initial-sip", 1, "consult-retained-initial-sip", "return-retained-initial-sip", None,
        ),
        RetainedCall::Established(moved) => (
            moved.result.as_ref().unwrap()["connection_id"].as_str().unwrap().to_owned(), "sip-established", "established-sip", 2,
            "consult-retained-established-sip", "return-retained-established-sip", Some(moved.record_id.as_str()),
        ),
    };
    let replaced_native = match call {
        RetainedCall::Initial => None,
        RetainedCall::Established(moved) => {
            let source = moved.connection_id.as_ref().unwrap();
            assert_eq!(moved.result.as_ref().unwrap()["replaces"], source.as_str());
            assert_ne!(&phone, source);
            Some(source.as_str())
        }
    };
    let prior_move = if let Some(record) = moved_record {
        let (controller_revoked,expires): (bool,i64) = db.query_row("SELECT k.revoked,n.expires_at FROM native_replacements n JOIN credentials k ON k.id=n.credential_id WHERE n.operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let source_credential_refused = matches!(f.server.store.authenticate(&f.client.credential),Err(error) if error.code == "unauthorized");
        let preparation_expired = chrono::Utc::now().timestamp() > expires;
        let lifetime_current: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN connections c ON c.id=l.connection_id AND c.generation=l.generation AND c.observed_runtime_id=l.runtime_epoch WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.generation=2 AND l.retired=0 AND c.state IN ('ready','active'))",params![record,phone],|r|r.get(0)).unwrap();
        assert!(
            controller_revoked
                && source_credential_refused
                && preparation_expired
                && lifetime_current
        );
        json!({"controller_revoked":controller_revoked,"source_credential_refused":source_credential_refused,"preparation_expired":preparation_expired,"lifetime_current":lifetime_current})
    } else {
        Value::Null
    };
    let phone_identity: String = db
        .query_row(
            "SELECT identity_id FROM connections WHERE id=?1",
            [&phone],
            |r| r.get(0),
        )
        .unwrap();
    let key = f
        .server
        .store
        .issue(
            &phone_identity,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = key.token;
    let mut status = CanonicalHttpClient::open(cfg).await.unwrap();
    // A committed move has already revoked its original controller. Only its
    // fresh current manager may invite; the independent native primary remains
    // the private media endpoint and signs its own commands.
    let mut manager = if matches!(call, RetainedCall::Established(_)) {
        Some(
            CanonicalClient::open(party_with_scope(
                f,
                &f.client.identity_id,
                162,
                vec!["audio".into()],
                Scope::operator(),
            ))
            .await
            .unwrap(),
        )
    } else {
        None
    };
    send_info(peer).await;
    let baseline = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
        .await
        .unwrap()
        .unwrap();
    let baseline_status = settled(
        &mut status,
        baseline.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(baseline_status["state"], "completed");
    let candidate = enroll_kind_with_scope(
        f,
        manager.as_mut().unwrap_or(&mut peers[0].client),
        cid,
        "retained-sip-consultant",
        161,
        false,
        false,
        IdentityKind::Human,
        Some(signal_scope()),
    )
    .await;
    peers.push(candidate);
    let (consult, controller_key, record) =
        begin_request(f, &mut peers, cid, consult_operation, 0, &phone, 1).await;
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
    assert_eq!(selected["remote"]["source_kind"], selected_kind);
    assert_eq!(selected["remote"]["generation"], generation);
    if let Some(record) = moved_record {
        assert_eq!(selected["remote"]["record_id"], record);
        assert!(selected["remote"].get("endpoint_credential_id").is_none());
    }
    assert_eq!(selected["remote"]["connection_id"], phone);
    assert!(selected["remote"].get("key_id").is_none());
    for (source, target, name) in [
        (0, 1, "retained-primary-private"),
        (1, 0, "retained-consultant-private"),
    ] {
        let targets = [target];
        let (_, leak) = tokio::join!(
            probe(&mut peers, source, &targets, name),
            sip_audio(peer, 0, Duration::from_millis(2600))
        );
        assert_eq!(leak, 0, "private audio reached held SIP call");
    }
    let before: u64 = db
        .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    let (held_packet, held_address) = send_info(peer).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(4);
    let held_operation = loop {
        let count: u64 = db
            .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
                r.get(0)
            })
            .unwrap();
        if count == before + 1 {
            break db.query_row("SELECT o.operation_id FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id ORDER BY s.rowid DESC LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "held SIP input was not durably captured"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let held_status = settled(&mut status, &held_operation, cid).await;
    let submitted:u64=db.query_row("SELECT count(*) FROM received_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1 AND r.disposition IN ('local_submitted','outcome_unknown')",[&held_operation],|r|r.get(0)).unwrap();
    assert_eq!(
        submitted, 0,
        "held remote signal entered private audience: {held_status}"
    );
    let (a, b) = peers.split_at_mut(1);
    let (a, b) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut a[0].client)),
        tokio::time::timeout(Duration::from_millis(200), signal(&mut b[0].client))
    );
    assert!(
        a.is_err() && b.is_err(),
        "held SIP signal reached private pair"
    );
    let info_before = peer.info.lock().unwrap().len();
    let primary_id = peers[0].id.clone();
    let private_send = peers[0]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some(primary_id.clone()),
                operation_id: Some("private-native-digit".into()),
                ..Default::default()
            },
            "9",
            100,
            "info",
        )
        .await
        .unwrap();
    assert_ne!(
        private_send.state,
        rvoip_uctp::v2::payloads::OperationState::Failed
    );
    let private_frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[1].client))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(private_frame.original()["payload"]["digits"], "9");
    assert_eq!(
        private_frame.scope(WireScope::Operation),
        Some("private-native-digit")
    );
    let mut cfg = canonical_http_config(f);
    cfg.credential = controller_key.token.clone();
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let original = controller.transition(consult.clone()).await.unwrap();
    assert_eq!(
        peer.info.lock().unwrap().len(),
        info_before,
        "private native digit leaked to held SIP call"
    );
    let returning = TransitionCommand {
        operation_id: return_operation.into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        connection_id: Some(peers[0].id.clone()),
        expected_revision: revision(f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: record.clone(),
            retained_connid: phone.clone(),
        },
        payload_extensions: Default::default(),
    };
    controller.transition(returning.clone()).await.unwrap();
    let returned = wait_state(f, &controller_key.id, &returning.operation_id, true).await;
    assert_eq!(returned.result.as_ref().unwrap()["remote_continuing"], true);
    assert_eq!(
        returned.result.as_ref().unwrap()["retired_connection_ids"],
        json!([peers[1].id])
    );
    assert_eq!(returned.effects.len(), 1);
    assert_eq!(returned.effects[0].attempt, 1);
    assert_eq!(
        controller
            .transition(returning.clone())
            .await
            .unwrap()
            .result,
        returned.result
    );
    assert_eq!(
        controller.transition(consult).await.unwrap().result,
        original.result
    );
    peers[1].live = false;
    sip_native_duplex(peer, &mut peers[..1], 0, "retained-original-return").await;
    peer.socket
        .send_to(held_packet.as_bytes(), held_address)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[0].client))
            .await
            .is_err()
    );
    send_rfc(peer).await;
    let recovered = tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        recovered.scope(WireScope::Operation),
        Some(held_operation.as_str())
    );
    assert_eq!(recovered.original()["payload"]["source"], "rfc4733");
    let recovered_status = settled(
        &mut status,
        recovered.scope(WireScope::Operation).unwrap(),
        cid,
    )
    .await;
    assert_eq!(recovered_status["state"], "completed");
    assert_eq!(
        settled(&mut status, &held_operation, cid).await,
        held_status
    );
    assert!(!matches!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut peers[1].client)).await,
        Ok(Some(_))
    ));
    assert_eq!(peer.unique(), 1);
    let source_count: u64 = db
        .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(source_count, before + 2);
    if let Some(path) = std::env::var_os("CONNECT_RETAINED_SIP_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let row = json!({"algorithm":algorithm,"source_kind":source_kind,"moved_record":moved_record,"replaced_native":replaced_native,"prior_move":prior_move,"remote":phone,"selection":selected,"baseline":baseline_status,"baseline_frame":baseline.original(),"held":held_status,"held_operation":held_operation,"private_frame":private_frame.original(),"returned":returned,"recovered":recovered_status,"recovered_frame":recovered.original(),"calls":peer.unique(),"held_submitted":submitted,"source_operation_count":source_count,"info_count_private_before":info_before,"info_count_private_after":peer.info.lock().unwrap().len()});
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
