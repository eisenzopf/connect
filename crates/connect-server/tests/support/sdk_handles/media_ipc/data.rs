//! Exact reliable data bytes stay on binary IPC; recovery stays control-only.
use super::super::selections::{revision, stream};
use super::*;
use sdk::{DataPayload, Direction, Medium, StreamSelector};

fn payload(format: &str) -> (Vec<u8>, DataPayload) {
    let size = 64 * 1024;
    match format {
        "text" => {
            let text = format!("🦀{}", "x".repeat(size - 4));
            (text.as_bytes().to_vec(), DataPayload::Text(text))
        }
        "json" => {
            let value = json!({"data":"x".repeat(size - 11)});
            (
                serde_json::to_vec(&value).unwrap(),
                DataPayload::Json(value),
            )
        }
        _ => {
            let bytes = (0..size).map(|v| v as u8).collect::<Vec<_>>();
            (bytes.clone(), DataPayload::Binary(bytes))
        }
    }
}
fn valid_live_revocation(refused: &Value) -> bool {
    matches!(
        (refused["kind"].as_str(), refused["certainty"].as_str()),
        (Some("authentication"), Some("refused" | "uncertain"))
            | (Some("transport-uncertain"), Some("uncertain"))
    )
}
#[tokio::test]
async fn managed_typed_data_preserves_full_payload_original_ids_recovery_and_authority() {
    for algorithm in ["EdDSA", "ES256"] {
        for format in ["text", "json", "binary"] {
            let mut f = Fixture::with_native_end_scope(algorithm, 16).await;
            // The server's default configured message limit is16KiB. Exercise
            // the supported64KiB contract with an explicit isolated host limit.
            f.server.shutdown().await.unwrap();
            f.config.limits.message_bytes = 64 * 1024;
            f.server = RunningCanonicalServer::start(f.config.clone())
                .await
                .unwrap();
            f.client.endpoint = f.server.address;
            super::super::super::canonical_native_connections::ready_endpoint_with_capabilities(
                &f,
                vec!["data".into()],
            )
            .await;
            let mut scope = Scope::collaboration(vec!["*".into()]);
            scope
                .operations
                .extend(["stream.subscribe".into(), "connection.end".into()]);
            let target_config = super::super::super::canonical_native_connections::party_with_scope(
                &f,
                &f.peer.identity_id,
                211,
                vec!["data".into()],
                scope,
            );
            let source = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let target = Client::connect(target_config, "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let room = source
                .create_conversation(
                    vec![f.peer.identity_id.parse().unwrap()],
                    ConversationPolicy::Persistent,
                    mutation("ipc-data-create"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            let invited = room
                .refresh(options())
                .await
                .unwrap()
                .participants
                .into_iter()
                .find(|p| p.identity_id.as_str() == f.peer.identity_id)
                .unwrap()
                .id;
            let session = room
                .invite(
                    "ipc-data-session".parse().unwrap(),
                    vec![invited],
                    sdk::SessionMedium::Mixed,
                    "synthetic IPC data".into(),
                    json!({"data":["reliable-ordered"]}),
                    mutation("ipc-data-invite"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            let target_session = target
                .conversation_by_id(room.id().clone())
                .session_by_id(session.id().clone());
            target_session
                .accept(
                    json!({"data":["reliable-ordered"]}),
                    sdk::TopologyRevision::try_from(0).unwrap(),
                    mutation("ipc-data-accept"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap();
            let sending = session
                .offer_connection(
                    "ipc-data-source".parse().unwrap(),
                    NonEmpty {
                        first: stream("application", Medium::Data, Direction::Sendonly, format),
                        rest: vec![],
                    },
                    mutation("ipc-data-source-offer"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            let receiving = target_session
                .offer_connection(
                    "ipc-data-target".parse().unwrap(),
                    NonEmpty {
                        first: stream("application", Medium::Data, Direction::Recvonly, format),
                        rest: vec![],
                    },
                    mutation("ipc-data-target-offer"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value;
            receiving
                .subscribe(
                    NonEmpty {
                        first: StreamSelector::Stream("application".parse().unwrap()),
                        rest: vec![],
                    },
                    revision(1),
                    mutation("ipc-data-subscribe"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap();
            let from = sending.refresh(options()).await.unwrap().participant_id;
            let recipient = receiving.refresh(options()).await.unwrap().participant_id;
            let source_owner = helper::ManagedClient::new(source.clone());
            let target_owner = helper::ManagedClient::new(target.clone());
            let token = "c".repeat(64);
            let (attached, mut sender) = attach(&source_owner, &token, &token).await;
            attached.unwrap();
            let (attached, mut receiver) = attach(&target_owner, &token, &token).await;
            attached.unwrap();
            let binding = json!({"conversation_id":room.id(),"session_id":session.id(),"connection_id":sending.id()});
            let receive_binding = json!({"conversation_id":room.id(),"session_id":session.id(),"connection_id":receiving.id()});
            let command = json!({"action":"send_data","binding":binding,"stream_id":"application",
                "message_id":"ipc-data-message","to":[recipient],"format":format,"mutation":{"operation_id":"ipc-data-send"}});
            let (bytes, expected) = payload(format);
            assert_eq!(bytes.len(), 64 * 1024);
            let (kind, submitted) = sender.request(command.clone(), bytes.clone()).await;
            assert_eq!(kind, wire::Kind::Response, "{submitted}");
            assert_eq!(submitted["type"], "data_submitted");
            assert_eq!(submitted["binding"], binding);
            assert_eq!(submitted["stream_id"], "application");
            let saved = submitted["reference"].clone();
            assert_eq!(saved["operation_id"], "ipc-data-send");
            assert!(saved.get("payload").is_none());
            let delivered = receiving
                .stream_by_id("application".parse().unwrap())
                .receive_data(options())
                .await
                .unwrap();
            assert_eq!(delivered.payload, expected);
            assert_eq!(delivered.from, from);
            assert_eq!(delivered.message_id.as_str(), "ipc-data-message");
            assert_eq!(delivered.operation_id.as_str(), "ipc-data-send");
            // A fresh control owner can recover this original admission without
            // acquiring the physical Stream. Payload never enters control output.
            let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
                .await
                .unwrap();
            let observer = helper::ManagedClient::new(fresh.clone());
            let result = observer
                .execute(request(
                    json!({"type":"operation","reference":saved,"kind":"data_send",
                "action":{"type":"wait"}}),
                ))
                .await
                .unwrap();
            assert_eq!(
                result["value"],
                json!({"messageId":"ipc-data-message","connectionId":sending.id(),"boundary":"core-queue-admission"})
            );
            let record = result["snapshot"]["recordId"].clone();
            let (kind, replayed) = sender.request(command.clone(), bytes.clone()).await;
            assert_eq!(kind, wire::Kind::Response);
            assert_eq!(replayed["reference"], saved);
            assert!(
                matches!(receiving.stream_by_id("application".parse().unwrap()).receive_data(
                RequestOptions::new(Duration::from_millis(100)).unwrap()).await,Err(e) if e.kind==sdk::ErrorKind::Deadline)
            );
            let mut changed = command.clone();
            changed["message_id"] = json!("ipc-data-changed");
            let (kind, refused) = sender.request(changed, bytes.clone()).await;
            assert_eq!(kind, wire::Kind::Failure);
            assert_eq!(refused["certainty"], "refused");
            assert_eq!(refused["operation"]["operationId"], "ipc-data-send");
            let mut invalid = command.clone();
            invalid["to"] = json!([]);
            invalid["mutation"]["operation_id"] = json!("ipc-data-invalid");
            let (kind, refused) = sender.request(invalid, bytes.clone()).await;
            assert_eq!(kind, wire::Kind::Failure);
            assert_eq!(refused["certainty"], "not-submitted");
            assert_eq!(refused["operation"]["operationId"], "ipc-data-invalid");
            // Public Rust publication independently exercises the binary receive path.
            sending
                .stream_by_id("application".parse().unwrap())
                .send_data(
                    "ipc-data-return-message".parse().unwrap(),
                    NonEmpty {
                        first: recipient.clone(),
                        rest: vec![],
                    },
                    expected,
                    mutation("ipc-data-return"),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap();
            let id=receiver.send(json!({"action":"receive_data","binding":receive_binding,"stream_id":"application"}),vec![]).await;
            let frame = receiver.next().await;
            assert_eq!(frame.request_id, id);
            assert_eq!(frame.kind, wire::Kind::Packet);
            assert_eq!(frame.payload, bytes);
            let metadata: Value = serde_json::from_slice(&frame.metadata).unwrap();
            assert_eq!(
                metadata,
                json!({"type":"data","binding":receive_binding,"stream_id":"application",
                "message_id":"ipc-data-return-message","operation_id":"ipc-data-return","from":from,"format":format})
            );
            let (kind, stats) = receiver
                .request(json!({"action":"data_stats"}), vec![])
                .await;
            assert_eq!(kind, wire::Kind::Response);
            assert_eq!(stats["accepted_messages"], "2");
            assert_eq!(stats["queued_bytes"], 0);
            let (attached, mut alien) = attach(&observer, &token, &token).await;
            attached.unwrap();
            let (kind, refused) = alien.request(command.clone(), bytes.clone()).await;
            assert_eq!(kind, wire::Kind::Failure);
            assert_eq!(refused["kind"], "unsupported");
            assert_eq!(refused["certainty"], "not-submitted");
            assert_eq!(refused["operation"]["operationId"], "ipc-data-send");
            let actor = f.server.store.authenticate(&f.client.credential).unwrap();
            let admin = f.server.store.authenticate(&f.admin).unwrap();
            let store = f.server.store.clone();
            tokio::task::spawn_blocking(move || store.revoke(&admin, &actor.credential_id))
                .await
                .unwrap()
                .unwrap();
            let (kind, refused) = sender.request(command, bytes).await;
            assert_eq!(kind, wire::Kind::Failure);
            // Revocation may retire the authenticated peer before its reply.
            // Preserve uncertainty rather than inventing a refusal receipt.
            if !valid_live_revocation(&refused) {
                let kind = match refused["kind"].as_str() {
                    Some(
                        value @ ("authentication"
                        | "transport-uncertain"
                        | "closed"
                        | "deadline"
                        | "forbidden"
                        | "backpressure"
                        | "protocol"
                        | "unsupported"),
                    ) => value,
                    _ => "other",
                };
                let certainty = match refused["certainty"].as_str() {
                    Some(
                        value @ ("refused" | "not-submitted" | "uncertain" | "read-only"
                        | "confirmed"),
                    ) => value,
                    _ => "other",
                };
                eprintln!(
                    "ipc-data-revocation-failure {}",
                    json!({"kind":kind,"certainty":certainty,"original_operation_matches":refused["operation"]["operationId"] == "ipc-data-send"})
                );
            }
            assert!(valid_live_revocation(&refused), "{refused}");
            let live_revocation_outcome = refused["kind"].clone();
            let live_revocation_certainty = refused["certainty"].clone();
            assert_eq!(refused["operation"]["operationId"], "ipc-data-send");
            let result = observer
                .execute(request(
                    json!({"type":"operation","reference":saved,"kind":"data_send",
                "action":{"type":"status"}}),
                ))
                .await
                .unwrap_err();
            // Shared::request_inner returns Cancelled/ReadOnly when the
            // revoked original control owner has already retired. This is
            // still a failed observation, with no mutation or receipt grant.
            let valid_status_revocation = matches!(
                (result.kind, result.certainty),
                (sdk::ErrorKind::Authentication, sdk::Certainty::Refused)
                    | (sdk::ErrorKind::Authentication, sdk::Certainty::ReadOnly)
                    | (sdk::ErrorKind::TransportUncertain, sdk::Certainty::ReadOnly)
                    | (sdk::ErrorKind::Cancelled, sdk::Certainty::ReadOnly)
            );
            if !valid_status_revocation {
                let value = sdk::helper::error_json(&result);
                eprintln!(
                    "ipc-data-status-revocation-failure {}",
                    json!({"kind":value["kind"],"certainty":value["certainty"],
                    "original_operation_matches":result.operation.as_ref().is_some_and(|key|key.operation_id.as_str()=="ipc-data-send")})
                );
            }
            assert!(valid_status_revocation);
            assert_eq!(
                result.operation.as_ref().unwrap().operation_id.as_str(),
                "ipc-data-send"
            );
            // Deterministically exercise the closed-owner observation branch,
            // independently of the server revocation/retirement scheduling.
            fresh.close(options()).await.unwrap();
            let closed = observer
                .execute(request(json!({"type":"operation","reference":saved,
                    "kind":"data_send","action":{"type":"status"}})))
                .await
                .unwrap_err();
            assert!(matches!(
                (closed.kind, closed.certainty),
                (sdk::ErrorKind::Cancelled, sdk::Certainty::ReadOnly)
            ));
            assert_eq!(
                closed.operation.as_ref().unwrap().operation_id.as_str(),
                "ipc-data-send"
            );
            let Err(rejected) =
                Client::connect(f.client.clone(), "tnt_default".parse().unwrap()).await
            else {
                panic!("Revoked credential authenticated a fresh owner");
            };
            assert_eq!(rejected.kind, sdk::ErrorKind::Authentication);
            assert_eq!(rejected.certainty, sdk::Certainty::ReadOnly);
            let db = rusqlite::Connection::open(&f.config.database).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                    [record.as_str().unwrap()],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM messages WHERE id='ipc-data-changed'",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            observer.close(options()).await.unwrap();
            target_owner.close(options()).await.unwrap();
            source_owner.close(options()).await.unwrap();
            drop((sender, receiver, alien));
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let s = f.server.service.canonical_runtime_snapshot().unwrap();
                    if s.physical_peers == 0 && s.media_readers == 0 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            f.server.shutdown().await.unwrap();
            eprintln!(
                "managed-data-channel-witness {}",
                json!({"profile":media::PROFILE,"algorithm":algorithm,"format":format,
                "payload_bytes":65536,"binary_both_directions":true,"original_control_recovery":true,"one_effect_replay":true,
                "changed_intent_refused":true,"invalid_audience_original_id":true,"foreign_custody_refused":true,
                "current_revocation_refused":true,"live_revocation_outcome":live_revocation_outcome,"live_revocation_certainty":live_revocation_certainty,"original_owners_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
            );
        }
    }
}
