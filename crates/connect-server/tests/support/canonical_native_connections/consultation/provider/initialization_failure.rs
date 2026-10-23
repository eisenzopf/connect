//! Public decisions at real provider creation/export uncertainty boundaries.
use super::*;
#[allow(clippy::too_many_arguments)]
pub(super) async fn case(
    f: &Fixture,
    clients: &mut [CanonicalClient],
    cid: &str,
    sh: u16,
    th: u16,
    credential: &connect_store::IssuedCredential,
    command: &TransitionCommand,
    bridge: &mut Bridge,
    record: &str,
    provider: &Provider,
    bodies: &mut mpsc::UnboundedReceiver<Value>,
    remote_clock: tokio::time::Instant,
    primary_clock: tokio::time::Instant,
    mode: Case,
    record_uuid: Option<&str>,
) {
    let observed = tokio::time::timeout(Duration::from_secs(6), bodies.recv()).await;
    let create = match observed {
        Ok(Some(body)) => body,
        other => {
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let mut q=c.prepare("SELECT command_type,state,COALESCE(json_extract(problem_json,'$.code'),'') FROM operations WHERE command_type IN ('session.consult','connection.offer') ORDER BY created_at,operation_id").unwrap();
            let facts = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            panic!("provider create was not observed: {other:?}; bridge={:?}; runtime={:?}; operations={facts:?}",bridge.status(),f.server.service.canonical_runtime_snapshot());
        }
    };
    assert_eq!(create["assistantId"], "private-fixture-assistant");
    assert!(!create
        .to_string()
        .contains("authorized provider history marker"));
    let mut cfg = canonical_http_config(f);
    cfg.credential = credential.token.clone();
    let mut client = CanonicalHttpClient::open(cfg.clone()).await.unwrap();
    let cancel_scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some(command.session_id.clone()),
        connection_id: command.connection_id.clone(),
        operation_id: Some("cancel-private-provider-initialization".into()),
        ..Default::default()
    };
    let cancel_body = json!({"operation_id":command.operation_id,"reason":"cancel private provider initialization"});
    if matches!(
        mode,
        Case::AutomaticCancelExport
            | Case::AutomaticCancelRecordExport
            | Case::AutomaticCancelCreation
            | Case::AutomaticCancelCreationRecordOutage
    ) {
        if matches!(
            mode,
            Case::AutomaticCancelExport | Case::AutomaticCancelRecordExport
        ) {
            let observed = tokio::time::timeout(Duration::from_secs(6), bodies.recv()).await;
            let export = match observed {
                Ok(Some(body)) => body,
                other => {
                    let c = rusqlite::Connection::open(&f.config.database).unwrap();
                    let facts:(String,bool,bool,bool,bool,bool,String,Option<i64>,Option<i64>)=c.query_row("SELECT o.state,p.started,p.provider_ref IS NOT NULL,p.export_started,p.uncertain,p.export_uncertain,d.state,p.creation_expires_at,p.export_expires_at FROM private_provider_contexts p JOIN operations o ON o.record_id=p.operation_record_id JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).unwrap();
                    panic!("provider export was not observed: {other:?}; bridge={:?}; phases={facts:?}; now={}; runtime={:?}",bridge.status(),chrono::Utc::now().timestamp(),f.server.service.canonical_runtime_snapshot());
                }
            };
            assert_eq!(export["type"], "add-message");
            assert!(export
                .to_string()
                .contains("authorized provider history marker"));
            if let Some(uuid) = record_uuid {
                let content = export["message"]["content"].as_str().unwrap();
                let data = content.split_once("permission. ").unwrap().1;
                let view: connect_protocol::ContextView = serde_json::from_str(data).unwrap();
                let item = view.items.iter().find(|v| v.source_id == uuid).unwrap();
                let reference = item.record.as_ref().unwrap();
                assert_eq!(reference.uuid, uuid);
                assert_eq!(reference.session_id, "supervised-record-source");
                assert_eq!(reference.revision, 1);
                assert_eq!(
                    reference.state,
                    connect_protocol::ContextRecordState::Available
                );
                assert_eq!(
                    reference.content_hash.as_deref(),
                    Some(
                        format!(
                            "sha512-{}",
                            base64::engine::general_purpose::URL_SAFE_NO_PAD
                                .encode(sha2::Sha512::digest(item.body.as_bytes()))
                        )
                        .as_str()
                    )
                );
            }
        }
        // Cancellation records intent while an already submitted authorized
        // export can still acknowledge. It must forbid the next native effect,
        // rather than inventing cancellation of the earlier provider write.
        client
            .request_value(
                "operation.cancel",
                cancel_body.clone(),
                cancel_scope.clone(),
            )
            .await
            .unwrap()
            .ack()
            .unwrap();
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let pending:(String,bool,bool,bool)=c.query_row("SELECT o.state,o.cancel_requested,p.delivered_at IS NOT NULL,p.closed_at IS NOT NULL FROM operations o JOIN private_provider_contexts p ON p.operation_record_id=o.record_id WHERE o.record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(pending, ("unknown".into(), true, false, false));
        drop(c);
        let deleting = if mode == Case::AutomaticCancelRecordExport {
            // Mutation is independently authorized by the installation operator.
            // The pending export still retains its original record custody.
            let mut admin = canonical_http_config(f);
            admin.credential = f.admin.clone();
            let mut api = connect_client::application::ApplicationClient::open(admin)
                .await
                .unwrap();
            let request = connect_protocol::record_mutations::Request {
                operation_id: "erase-record-during-private-export".into(),
                conversation_id: cid.into(),
                session_id: "supervised-record-source".into(),
                uuid: record_uuid.unwrap().into(),
                expected_revision: 1,
                action: connect_protocol::record_mutations::Action::Delete,
            };
            let deleting = tokio::spawn(async move {
                let receipt = api.mutate_record(request.clone()).await.unwrap();
                assert_eq!(receipt.revision, 2);
                assert_eq!(receipt.uuid, request.uuid);
                let replay = api.mutate_record(request).await.unwrap();
                assert!(
                    receipt == replay,
                    "record mutation replay changed its original receipt"
                );
            });
            // The writer acquires member identities before the Conversation.
            // Probe all current members so ordering cannot hide its reservation
            // on an earlier identity behind the retained export reader.
            let readers = {
                let c = rusqlite::Connection::open(&f.config.database).unwrap();
                let mut q = c.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY identity_id").unwrap();
                q.query_map([cid], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(|id| {
                        connect_store::AuthorityScope::identity(
                            &id.unwrap(),
                            connect_store::AuthorityMode::Read,
                        )
                        .unwrap()
                    })
                    .collect::<Vec<_>>()
            };
            let until = tokio::time::Instant::now() + Duration::from_secs(1);
            loop {
                match f.server.store.try_acquire_authority(&readers) {
                    Err(e) if e.code == "authority-busy" => break,
                    Ok(held) => drop(held),
                    Err(e) => panic!("erasure admission probe: {}", e.code),
                }
                assert!(
                    tokio::time::Instant::now() < until,
                    "record erasure writer never queued"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(
                !deleting.is_finished(),
                "erasure completed before exact export acknowledgement"
            );
            Some(deleting)
        } else {
            None
        };
        if mode == Case::AutomaticCancelCreationRecordOutage {
            // The actual HTTP create has returned while recording its captured
            // reference is refused. Other current call IO remains usable.
            clocked_audio_reaches(clients, sh, th, 121100, remote_clock).await;
            clients.swap(0, 1);
            clocked_audio_reaches(clients, th, sh, 122100, primary_clock).await;
            clients.swap(0, 1);
            assert_eq!(provider.creations.load(Ordering::SeqCst), 1);
            assert_eq!(provider.exports.load(Ordering::SeqCst), 0);
            let c = rusqlite::Connection::open(&f.config.database).unwrap();
            let captured:bool=c.query_row("SELECT provider_ref IS NOT NULL FROM private_provider_contexts WHERE operation_record_id=?1",[record],|r|r.get(0)).unwrap();
            assert!(!captured);
            c.execute_batch("DROP TRIGGER hold_private_created_receipt")
                .unwrap();
        } else {
            provider.release_ack.notify_one();
        }
        if let Some(deleting) = deleting {
            tokio::time::timeout(Duration::from_secs(6), deleting)
                .await
                .unwrap()
                .unwrap();
        }
        client
            .request_value(
                "operation.cancel",
                cancel_body.clone(),
                cancel_scope.clone(),
            )
            .await
            .unwrap()
            .ack()
            .unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(12), bridge.context_ready())
            .await
            .unwrap()
            .is_err()
    );
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let facts = loop {
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let facts:(String,bool,bool,bool,bool,bool,u64)=c.query_row("SELECT o.state,o.cancel_requested,p.started,p.provider_ref IS NOT NULL,p.delivered_at IS NOT NULL,p.closed_at IS NOT NULL,(SELECT COUNT(*) FROM native_consultation_attempts n WHERE n.operation_record_id=o.record_id) FROM operations o JOIN private_provider_contexts p ON p.operation_record_id=o.record_id WHERE o.record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
        if matches!(bridge.status(), PrivateVapiStatus::Stopped { .. })
            && (mode == Case::AutomaticLostCreationResponse || facts.0 == "cancelled")
        {
            break facts;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "initialization did not settle: {facts:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if matches!(
        mode,
        Case::AutomaticCancelExport
            | Case::AutomaticCancelRecordExport
            | Case::AutomaticCancelCreation
            | Case::AutomaticCancelCreationRecordOutage
    ) {
        let exported = matches!(
            mode,
            Case::AutomaticCancelExport | Case::AutomaticCancelRecordExport
        );
        assert_eq!(
            facts,
            ("cancelled".into(), true, true, true, exported, true, 0)
        );
        assert_eq!(
            provider.exports.load(Ordering::SeqCst),
            usize::from(exported)
        );
        assert_eq!(provider.ended.load(Ordering::SeqCst), 1);
    } else {
        assert_eq!(
            facts,
            ("unknown".into(), false, true, false, false, false, 0)
        );
        assert_eq!(provider.exports.load(Ordering::SeqCst), 0);
        assert_eq!(provider.ended.load(Ordering::SeqCst), 0);
        client
            .request_value("operation.cancel", cancel_body, cancel_scope)
            .await
            .unwrap()
            .ack()
            .unwrap();
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let state: String = c
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [record],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            state, "unknown",
            "a lost creation response cannot prove cancellation"
        );
    }
    for _ in 0..2 {
        let replay = client.transition(command.clone()).await.unwrap();
        assert_eq!(replay.operation_id.as_str(), command.operation_id);
        assert_eq!(
            serde_json::to_value(replay.state).unwrap(),
            json!(if mode != Case::AutomaticLostCreationResponse {
                "cancelled"
            } else {
                "unknown"
            })
        );
    }
    assert_eq!(provider.creations.load(Ordering::SeqCst), 1);
    assert_eq!(provider.private_audio.load(Ordering::SeqCst), 0);
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let count: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM private_provider_contexts WHERE operation_record_id=?1",
            [record],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(c);
    clocked_audio_reaches(clients, sh, th, 123100, remote_clock).await;
    clients.swap(0, 1);
    clocked_audio_reaches(clients, th, sh, 124100, primary_clock).await;
    clients.swap(0, 1);
}
