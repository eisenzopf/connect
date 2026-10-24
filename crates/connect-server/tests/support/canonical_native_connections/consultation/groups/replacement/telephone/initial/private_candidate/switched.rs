//! Actual original SIP candidate, its distinct public sibling and fresh return.
use super::super::super::received::privacy::{settled, signal};
use super::super::super::received::{send_info, send_rfc};
use super::*;
use rvoip_uctp::v2::Scope as WireScope;

pub(super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    clients: &mut Vec<Peer>,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    controller_key: &str,
    mut observed: Value,
    public_media: Option<std::net::SocketAddr>,
    public_dialog: Option<Dialog>,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let cid = observed["conversation_id"].as_str().unwrap().to_owned();
    let record = observed["record"].as_str().unwrap().to_owned();
    let child = observed["child"].as_str().unwrap().to_owned();
    let phone = observed["connection"].as_str().unwrap().to_owned();
    let public = observed["public"][0].as_str().unwrap().to_owned();
    let private_media = *peer.media_target.lock().unwrap();
    assert!(private_media.is_some() && public_media.is_some() && private_media != public_media);
    assert_eq!(
        db.query_row(
            "SELECT kind FROM identities WHERE id=?1",
            [&f.peer.identity_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "human"
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let selected: Value = loop {
        let value: Option<String> = db.query_row("SELECT selection_json FROM native_consultation_attempts WHERE operation_record_id=?1 AND applied_json IS NOT NULL", [&record], |r|r.get(0)).optional().unwrap();
        if let Some(value) = value {
            break serde_json::from_str(&value).unwrap();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "original SIP candidate audience was not applied"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let selected_phone = selected["private"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["connection_id"] == phone)
        .unwrap()
        .clone();
    assert_eq!(selected_phone["source_kind"], "private-initial-sip");
    assert_eq!(selected_phone["record_id"], child);
    assert_eq!(selected_phone["runtime_connection_id"], observed["core"]);
    assert_eq!(selected_phone["sip_call_id"], observed["call"]);
    assert!(
        selected_phone.get("key_id").is_none() && selected_phone.get("credential_id").is_none()
    );
    assert!(!selected["private"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["connection_id"] == public));
    // Shared directional-authority exercise uses [held remote, native partner].
    clients.swap(0, 1);
    for p in clients.iter_mut() {
        p.client.start_signaling_reader().unwrap();
    }
    let private_counts = marked(peer, clients).await;
    assert_eq!(
        private_counts[0], 0,
        "private candidate leaked to held remote"
    );
    assert!(
        private_counts[1] > 10,
        "private SIP candidate did not reach its native primary"
    );
    *peer.media_target.lock().unwrap() = public_media;
    let sibling_counts = marked(peer, clients).await;
    assert_eq!(
        sibling_counts,
        vec![0, 0],
        "same Participant/Endpoint public SIP input entered private pair or held remote"
    );
    *peer.media_target.lock().unwrap() = private_media;
    drain_audio(peer).await;
    let (_, duplex) = tokio::join!(
        probe(clients, 1, &[], "original-private-sip-candidate-duplex"),
        audio_by_call(peer, private_media.unwrap(), public_media.unwrap())
    );
    assert!(
        duplex[0] > 10,
        "native primary did not reach original private SIP call"
    );
    assert_eq!(
        duplex[1], 0,
        "native private input reached public SIP sibling"
    );
    let completed = wait_state(f, controller_key, &command.operation_id, true).await;
    assert_eq!(completed.effects.len(), 1);
    assert_eq!(completed.effects[0].attempt, 1);
    assert_eq!(
        controller.transition(command.clone()).await.unwrap().result,
        completed.result
    );
    assert_eq!(peer.unique(), 2);

    let source_key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = source_key.token;
    let mut source_status = CanonicalHttpClient::open(cfg).await.unwrap();
    let mut frames = vec![];
    let mut statuses = vec![];
    let mut original_info = None;
    for source in ["sip-info", "rfc4733"] {
        if source == "sip-info" {
            original_info = Some(send_info(peer).await);
        } else {
            send_rfc(peer).await;
        }
        let frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut clients[1].client))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.original()["payload"]["source"], source);
        assert_eq!(
            frame.scope(WireScope::Connection),
            Some(clients[1].id.as_str())
        );
        assert_eq!(frame.original()["payload"]["digits"], "5");
        let status = settled(
            &mut source_status,
            frame.scope(WireScope::Operation).unwrap(),
            &cid,
        )
        .await;
        assert_eq!(status["state"], "completed");
        assert_eq!(status["result"]["recipients"].as_array().unwrap().len(), 1);
        assert_eq!(
            status["result"]["recipients"][0]["connection_id"],
            clients[1].id
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), signal(&mut clients[0].client))
                .await
                .is_err()
        );
        frames.push(frame.original().clone());
        statuses.push(status);
    }
    let authority = super::super::super::private_primary::authority::exercise(
        f,
        peer,
        clients,
        &cid,
        &selected_phone,
        &mut source_status,
    )
    .await;
    let info_before = peer.info.lock().unwrap().len();
    let primary = clients[1].id.clone();
    clients[1]
        .client
        .send_native_dtmf(
            CommandScope {
                conversation_id: Some(cid.clone()),
                session_id: Some("native-session".into()),
                connection_id: Some(primary.clone()),
                operation_id: Some("primary-to-original-private-sip-candidate".into()),
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
            "native primary INFO did not reach private SIP candidate"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let info = String::from_utf8(peer.info.lock().unwrap().last().unwrap().clone()).unwrap();
    assert!(info.contains("Signal=9"));
    assert_eq!(
        info.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("call-id"))
                .map(|(_, value)| value.trim())
        }),
        observed["call"].as_str()
    );
    let mut cfg = canonical_http_config(f);
    cfg.credential = clients[1].config.as_ref().unwrap().credential.clone();
    let mut primary_status = CanonicalHttpClient::open(cfg).await.unwrap();
    let sent = settled(
        &mut primary_status,
        "primary-to-original-private-sip-candidate",
        &cid,
    )
    .await;
    assert_eq!(sent["state"], "completed");
    assert_eq!(sent["result"]["recipients"].as_array().unwrap().len(), 1);
    assert_eq!(sent["result"]["recipients"][0]["connection_id"], phone);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), signal(&mut clients[0].client))
            .await
            .is_err()
    );

    let store = f.server.store.clone();
    let admin = store.authenticate(&f.admin).unwrap();
    let key = controller_key.to_owned();
    tokio::task::spawn_blocking(move || store.revoke(&admin, &key))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(controller.transition(command.clone()).await, Err(connect_client::ClientError::Application(ref e)) if e.code == "unauthorized" || e.code == "forbidden"),
        "revoked controller still replayed consultation"
    );
    let expires: i64 = db
        .query_row(
            "SELECT expires_at FROM operations WHERE record_id=?1",
            [&child],
            |r| r.get(0),
        )
        .unwrap();
    let remaining = (expires + 1 - chrono::Utc::now().timestamp()).max(0);
    assert!(remaining <= 31, "fixture admission deadline is not bounded");
    if remaining > 0 {
        tokio::time::sleep(Duration::from_secs(remaining as u64)).await;
    }
    assert!(chrono::Utc::now().timestamp() > expires);
    let after_expiry = marked(peer, clients).await;
    assert_eq!(after_expiry[0], 0);
    assert!(after_expiry[1] > 10);
    assert_eq!(
        peer.unique(),
        2,
        "expired admission/controller replacement redialed the original candidate"
    );

    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(f);
    cfg.credential = key.token;
    let mut returning = CanonicalHttpClient::open(cfg).await.unwrap();
    let return_command = TransitionCommand {
        operation_id: "return-original-private-sip-candidate".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        connection_id: Some(primary),
        expected_revision: revision(f),
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: record.clone(),
            retained_connid: clients[0].id.clone(),
        },
        payload_extensions: Default::default(),
    };
    returning.transition(return_command.clone()).await.unwrap();
    let returned = wait_state(f, &key.id, &return_command.operation_id, true).await;
    assert_eq!(
        returned.result.as_ref().unwrap()["retired_connection_ids"],
        json!([phone])
    );
    assert_eq!(returned.effects.len(), 1);
    assert_eq!(returned.effects[0].attempt, 1);
    assert_eq!(
        returning.transition(return_command).await.unwrap().result,
        returned.result
    );
    assert!(db.query_row("SELECT a.retired_at IS NOT NULL AND c.state='retired' FROM initial_telephone_answers a JOIN operations o ON o.record_id=a.operation_record_id JOIN connections c ON c.id=o.connection_id WHERE o.record_id=?1",[&child],|r|r.get::<_,bool>(0)).unwrap());
    assert!(db.query_row("SELECT a.retired_at IS NULL AND c.state IN ('ready','active') FROM initial_telephone_answers a JOIN operations o ON o.record_id=a.operation_record_id JOIN connections c ON c.id=o.connection_id WHERE c.id=?1",[&public],|r|r.get::<_,bool>(0)).unwrap());
    let before_replay: u64 = db
        .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    let (packet, address) = original_info.unwrap();
    peer.socket
        .send_to(packet.as_bytes(), address)
        .await
        .unwrap();
    send_rfc(peer).await;
    for p in clients.iter_mut() {
        assert!(
            tokio::time::timeout(Duration::from_millis(200), signal(&mut p.client))
                .await
                .is_err()
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        before_replay,
        "old private input escaped into restored public audience"
    );
    for (frame, status) in frames.iter().zip(&statuses) {
        assert_eq!(
            &settled(
                &mut source_status,
                frame["operation_id"].as_str().unwrap(),
                &cid
            )
            .await,
            status
        );
    }
    *peer.media_target.lock().unwrap() = public_media;
    *peer.dialog.lock().unwrap() = public_dialog;
    let recovered = marked(peer, clients).await;
    assert!(
        recovered.iter().all(|n| *n > 10),
        "original public sibling did not survive private return"
    );
    assert_eq!(peer.unique(), 2);
    send_info(peer).await;
    let mut restored_frames = vec![];
    let mut restored_status = None;
    for p in clients.iter_mut() {
        let frame = tokio::time::timeout(Duration::from_secs(5), signal(&mut p.client))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.scope(WireScope::Connection), Some(p.id.as_str()));
        let status = settled(
            &mut source_status,
            frame.scope(WireScope::Operation).unwrap(),
            &cid,
        )
        .await;
        assert_eq!(status["state"], "completed");
        let recipients = status["result"]["recipients"].as_array().unwrap();
        assert_eq!(recipients.len(), 2);
        assert!(recipients.iter().all(|r| r["connection_id"] != phone));
        assert!(recipients.iter().any(|r| r["connection_id"] == p.id));
        if let Some(previous) = &restored_status {
            assert_eq!(previous, &status);
        }
        restored_status = Some(status);
        restored_frames.push(frame.original().clone());
    }
    let source_count: u64 = db
        .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        source_count, 6,
        "replayed private input created another logical source"
    );
    observed["restored_frames"] = json!(restored_frames);
    observed["restored_status"] = restored_status.unwrap();
    observed["source_operation_count"] = json!(source_count);
    observed["source_kind"] = json!("private-initial-sip");
    observed["admission_expired"] = json!(true);
    observed["controller_revoked"] = json!(true);
    observed["selection"] = selected;
    observed["completed"] = serde_json::to_value(completed).unwrap();
    observed["private_frames"] = json!(frames);
    observed["private_statuses"] = json!(statuses);
    observed["authority"] = authority;
    observed["private_native_status"] = sent;
    observed["returned"] = serde_json::to_value(returned).unwrap();
    observed["private_counts"] = json!(private_counts);
    observed["public_sibling_private_counts"] = json!(sibling_counts);
    observed["private_native_to_sip_by_call"] = json!(duplex);
    observed["after_expired_admission_and_revoked_controller"] = json!(after_expiry);
    observed["recovered_public_counts"] = json!(recovered);
    observed["distinct_sip_calls"] = json!(peer.unique());
    if let Some(path) = std::env::var_os("CONNECT_PRIVATE_SIP_SWITCH_CAPTURE") {
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        std::fs::write(
            path.with_extension(format!("{}.json", observed["algorithm"].as_str().unwrap())),
            serde_json::to_vec_pretty(&observed).unwrap(),
        )
        .unwrap();
    }
}

async fn drain_audio(peer: &PeerSip) {
    let until = tokio::time::Instant::now() + Duration::from_millis(250);
    let mut bytes = [0; 4096];
    while let Ok(Ok(_)) = tokio::time::timeout_at(until, peer.media.recv_from(&mut bytes)).await {}
}
async fn audio_by_call(
    peer: &PeerSip,
    private: std::net::SocketAddr,
    public: std::net::SocketAddr,
) -> [usize; 2] {
    let until = tokio::time::Instant::now() + Duration::from_millis(2600);
    let mut bytes = [0; 4096];
    let mut counts = [0, 0];
    while let Ok(Ok((n, address))) =
        tokio::time::timeout_at(until, peer.media.recv_from(&mut bytes)).await
    {
        if n < 12 || bytes[0] >> 6 != 2 || (200..=211).contains(&bytes[1]) {
            continue;
        }
        assert_eq!(bytes[1] & 0x7f, 0);
        let mut header = 12 + usize::from(bytes[0] & 15) * 4;
        if bytes[0] & 0x10 != 0 {
            header +=
                4 + 4 * usize::from(u16::from_be_bytes([bytes[header + 2], bytes[header + 3]]));
        }
        let end = if bytes[0] & 0x20 != 0 {
            n - usize::from(bytes[n - 1])
        } else {
            n
        };
        assert!(header <= end);
        if bytes[header..end]
            .iter()
            .any(|b| !matches!(*b, 0xff | 0x7f))
        {
            let index = if address == private {
                0
            } else {
                assert_eq!(address, public, "unrecognized physical SIP source");
                1
            };
            counts[index] += 1;
        }
    }
    counts
}
