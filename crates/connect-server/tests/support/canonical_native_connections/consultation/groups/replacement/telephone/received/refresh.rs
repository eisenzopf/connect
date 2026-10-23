//! Actual native credential rotation with one frozen SIP-to-native recipient.
use super::*;
use rusqlite::OptionalExtension;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_moved_sip_dtmf_refresh_preserves_queued_epoch_and_fresh_bidirectional_delivery()
{
    for algorithm in ["EdDSA", "ES256"] {
        telephone_case_profile(Stop::DtmfRefresh, true, algorithm).await;
    }
}
fn successor(f: &Fixture, algorithm: &str) -> (String, String, String, serde_json::Value) {
    use base64::Engine;
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let (old,key,device):(String,String,String)=db.query_row("SELECT credential_id,key_id,device_id FROM native_admission_sources WHERE connection_id='codec-source'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let actor = f.server.store.revalidate(&old).unwrap();
    let grant = f
        .server
        .store
        .issue(
            &actor.identity_id,
            actor.scope.clone(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let public = f
        .server
        .store
        .resolve_peer_signing_key(&old, "tnt_default", &key, algorithm)
        .unwrap();
    let revision = db
        .query_row(
            "SELECT revision FROM peer_signing_keys WHERE key_id=?1",
            [&key],
            |r| r.get(0),
        )
        .unwrap();
    let facts:serde_json::Value=db.query_row("SELECT json_object('connection_id',n.connection_id,'record_id',n.operation_record_id,'generation',n.generation,'participant_id',n.participant_id,'membership_epoch',n.membership_epoch,'endpoint_id',n.endpoint_id,'stream_revision',json_extract(c.readiness_json,'$.stream_revision')) FROM native_admission_sources n JOIN connections c ON c.id=n.connection_id WHERE n.connection_id='codec-source'",[],|r|{let value:String=r.get(0)?;Ok(serde_json::from_str(&value).unwrap())}).unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    f.server
        .store
        .execute_provisioning(
            &admin.credential_id,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-actual-dtmf-successor".into(),
                identity_id: actor.identity_id,
                credential_id: grant.id.clone(),
                device_id: device,
                key_id: key,
                algorithm: if algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(public.public_key),
                not_before: public.not_before,
                expires_at: public.expires_at,
                expected_revision: Some(revision),
            },
        )
        .unwrap();
    (old, grant.id, grant.token, facts)
}
async fn status(
    http: &mut CanonicalHttpClient,
    cid: &str,
    operation: &str,
    wanted: rvoip_uctp::v2::payloads::OperationState,
) -> rvoip_uctp::v2::payloads::OperationUpdated {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row = http
            .operation_get(
                operation,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if row.state == wanted {
            return row;
        }
        assert!(
            row.state == rvoip_uctp::v2::payloads::OperationState::Preparing
                && tokio::time::Instant::now() < deadline,
            "{}",
            serde_json::to_string(&row).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn observe(value: serde_json::Value) {
    if let Some(path) = std::env::var_os("CONNECT_DTMF_REFRESH_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
}
pub(crate) async fn exercise(
    f: &Fixture,
    cid: &str,
    peers: &mut [Peer],
    phone: &PeerSip,
    payload: u8,
    moved: &OperationRecord,
    algorithm: &str,
) {
    let (old, next, token, before) = successor(f, algorithm);
    let reader = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = reader.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    db.execute_batch("CREATE TRIGGER pause_original_received_frame BEFORE UPDATE OF attempted ON received_dtmf_recipients WHEN NEW.attempted=1 BEGIN SELECT RAISE(ABORT,'controlled-epoch-reservation-pause'); END;").unwrap();
    send_info(phone).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let (original, registration, frozen) = loop {
        let row:Option<(String,String,String)>=db.query_row("SELECT o.operation_id,r.registration_id,r.target_json FROM received_dtmf_sources s JOIN operations o ON o.record_id=s.operation_record_id JOIN received_dtmf_recipients r ON r.operation_record_id=s.operation_record_id WHERE s.parent_record_id=?1 AND o.state='preparing' AND r.attempted=0",[&moved.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().unwrap();
        if let Some(row) = row {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "original received input was not frozen"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let original_snapshot: serde_json::Value = serde_json::from_str(&frozen).unwrap();
    assert_eq!(original_snapshot["credential_epoch"], 0);
    assert_eq!(original_snapshot["credential_id"], old);
    let rotate_at = tokio::time::Instant::now();
    let authentication = peers[0].client.refresh_credential(token).await.unwrap();
    assert_eq!(authentication.identity_id, f.peer.identity_id);
    assert!(
        rotate_at.elapsed() < Duration::from_millis(800),
        "rotation crossed the original one-second native metadata budget: {:?}",
        rotate_at.elapsed()
    );
    let received_at:String=db.query_row("SELECT json_extract(s.observation_json,'$.received_at') FROM received_dtmf_sources s JOIN operations o ON o.record_id=s.operation_record_id WHERE o.operation_id=?1",[&original],|r|r.get(0)).unwrap();
    let input_age_ms = (chrono::Utc::now()
        - chrono::DateTime::parse_from_rfc3339(&received_at)
            .unwrap()
            .with_timezone(&chrono::Utc))
    .num_milliseconds();
    assert!(
        (0..800).contains(&input_age_ms),
        "rotation did not precede the original native admission deadline: {input_age_ms}ms"
    );
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    f.server.store.revoke(&admin, &old).unwrap();
    db.execute_batch("DROP TRIGGER pause_original_received_frame")
        .unwrap();
    let refused = status(
        &mut http,
        cid,
        &original,
        rvoip_uctp::v2::payloads::OperationState::Failed,
    )
    .await;
    assert_eq!(
        refused.problem.as_ref().unwrap()["signal"]["recipients"][0]["disposition"],
        "not_started"
    );
    assert!(refused.problem.as_ref().unwrap()["signal"]["recipients"][0]["frame_id"].is_null());
    let after:serde_json::Value=db.query_row("SELECT json_object('connection_id',n.connection_id,'record_id',n.operation_record_id,'generation',n.generation,'participant_id',n.participant_id,'membership_epoch',n.membership_epoch,'endpoint_id',n.endpoint_id,'stream_revision',json_extract(c.readiness_json,'$.stream_revision')) FROM native_admission_sources n JOIN connections c ON c.id=n.connection_id WHERE n.connection_id='codec-source'",[],|r|{let value:String=r.get(0)?;Ok(serde_json::from_str(&value).unwrap())}).unwrap();
    assert_eq!(before, after);
    assert_eq!(db.query_row("SELECT target_json FROM received_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1",[&original],|r|r.get::<_,String>(0)).unwrap(),frozen);
    assert_eq!(
        db.query_row(
            "SELECT credential_id FROM native_admission_sources WHERE connection_id='codec-source'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        next
    );
    assert_eq!(
        db.query_row(
            "SELECT credential_id FROM native_offer_operations WHERE operation_record_id=?1",
            [before["record_id"].as_str().unwrap()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        old
    );
    let first = {
        let receive = signal(&mut peers[0].client);
        tokio::pin!(receive);
        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut receive)
                .await
                .is_err(),
            "queued old epoch was retargeted to the new credential"
        );
        send_info(phone).await;
        tokio::time::timeout(Duration::from_secs(5), &mut receive)
            .await
            .unwrap()
    };
    let mut deliveries = Vec::new();
    for (index, kind) in ["sip-info", "rfc4733"].into_iter().enumerate() {
        let frame = if index == 0 {
            first.clone()
        } else {
            send_rfc(phone).await;
            tokio::time::timeout(Duration::from_secs(5), signal(&mut peers[0].client))
                .await
                .unwrap()
        };
        assert_eq!(frame.original()["payload"]["source"], kind);
        assert_eq!(frame.original()["payload"]["duration_ms"], 237);
        assert_eq!(frame.original()["payload"]["digits"], "5");
        let operation = frame.scope(rvoip_uctp::v2::Scope::Operation).unwrap();
        assert_ne!(operation, original);
        let done = status(
            &mut http,
            cid,
            operation,
            rvoip_uctp::v2::payloads::OperationState::Completed,
        )
        .await;
        let (id,target):(String,String)=db.query_row("SELECT r.registration_id,r.target_json FROM received_dtmf_recipients r JOIN operations o ON o.record_id=r.operation_record_id WHERE o.operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_ne!(id, registration);
        let target: serde_json::Value = serde_json::from_str(&target).unwrap();
        assert_eq!(target["credential_epoch"], 1);
        assert_eq!(target["credential_id"], next);
        assert_eq!(target["connection_id"], before["connection_id"]);
        assert_eq!(
            done.result.as_ref().unwrap()["recipients"][0]["frame_id"],
            frame.id()
        );
        deliveries.push(json!({"operation":done,"native_frame":frame.original(),"target":target,"registration":id}));
    }
    let outgoing = "native-to-moved-after-refresh";
    let accepted = peers[0]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
                connection_id: Some("codec-source".into()),
                operation_id: Some(outgoing.into()),
                ..Default::default()
            },
            "7",
            100,
            "info",
        )
        .await
        .unwrap();
    assert!(matches!(
        accepted.state,
        rvoip_uctp::v2::payloads::OperationState::Preparing
            | rvoip_uctp::v2::payloads::OperationState::Completed
    ));
    let outgoing_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let done = loop {
        let row = peers[0]
            .client
            .operation_get(
                outgoing,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if row.state == rvoip_uctp::v2::payloads::OperationState::Completed {
            break row;
        }
        assert!(
            row.state == rvoip_uctp::v2::payloads::OperationState::Preparing
                && tokio::time::Instant::now() < outgoing_deadline,
            "{}",
            serde_json::to_string(&row).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let source:String=db.query_row("SELECT d.observation_json FROM native_dtmf_sources d JOIN operations o ON o.record_id=d.operation_record_id WHERE o.operation_id=?1",[outgoing],|r|r.get(0)).unwrap();
    let source: serde_json::Value = serde_json::from_str(&source).unwrap();
    assert_eq!(source["credential_revision"], 2);
    assert_eq!(source["signing_algorithm"], algorithm);
    let packets = phone.info.lock().unwrap().clone();
    assert_eq!(packets.len(), 1);
    let Message::Request(request) = parse_message(&packets[0]).unwrap() else {
        panic!()
    };
    assert_eq!(
        std::str::from_utf8(&request.body).unwrap(),
        "Signal=7\r\nDuration=100\r\n"
    );
    assert_eq!(
        serde_json::to_value(
            http.operation_get(
                &original,
                CommandScope {
                    conversation_id: Some(cid.into()),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&refused).unwrap()
    );
    observe(
        json!({"algorithm":algorithm,"before":before,"after":after,"queued":refused,"queued_target":original_snapshot,"queued_registration":registration,"received":deliveries,"outgoing":done,"outgoing_source":source,"current_credential":next,"sip_calls":phone.unique(),"input_age_at_refresh_ms":input_age_ms}),
    );
    assert_eq!(phone.unique(), 1);
    sip_native_duplex(phone, peers, payload, "moved-sip-after-dtmf-auth-refresh").await;
}
