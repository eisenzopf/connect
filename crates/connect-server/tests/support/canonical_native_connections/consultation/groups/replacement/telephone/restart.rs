//! Actual configured restart retains facts, fences old media and never redials.
use super::*;
use rusqlite::OptionalExtension;

struct Captures {
    attempts: Vec<(String, String, String)>,
    known: Vec<(String, String)>,
}
fn captures(c: &rusqlite::Connection, record: &str) -> Captures {
    let mut attempts = c.prepare("SELECT phase,attempt_id,effect_id FROM native_replacement_steps WHERE operation_record_id=?1 UNION ALL SELECT phase,attempt_id,effect_id FROM telephone_replacement_phases WHERE operation_record_id=?1 ORDER BY 1,2").unwrap();
    let attempts = attempts
        .query_map([record], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let mut known = c.prepare("SELECT 'step:'||phase,observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND observation_json IS NOT NULL UNION ALL SELECT 'phone:'||phase,observation_json FROM telephone_replacement_phases WHERE operation_record_id=?1 AND observation_json IS NOT NULL UNION ALL SELECT 'resources',observation_json FROM telephone_replacement_resources WHERE operation_record_id=?1 AND observation_json IS NOT NULL UNION ALL SELECT 'commit',observation_json||audience_json FROM telephone_replacement_commits WHERE operation_record_id=?1 AND observation_json IS NOT NULL ORDER BY 1").unwrap();
    let known = known
        .query_map([record], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    Captures { attempts, known }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_restart_private_duplex_keeps_known_captures_unknown_root_and_refuses_late_old_events(
) {
    telephone_case(Stop::PrivateRestart, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_restart_completed_move_preserves_receipt_without_live_claim_or_redial(
) {
    telephone_case(Stop::CommittedRestart, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_restart_private_owner_drop_preserves_captures_and_refuses_old_io() {
    telephone_case(Stop::PrivateDrop, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_restart_completed_owner_drop_fences_ready_claim_under_successor_epoch(
) {
    telephone_case(Stop::CommittedDrop, true).await;
}

pub(super) async fn without_redial(
    f: Fixture,
    command: TransitionCommand,
    token: &str,
    peer: &PeerSip,
    bind: std::net::SocketAddr,
    before: OperationRecord,
    payload: Option<u8>,
    abrupt: bool,
) {
    let old_epoch = f.server.store.runtime_epoch().to_owned();
    let mut config = canonical_http_config(&f);
    config.credential = token.into();
    let database = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let frozen = captures(&database, &before.record_id);
    let candidate: String = database
        .query_row(
            "SELECT candidate_connection_id FROM native_replacements WHERE operation_record_id=?1",
            [&before.record_id],
            |r| r.get(0),
        )
        .unwrap();
    let completed = before.state == OperationState::Completed;
    assert_eq!(peer.unique(), 1);
    if abrupt {
        drop(f.server);
    } else {
        f.server.shutdown().await.unwrap();
    }
    until(|| {
        let methods = peer.methods.lock().unwrap();
        methods.contains(&if payload.is_some() {
            Method::Bye
        } else {
            Method::Cancel
        }) && methods.contains(&Method::Ack)
    })
    .await;
    until(|| {
        let udp = std::net::UdpSocket::bind(bind);
        let tcp = std::net::TcpListener::bind(bind);
        udp.is_ok() && tcp.is_ok()
    })
    .await;
    assert_sip_listener_released(bind);
    let restarted = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    assert_ne!(old_epoch, restarted.store.runtime_epoch());
    config.endpoint = restarted.http_address.unwrap();
    let mut client = CanonicalHttpClient::open(config).await.unwrap();
    let scope = CommandScope {
        conversation_id: Some(command.conversation_id.clone()),
        session_id: Some(command.session_id.clone()),
        connection_id: command.connection_id.clone(),
        ..Default::default()
    };
    let current = client
        .operation_get(&command.operation_id, scope.clone())
        .await
        .unwrap();
    let expected = if completed {
        rvoip_uctp::v2::payloads::OperationState::Completed
    } else {
        rvoip_uctp::v2::payloads::OperationState::Unknown
    };
    assert_eq!(current.state, expected);
    assert_eq!(
        current.record_id.as_ref().unwrap().as_str(),
        before.record_id
    );
    if completed {
        assert_eq!(current.result, before.result);
    } else {
        assert!(current.result.is_none());
    }
    let row: Option<(String,u64,Option<String>,Option<String>)> = database.query_row("SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id=?1",[&candidate],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().unwrap();
    if completed {
        let (state, generation, observed, readiness) = row.as_ref().unwrap();
        if abrupt {
            assert_eq!(state, "interrupted");
        } else {
            assert!(matches!(state.as_str(), "retired" | "interrupted"));
        }
        assert_eq!(*generation, 3);
        assert!(observed.is_none() && readiness.is_none());
        let epoch: String = database
            .query_row(
                "SELECT runtime_epoch FROM telephone_connection_lifetimes WHERE connection_id=?1",
                [&candidate],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(epoch, old_epoch);
        if abrupt {
            let retired: bool = database
                .query_row(
                    "SELECT retired FROM telephone_connection_lifetimes WHERE connection_id=?1",
                    [&candidate],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                !retired,
                "retired owner wrote a false completed cleanup capture"
            );
        }
    } else {
        assert!(
            row.is_none(),
            "private preparation invented a public ready connection"
        );
    }
    let raw = peer.calls.lock().unwrap().len();
    let replay = client.transition(command.clone()).await.unwrap();
    assert_eq!(replay.state, expected);
    assert_eq!(replay.result, current.result);
    // Deliver genuine late old-dialog INFO and old RTP to the restarted host.
    // They cannot become new attributed confirmation or revive its old route.
    if let Some(payload) = payload {
        let _ = sip_audio(peer, payload, Duration::from_millis(100)).await;
        for digit in ["5", "5", "#"] {
            peer.digit(digit).await;
        }
        let (_, marked) = tokio::join!(
            peer.rtp(payload),
            sip_audio(peer, payload, Duration::from_millis(900))
        );
        assert_eq!(marked, 0, "old telephone media resumed after restart");
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let again = client
        .operation_get(&command.operation_id, scope)
        .await
        .unwrap();
    assert_eq!(again.state, expected);
    assert_eq!(again.revision, current.revision);
    assert_eq!(again.result, current.result);
    assert_eq!(
        peer.calls.lock().unwrap().len(),
        raw,
        "restart/replay redialed a retained callback"
    );
    let after = captures(&database, &before.record_id);
    assert!(
        frozen.attempts == after.attempts,
        "restart changed once-only attempts"
    );
    assert!(
        frozen.known == after.known,
        "restart changed actual known captures"
    );
    let after_row: Option<(String,u64,Option<String>,Option<String>)> = database.query_row("SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id=?1",[&candidate],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().unwrap();
    assert_eq!(row, after_row);
    let runtime = restarted.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(runtime.connections, 0);
    assert_eq!(runtime.replacement_preparations, 0);
    assert_eq!(runtime.media_readers, 0);
    restarted.shutdown().await.unwrap();
    assert_sip_listener_released(bind);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_lost_command_reply_recovers_same_attempt_and_never_redials_unknown_submission(
) {
    telephone_case(Stop::LostReply, true).await;
}

pub(super) async fn submit_without_read(
    f: &Fixture,
    token: &str,
    command: &TransitionCommand,
) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;
    let value = json!({"v":2,"type":"connection.replace","id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),
        "cid":command.conversation_id,"sid":command.session_id,"connid":command.connection_id,"operation_id":command.operation_id,"expected_revision":command.expected_revision,
        "payload":serde_json::to_value(&command.intent).unwrap().as_object().unwrap().iter().filter(|(k,v)|k.as_str()!="type" && !v.is_null()).map(|(k,v)|(k.clone(),v.clone())).collect::<serde_json::Map<_,_>>()});
    Envelope::from_value(value.clone()).unwrap();
    let body = serde_json::to_vec(&value).unwrap();
    let address = f.server.http_address.unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let header=format!("POST /v2/commands HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
    socket.write_all(header.as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    socket
}
