//! Actual received signal authority, using fenced credential edits as fault setup.
use super::*;
use rvoip_uctp::v2::{
    payloads::{OperationState, OperationUpdated},
    Envelope, Scope as WireScope,
};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_initial_sip_received_signal_only_and_current_direction_permissions_preserve_call_without_media(
) {
    for algorithm in ["EdDSA", "ES256"] {
        initial_selected_signal_case(
            false,
            algorithm,
            EndpointTransport::Telephone,
            "participant",
            false,
            Some("received-authority"),
        )
        .await;
    }
}
async fn permissions(f: &Fixture, identity: &str, key: &str, operations: Vec<String>) {
    let store = f.server.store.clone();
    let identity = identity.to_owned();
    let key = key.to_owned();
    let path = f.config.database.clone();
    tokio::task::spawn_blocking(move || {
        let _writer = store
            .acquire_authority(
                &[connect_store::AuthorityScope::identity(
                    &identity,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        assert_eq!(
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    rusqlite::params![key, serde_json::to_string(&operations).unwrap()]
                )
                .unwrap(),
            1
        );
    })
    .await
    .unwrap();
}
async fn signal(client: &mut CanonicalClient, phase: &str) -> Envelope {
    for _ in 0..128 {
        let frame = client
            .receive()
            .await
            .unwrap_or_else(|error| panic!("received authority phase {phase}: {error:?}"));
        if frame.kind() == "dtmf.received" {
            return frame;
        }
    }
    panic!("missing signal behind lifecycle hints")
}
async fn settled(
    http: &mut CanonicalHttpClient,
    operation: &str,
    scope: &CommandScope,
) -> OperationUpdated {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let r = http.operation_get(operation, scope.clone()).await.unwrap();
        if matches!(
            r.state,
            OperationState::Completed
                | OperationState::Failed
                | OperationState::Unknown
                | OperationState::Cancelled
        ) {
            return r;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn count(db: &rusqlite::Connection) -> u64 {
    db.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn frame_scope(frame: &Envelope, native_id: &str) {
    assert_eq!(frame.scope(WireScope::Connection), Some(native_id));
    assert_eq!(frame.original()["payload"]["digits"], "5");
    assert_eq!(frame.original()["payload"]["duration_ms"], 237);
}
pub(in super::super) async fn exercise(
    f: &Fixture,
    peer: &PeerSip,
    native: &mut [Peer],
    cid: &str,
    phone_key: &str,
    algorithm: &str,
) {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let owner = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(f);
    config.credential = owner.token;
    let mut http = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        ..Default::default()
    };
    let native_id = native[0].id.clone();
    let native_key = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let original_native: Vec<String> = serde_json::from_str(
        &db.query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&native_key],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!original_native.iter().any(|v| v == "*"));
    // Prove the current writer before changing rights.
    send_info(peer).await;
    let first = tokio::time::timeout(
        Duration::from_secs(5),
        signal(&mut native[0].client, "warmup"),
    )
    .await
    .unwrap();
    frame_scope(&first, &native_id);
    assert_eq!(
        settled(
            &mut http,
            first.scope(WireScope::Operation).unwrap(),
            &scope
        )
        .await
        .state,
        OperationState::Completed
    );
    permissions(
        f,
        &f.peer.identity_id,
        phone_key,
        vec!["dtmf.send".into(), "dtmf.receive".into()],
    )
    .await;
    // Cross the real monitor's former media-only cleanup path and drain packets
    // admitted before the writer commit. Datagram waits do not own control reads.
    tokio::time::sleep(Duration::from_millis(350)).await;
    let end = tokio::time::Instant::now() + Duration::from_millis(200);
    let mut bytes = [0; 4096];
    while let Ok(Ok(_)) = tokio::time::timeout_at(end, peer.media.recv_from(&mut bytes)).await {}
    let drain = tokio::time::Instant::now() + Duration::from_millis(200);
    while let Ok(Ok(_)) =
        tokio::time::timeout_at(drain, native[0].client.receive_native_media_datagram()).await
    {}
    let receive = async {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(900);
        let mut marked = 0;
        while let Ok(Ok(bytes)) =
            tokio::time::timeout_at(deadline, native[0].client.receive_native_media_datagram())
                .await
        {
            let packet = rvoip_uctp::substrate::unpack_rtp_datagram(&bytes).unwrap();
            if packet
                .rtp
                .payload
                .iter()
                .any(|b| !matches!(*b, 0xff | 0x7f))
            {
                marked += 1;
            }
        }
        marked
    };
    let (_, from_phone) = tokio::join!(peer.rtp(0), receive);
    assert_eq!(from_phone, 0, "signal permission leaked marked SIP audio");
    let (_, to_phone) = tokio::join!(
        probe(native, 0, &[], "received-signal-only-no-media"),
        sip_audio(peer, 0, Duration::from_millis(2600))
    );
    assert_eq!(to_phone, 0, "signal permission leaked marked native audio");
    let mut samples = Vec::new();
    for kind in ["sip-info", "rfc4733"] {
        if kind == "sip-info" {
            send_info(peer).await;
        } else {
            send_rfc(peer).await;
        }
        let frame =
            tokio::time::timeout(Duration::from_secs(5), signal(&mut native[0].client, kind))
                .await
                .unwrap();
        frame_scope(&frame, &native_id);
        assert_eq!(frame.original()["payload"]["source"], kind);
        let status = settled(
            &mut http,
            frame.scope(WireScope::Operation).unwrap(),
            &scope,
        )
        .await;
        assert_eq!(status.state, OperationState::Completed);
        samples.push(
            json!({"phase":"source-signal-only","operation":status,"frame":frame.original()}),
        );
    }
    assert_eq!(count(&db), 3);
    permissions(
        f,
        &f.client.identity_id,
        &native_key,
        vec![
            "connect:control".into(),
            // Retain this already authenticated peer's protocol floors using
            // metadata permissions, while removing offer/media subscriptions.
            "conversation.get".into(),
            "conversation.subscribe".into(),
            "dtmf.receive".into(),
            "operation.get".into(),
        ],
    )
    .await;
    send_info(peer).await;
    let frame = tokio::time::timeout(
        Duration::from_secs(5),
        signal(&mut native[0].client, "both-signal-only"),
    )
    .await
    .unwrap();
    frame_scope(&frame, &native_id);
    let status = settled(
        &mut http,
        frame.scope(WireScope::Operation).unwrap(),
        &scope,
    )
    .await;
    assert_eq!(status.state, OperationState::Completed);
    samples.push(json!({"phase":"both-signal-only","operation":status,"frame":frame.original()}));
    assert_eq!(count(&db), 4);
    permissions(
        f,
        &f.peer.identity_id,
        phone_key,
        vec!["dtmf.receive".into()],
    )
    .await;
    // Keep this exact control read alive through refusal and recovery.
    let mut next = Box::pin(signal(&mut native[0].client, "source-recovery"));
    peer.digit("9").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), next.as_mut())
            .await
            .is_err()
    );
    assert_eq!(
        count(&db),
        4,
        "source receive permission borrowed source send authority"
    );
    permissions(
        f,
        &f.peer.identity_id,
        phone_key,
        vec!["dtmf.send".into(), "dtmf.receive".into()],
    )
    .await;
    send_info(peer).await;
    let recovered = tokio::time::timeout(Duration::from_secs(5), next.as_mut())
        .await
        .unwrap();
    drop(next);
    frame_scope(&recovered, &native_id);
    assert_eq!(
        settled(
            &mut http,
            recovered.scope(WireScope::Operation).unwrap(),
            &scope
        )
        .await
        .state,
        OperationState::Completed
    );
    assert_eq!(
        count(&db),
        5,
        "refused source input was replayed after rights returned"
    );
    let without_receive = original_native
        .iter()
        .filter(|v| v.as_str() != "dtmf.receive")
        .cloned()
        .collect();
    permissions(f, &f.client.identity_id, &native_key, without_receive).await;
    let mut next = Box::pin(signal(&mut native[0].client, "recipient-recovery"));
    peer.digit("8").await;
    until(|| {
        db.query_row(
            "SELECT count(*) FROM received_dtmf_sources WHERE closed=1",
            [],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
            == 6
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), next.as_mut())
            .await
            .is_err()
    );
    let denied_id:String=db.query_row("SELECT o.operation_id FROM operations o JOIN received_dtmf_sources s ON s.operation_record_id=o.record_id WHERE json_extract(s.observation_json,'$.digits')='8'",[],|r|r.get(0)).unwrap();
    let denied = settled(&mut http, &denied_id, &scope).await;
    assert_eq!(denied.state, OperationState::Failed);
    assert_eq!(
        denied.problem.as_ref().unwrap()["signal"]["recipients"],
        json!([])
    );
    permissions(f, &f.client.identity_id, &native_key, original_native).await;
    send_info(peer).await;
    let recovered = tokio::time::timeout(Duration::from_secs(5), next.as_mut())
        .await
        .unwrap();
    drop(next);
    frame_scope(&recovered, &native_id);
    let final_status = settled(
        &mut http,
        recovered.scope(WireScope::Operation).unwrap(),
        &scope,
    )
    .await;
    assert_eq!(final_status.state, OperationState::Completed);
    assert_eq!(count(&db), 7);
    assert_eq!(peer.unique(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM initial_telephone_answers WHERE connected_at IS NOT NULL AND retired_at IS NULL",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    assert!(!peer.methods.lock().unwrap().contains(&Method::Bye));
    samples.push(json!({"phase":"recipient-denied","operation":denied}));
    samples.push(json!({"phase":"recipient-recovered","operation":final_status,"frame":recovered.original()}));
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_AUTHORITY_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes=serde_json::to_vec(&json!({"algorithm":algorithm,"source_missing_send_accepted":0,"marked_from_phone":from_phone,"marked_to_phone":to_phone,"calls":peer.unique(),"operations":count(&db),"samples":samples})).unwrap();
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
