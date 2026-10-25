//! A real answer and callback stay unconfirmed until the child is killed.
use super::*;
const CONNECTION: &str = "crash-answer-connection";
const OPERATION: &str = "crash-answer-offer";

fn scope(cid: &str) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(CONNECTION.into()),
        operation_id: Some(OPERATION.into()),
        expected_revision: Some(0),
    }
}

pub(super) async fn child(root: &Path, algorithm: &str) {
    let mut f = Fixture::with_native_dtmf_scope(algorithm, 12).await;
    assert!(f.config.database.starts_with(root));
    f.server.store.configure_session_reconnect_grace(1).unwrap();
    f.config.session_reconnect_grace_seconds = 1;
    ready_endpoint(&f).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session(&f, &mut client).await;
    let offer = json!({"substrate":"quic","capabilities":{
        "audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],
        "max_streams_per_connection":8,"identity_assurance_offered":"identified"},
        "streams_offered":[{"id":"crash-answer-voice","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let request = client
        .send_value("connection.offer", offer.clone(), scope(&cid))
        .await
        .unwrap();
    let answer = client.receive_control().await.unwrap();
    assert_eq!(answer.kind(), "connection.answer");
    assert_eq!(answer.original()["in_reply_to"], request);
    let callback = client.receive_control().await.unwrap();
    assert_eq!(callback.kind(), "connection.offer");
    // Deliberately do not answer the real callback. No confirmation, RTP or
    // protected Ready observation may be manufactured from this answer.
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let before:(String,bool,String,u64,Option<String>,Option<String>)=db.query_row(
        "SELECT o.state,n.effect_started,c.state,c.generation,c.observed_runtime_id,c.readiness_json FROM operations o JOIN native_offer_operations n ON n.operation_record_id=o.record_id JOIN connections c ON c.id=o.connection_id WHERE o.operation_id=?1",
        [OPERATION],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        before,
        ("preparing".into(), true, "preparing".into(), 1, None, None)
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_connection_lifetimes",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    checkpoint(
        &f,
        root,
        &cid,
        Phase::Answered,
        vec![CONNECTION.into()],
        Some(offer),
    );
    std::future::pending::<()>().await;
    drop((client, f));
}

pub(super) async fn verify(
    server: &RunningCanonicalServer,
    client: &mut CanonicalClient,
    ready: &Ready,
    db: &rusqlite::Connection,
) -> Value {
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let mut http =
        CanonicalHttpClient::open(connect_client::application::ApplicationClientConfig {
            endpoint: server.http_address.unwrap(),
            credential: ready.client.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            timeout_ms: 5000,
        })
        .await
        .unwrap();
    let old = http
        .operation_get(
            OPERATION,
            CommandScope {
                conversation_id: Some(ready.conversation.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(old.state, rvoip_uctp::v2::payloads::OperationState::Failed);
    let actor = server.store.authenticate(&ready.client.credential).unwrap();
    let ledger = || {
        server
            .store
            .execute_operation_action(
                &actor.credential_id,
                connect_protocol::transitions::OperationAction::Get {
                    operation_id: OPERATION.into(),
                },
            )
            .unwrap()
    };
    let original = ledger();
    assert_eq!(original.effects.len(), 1);
    assert_eq!(original.effects[0].attempt, 1);
    assert_eq!(
        original.effects[0].state,
        connect_protocol::transitions::EffectState::Failed
    );
    assert_eq!(old.record_id.as_ref().unwrap().as_str(), original.record_id);
    let replay = client
        .request_value(
            "connection.offer",
            ready.offer.clone().unwrap(),
            scope(&ready.conversation),
        )
        .await;
    let replay_disposition = match replay {
        Ok(batch) => {
            batch.ack().unwrap();
            assert!(batch.frames.iter().all(|f| !matches!(
                f.kind(),
                "connection.answer" | "connection.offer" | "connection.ready" | "stream.opened"
            )));
            "historical-outcome-only"
        }
        Err(ClientError::Application(_)) => "current-authority-refused",
        Err(error) => panic!("unexpected native replay failure: {error}"),
    };
    let after = http
        .operation_get(
            OPERATION,
            CommandScope {
                conversation_id: Some(ready.conversation.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    assert_eq!(
        serde_json::to_value(ledger()).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM connections", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_offer_runtime_admissions WHERE runtime_epoch=?1",
            [server.store.runtime_epoch()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM session_started_facts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    json!({"answered_before_confirmation":true,"ready_or_started_before_or_after":false,
        "original_operation_state":old.state,"original_effect_state":original.effects[0].state,
        "original_effect_attempt":original.effects[0].attempt,"replay_disposition":replay_disposition,
        "replay_preserved_original_record":true,"new_runtime_admissions":0})
}
