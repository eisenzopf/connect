//! Shared-ledger lifecycle only. Attempt issuance is NOT physical media evidence.
use super::*;
use crate::{AuthorityWait, PeerAuthorityGuard, PeerAuthorityRequest};
use connect_protocol::transitions::{OperationAction, OperationRecord, OperationState};
use std::time::Duration;
fn prepared(f: &Fixture, seconds: i64) -> (PeerAuthorityRequest, PeerAuthorityGuard) {
    let request = f.native_request(seconds);
    let proof = f.admit(&request);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    (request, proof)
}
fn status(f: &Fixture) -> OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "prepare-native".into(),
            },
        )
        .unwrap()
}
#[test]
fn native_effect_correlations_preserve_owner_scoped_offer_and_stream_operation_ids() {
    let mut f = Fixture::new();
    for actor in 0..2 {
        let identity = f.store.revalidate(&f.keys[actor]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[actor] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
        let mut request = f.native_request_for(actor, 600, &format!("same-id-{actor}"));
        let crate::PeerCommand::NativeConnectionOffer(offer) = &mut request.command else {
            panic!()
        };
        offer.operation_id = "shared-offer-id".into();
        let connection = offer.connection_id.clone();
        let proof = f.admit(&request);
        f.store
            .prepare_native_offer_with_peer(
                &f.keys[actor],
                &proof,
                Some(connect_protocol::provisioning::EndpointTransport::Quic),
            )
            .unwrap();
        let (lease, _) = f
            .store
            .begin_native_offer_with_peer(&f.keys[actor], &proof)
            .unwrap()
            .unwrap();
        {
            // Controlled readiness qualifies ledger namespacing only. Runtime
            // tests separately prove actual transport negotiation and IO.
            let c = f.store.connection().unwrap();
            c.execute(
                "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
                [lease.effect_id()],
            )
            .unwrap();
            c.execute(
                "UPDATE operations SET state='completed' WHERE record_id=?1",
                [lease.record_id()],
            )
            .unwrap();
            c.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![connection,f.store.runtime_epoch(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
            c.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),connection,f.store.runtime_epoch(),lease.generation()]).unwrap();
        }
        drop(proof);
        request.command = crate::PeerCommand::NativeStream(
            connect_protocol::native_streams::Intent {
                operation_id: "shared-stream-id".into(),
                conversation_id: f.cid.clone(),
                session_id: "session".into(),
                connection_id: connection,
                expected_revision: Some(1),
                by_participant: None,
                command: connect_protocol::native_streams::Command::Subscribe,
                payload: json!({"subscriptions":[{"from_participant":f.pids[1-actor],"kinds":["audio"]}]}),
            },
        );
        let proof = f.admit(&request);
        f.store
            .accept_native_stream_with_peer(&f.keys[actor], &proof)
            .unwrap();
        assert!(f
            .store
            .begin_native_stream_with_peer(&f.keys[actor], &proof)
            .unwrap()
            .is_some());
    }
    let c = f.store.connection().unwrap();
    let count: u64 = c.query_row("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE e.provider IN ('native-media','native-stream') AND e.correlation_key=o.record_id AND o.operation_id IN ('shared-offer-id','shared-stream-id')", [], |r|r.get(0)).unwrap();
    assert_eq!(count, 4);
}
#[test]
fn native_wire_acceptance_is_separate_from_private_completion_and_can_be_read_under_held_invocation(
) {
    let f = Fixture::new();
    let (_, proof) = prepared(&f, 600);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(
                &f.keys[0],
                &proof,
                connect_protocol::provisioning::EndpointTransport::Quic
            )
            .unwrap(),
        f.endpoints[0]
    );
    let record = f
        .store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(record.operation_id, "prepare-native");
    assert_eq!(record.command_type, "connection.offer");
    assert_eq!(record.state, OperationState::Accepted);
    assert!(record.result.is_none());
    assert_eq!(
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        record
    );
    assert_eq!(status(&f), record);
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Cancel {
                    operation_id: "ungranted-cancel".into(),
                    target_operation_id: record.operation_id.clone(),
                    reason: None,
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(status(&f).state, OperationState::Accepted);

    let (lease, running) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(running.state, OperationState::Preparing);
    let snapshot = f
        .store
        .conversation_snapshot(&f.keys[0], &f.cid, Default::default())
        .unwrap();
    assert!(snapshot.items.iter().any(|item|matches!(item,connect_protocol::events::SnapshotItem::Operation(record) if record.record_id==running.record_id && record.state==OperationState::Preparing)));
    assert_eq!(
        f.store
            .validate_native_offer_attempt(&f.keys[0], &lease, &proof)
            .unwrap()
            .state,
        OperationState::Preparing
    );

    assert!(running.result.is_none());
    assert_ne!(lease.runtime_connection_id(), lease.connection_id());
    assert_eq!(lease.operation_id(), record.operation_id);
    assert!(f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(status(&f).state, OperationState::Preparing);
    let c = f.store.connection().unwrap();
    let private: String = c
        .query_row(
            "SELECT state FROM operations WHERE command_type='connection.prepare'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(private, "completed");
    let not_ready:bool=c.query_row("SELECT readiness_json IS NULL AND observed_runtime_id IS NULL AND state='preparing' FROM connections WHERE id='connection-native'",[],|r|r.get(0)).unwrap();
    assert!(not_ready);
    assert_eq!(f.count_unlocked(&c, "native_offer_operations"), 1);
}
impl Fixture {
    fn count_unlocked(&self, c: &Connection, table: &str) -> u64 {
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn native_wire_replay_conflict_and_binding_proof_cannot_start_or_replace_an_attempt() {
    let f = Fixture::new();
    let (request, proof) = prepared(&f, 600);
    f.store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    let mut changed = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(ref mut wire) = changed.command else {
        panic!()
    };
    wire.offer["extension"] = json!("changed");
    let other = f.admit(&changed);
    assert_eq!(
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &other)
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(other);
    let mut binding = request;
    binding.command = crate::PeerCommand::Binding;
    let binding = f.admit(&binding);
    assert_eq!(
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .begin_native_offer_with_peer(&f.keys[0], &binding)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("native_offer_operations"), 1);
    assert_eq!(status(&f).state, OperationState::Accepted);
}
#[test]
fn native_wire_cancel_before_effect_retires_only_dormant_slot_but_attempted_cancel_remains_unknown()
{
    for started in [false, true] {
        let mut f = Fixture::new();
        let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("operation.cancel".into());
        f.keys[0] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
        let (_, proof) = prepared(&f, 600);
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap();
        let _lease = if started {
            Some(
                f.store
                    .begin_native_offer_with_peer(&f.keys[0], &proof)
                    .unwrap()
                    .unwrap()
                    .0,
            )
        } else {
            None
        };
        let action = OperationAction::Cancel {
            operation_id: "cancel-native".into(),
            target_operation_id: "prepare-native".into(),
            reason: Some("caller cancelled".into()),
        };
        let decision = f
            .store
            .execute_operation_action(&f.keys[0], action.clone())
            .unwrap();
        assert_eq!(
            f.store
                .execute_operation_action(&f.keys[0], action)
                .unwrap(),
            decision
        );
        let record = status(&f);
        assert!(record.cancel_requested);
        assert!(record.result.is_none());
        assert_eq!(
            record.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Cancelled
            }
        );
        assert!(f
            .store
            .begin_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        let c = f.store.connection().unwrap();
        let (state, generation): (String, u64) = c
            .query_row(
                "SELECT state,generation FROM connections WHERE id='connection-native'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (state.as_str(), generation),
            if started {
                ("preparing", 1)
            } else {
                ("retired", 2)
            }
        );
    }
}
#[test]
fn native_wire_source_expiry_before_effect_fails_without_extending_replay_or_allocating_media() {
    let f = Fixture::new();
    let (request, mut proof) = prepared(&f, 600);
    proof.limit_expiry(crate::now() + 2).unwrap();
    let original = f
        .store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    drop(proof);
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(status(&f).state, OperationState::Failed);
    let fresh = f.admit(&request);
    let replay = f
        .store
        .accept_native_offer_with_peer(&f.keys[0], &fresh)
        .unwrap();
    assert_eq!(replay.expires_at, original.expires_at);
    assert_eq!(replay.state, OperationState::Failed);
    assert!(f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &fresh)
        .unwrap()
        .is_none());
    assert_eq!(f.count("native_offer_operations"), 1);
}
#[test]
fn native_wire_restart_after_attempt_marks_unknown_and_never_issues_another_attempt() {
    let f = Fixture::new();
    let (request, proof) = prepared(&f, 600);
    f.store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let journal_before = f.count("journal");
    let limits = Limits {
        journal_events: journal_before + 12,
        ..Limits::default()
    };
    let before = f.store.runtime_epoch().to_owned();
    let path = f.store.path().to_owned();
    let credential = f.keys[0].clone();
    drop(proof);
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let store = Store::open(&path, limits).unwrap();
    assert_ne!(store.runtime_epoch(), before);
    assert_eq!(store.recover_native_offers(32).unwrap(), 1);
    assert_eq!(store.recover_native_offers(32).unwrap(), 0);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        journal_before + 1
    );
    let proof = store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let replay = store
        .accept_native_offer_with_peer(&credential, &proof)
        .unwrap();
    assert_eq!(replay.state, OperationState::Unknown);
    assert!(replay.result.is_none());
    assert_eq!(replay.effects.len(), 1);
    assert_eq!(replay.effects[0].id, lease.effect_id());
    assert_eq!(
        replay.effects[0].state,
        connect_protocol::transitions::EffectState::Unknown
    );

    assert!(store
        .begin_native_offer_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
    let c = store.connection().unwrap();
    let mapping:String=c.query_row("SELECT runtime_connection_id FROM connection_offers WHERE connection_id='connection-native'",[],|r|r.get(0)).unwrap();
    assert_eq!(mapping, lease.runtime_connection_id());

    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_offer_operations", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    drop(c);
    assert_eq!(
        store
            .validate_native_offer_attempt(&credential, &lease, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(proof);
    drop(store);
    drop(_dir);
}

#[test]
fn native_wire_accept_and_begin_storage_failures_roll_back_before_issuing_an_attempt() {
    let f = Fixture::new();
    let (_, proof) = prepared(&f, 600);
    let operations = f.count("operations");
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_native_accept_failure BEFORE INSERT ON native_offer_operations BEGIN SELECT RAISE(ABORT,'controlled accept outage'); END;").unwrap();
    assert_eq!(
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(f.count("native_offer_operations"), 0);
    assert_eq!(f.count("operations"), operations);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_native_accept_failure")
        .unwrap();
    let original = f
        .store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_native_begin_failure BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' AND EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=NEW.operation_record_id) BEGIN SELECT RAISE(ABORT,'controlled begin outage'); END;").unwrap();
    assert_eq!(
        f.store
            .begin_native_offer_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "storage-unavailable"
    );
    assert_eq!(status(&f).state, OperationState::Accepted);
    assert_eq!(f.count("operation_effects"), 0);
    let started: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT effect_started FROM native_offer_operations WHERE operation_record_id=?1",
            [&original.record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!started);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_native_begin_failure")
        .unwrap();
    let (lease, running) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(lease.record_id(), original.record_id);
    assert_eq!(running.effects.len(), 1);
    assert_eq!(
        running.effects[0].state,
        connect_protocol::transitions::EffectState::Submitting
    );
    assert_eq!(running.effects[0].id, lease.effect_id());
}
#[test]
fn native_wire_concurrent_claims_issue_one_attempt_and_one_stable_effect() {
    let f = Fixture::new();
    let (request, first) = prepared(&f, 600);
    f.store
        .accept_native_offer_with_peer(&f.keys[0], &first)
        .unwrap();
    let second = f.admit(&request);
    let claims = std::thread::scope(|scope| {
        let store = &f.store;
        let credential = &f.keys[0];
        let a = scope.spawn(move || store.begin_native_offer_with_peer(credential, &first));
        let b = scope.spawn(move || store.begin_native_offer_with_peer(credential, &second));
        vec![a.join().unwrap().unwrap(), b.join().unwrap().unwrap()]
    });
    assert_eq!(claims.iter().filter(|v| v.is_some()).count(), 1);
    assert_eq!(f.count("operation_effects"), 1);
    assert_eq!(f.count("connections"), 1);
    assert_eq!(status(&f).state, OperationState::Preparing);
}

fn extra_endpoint(f: &Fixture) -> String {
    extra_endpoint_for_transport(f, EndpointTransport::Quic)
}
fn extra_endpoint_for_transport(f: &Fixture, transport: EndpointTransport) -> String {
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("extra-{}-endpoint", transport.as_str()),
                identity_id: identity,
                device: DeviceSpec {
                    id: "device-0".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                address: if transport == EndpointTransport::Quic {
                    "quic://localhost:4451".into()
                } else {
                    "https://localhost:4451/connect".into()
                },
                transport,
                capabilities: vec!["audio".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    endpoint_id
}
#[test]
fn native_endpoint_selection_refuses_missing_and_ambiguous_profiles_without_creating_slots() {
    for ambiguous in [false, true] {
        let f = Fixture::new();
        let request = f.native_request(600);
        if ambiguous {
            let endpoint = extra_endpoint(&f);
            f.store
                .execute_provisioning(
                    &f.admin,
                    ProvisioningCommand::VerifyEndpoint {
                        operation_id: "verify-extra".into(),
                        endpoint_id: endpoint,
                        expected_revision: 0,
                        evidence_ref: "controlled-fixture".into(),
                    },
                )
                .unwrap();
        } else {
            f.store
                .execute_provisioning(
                    &f.admin,
                    ProvisioningCommand::RevokeEndpoint {
                        operation_id: "revoke-native-endpoint".into(),
                        endpoint_id: f.endpoints[0].clone(),
                        expected_revision: 0,
                    },
                )
                .unwrap();
        }
        let proof = f.admit(&request);
        assert_eq!(
            f.store
                .select_native_endpoint_with_peer(
                    &f.keys[0],
                    &proof,
                    connect_protocol::provisioning::EndpointTransport::Quic
                )
                .unwrap_err()
                .code,
            if ambiguous { "conflict" } else { "not-ready" }
        );
        assert_eq!(f.count("connections"), 0);
        assert_eq!(f.count("native_offer_operations"), 0);
    }
}
#[test]
fn native_endpoint_replay_preserves_saved_selection_instead_of_switching_to_another_profile() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let extra = extra_endpoint(&f);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(
                &f.keys[0],
                &proof,
                connect_protocol::provisioning::EndpointTransport::Quic
            )
            .unwrap(),
        f.endpoints[0]
    );
    f.store
        .prepare_connection_with_peer(
            &f.keys[0],
            f.request(0, "native").intent,
            f.endpoints[0].clone(),
            &proof,
        )
        .unwrap();
    drop(proof);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-extra".into(),
                endpoint_id: extra,
                expected_revision: 0,
                evidence_ref: "controlled-fixture".into(),
            },
        )
        .unwrap();
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(
                &f.keys[0],
                &proof,
                connect_protocol::provisioning::EndpointTransport::Quic
            )
            .unwrap(),
        f.endpoints[0]
    );
}

fn cancellable() -> Fixture {
    let mut f = Fixture::new();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("operation.cancel".into());
    f.keys[0] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    f
}
fn cancel_action(id: &str) -> OperationAction {
    OperationAction::Cancel {
        operation_id: id.into(),
        target_operation_id: "prepare-native".into(),
        reason: None,
    }
}
#[test]
fn native_dormant_retirement_fanout_and_decision_are_atomic_and_replay_exactly() {
    let f = cancellable();
    let anchors = f
        .keys
        .iter()
        .map(|key| {
            f.store
                .conversation_events(key, &f.cid, Default::default())
                .unwrap()
                .cursor
        })
        .collect::<Vec<_>>();
    let (_, proof) = prepared(&f, 600);
    f.store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_retirement_outage BEFORE INSERT ON journal WHEN NEW.kind='connection_retired' BEGIN SELECT RAISE(ABORT,'controlled retirement outage'); END;").unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(&f.keys[0], cancel_action("cancel-atomic"))
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(status(&f).state, OperationState::Accepted);
    let unchanged: (String, u64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state,generation FROM connections WHERE id='connection-native'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(unchanged, ("preparing".into(), 1));
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_retirement_outage")
        .unwrap();
    let decision = f
        .store
        .execute_operation_action(&f.keys[0], cancel_action("cancel-atomic"))
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action(&f.keys[0], cancel_action("cancel-atomic"))
            .unwrap(),
        decision
    );
    assert_eq!(status(&f).state, OperationState::Cancelled);
    assert_eq!(f.count("operation_effects"), 0);
    let facts: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM journal WHERE kind='connection_retired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(facts, 4);
    drop(proof);
    for (actor, key) in f.keys.iter().enumerate() {
        let page = f
            .store
            .conversation_events(
                key,
                &f.cid,
                connect_protocol::events::EventQuery {
                    after: Some(anchors[actor].clone()),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap();
        let facts = page
            .events
            .iter()
            .filter_map(|event| match &event.data {
                connect_protocol::events::JournalData::ConnectionRetired(value) => {
                    Some((event, value))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(facts.len(), 1);
        let (event, value) = facts[0];
        assert_eq!(
            event.connection_id.as_deref(),
            Some(value.connection_id.as_str())
        );
        assert_eq!(event.session_id.as_deref(), Some("session"));
        assert_eq!(event.participant_id.as_deref(), Some(f.pids[0].as_str()));
        assert_eq!(
            (value.generation, value.revision, value.state.as_str()),
            (2, 1, "retired")
        );
        assert_eq!(event.source.kind, "connect-native-pre-effect-retirement");
        assert_eq!(
            event.operation_id.as_deref(),
            if actor == 0 {
                Some("prepare-native")
            } else {
                None
            }
        );
        if let Ok(directory) = std::env::var("CONNECT_GATE6_NATIVE_RETIREMENT_SAMPLES") {
            let directory = std::path::Path::new(&directory);
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(
                directory.join(format!("participant-{actor}.json")),
                serde_json::to_vec_pretty(&connect_protocol::Outcome::ConversationEvents(page))
                    .unwrap(),
            )
            .unwrap();
        }
    }
}
#[test]
fn native_split_budgets_admit_one_cancel_receipt_and_preserve_unknown_cleanup_fanout() {
    for started in [false, true] {
        let mut f = cancellable();
        let (_, proof) = prepared(&f, 600);
        let operations = f.count("operations");
        let journal = f.count("journal");
        // Four recipients need five ready facts, five future cleanup facts,
        // two uncertainty/cancel facts, acceptance and optional preparing.
        f.store.limits.operation_receipts = operations + 2;
        f.store.limits.journal_events = journal + 13 + u64::from(started);
        f.store
            .accept_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap();
        if started {
            f.store
                .begin_native_offer_with_peer(&f.keys[0], &proof)
                .unwrap()
                .unwrap();
        }
        let denied = Operation::Send {
            request_id: "cannot-use-reserve".into(),
            conversation_id: f.cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("capacity test"),
        };
        assert_eq!(
            f.store.execute(&f.keys[0], &denied).unwrap_err().code,
            "capacity-exceeded"
        );
        f.store
            .execute_operation_action(&f.keys[0], cancel_action("cancel-at-capacity"))
            .unwrap();
        assert_eq!(f.count("operations"), operations + 2);
        if started {
            assert_eq!(status(&f).state, OperationState::Unknown);
            // Receipt space is independent; let a new writer reach the journal
            // check and prove uncertainty did not release cleanup's fanout.
            f.store.limits.operation_receipts += 10;
            f.store.limits.journal_events = f.count("journal") + 5;
            assert_eq!(
                f.store.execute(&f.keys[0], &denied).unwrap_err().code,
                "capacity-exceeded"
            );
            let c = f.store.connection().unwrap();
            let consumed: u64 = c
                .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
                .unwrap();
            assert!(
                crate::tasks::budget::journal(&c, f.store.limits.journal_events, consumed, 0)
                    .unwrap()
            );
            assert!(
                !crate::tasks::budget::journal(&c, f.store.limits.journal_events, consumed, 2)
                    .unwrap()
            );
        } else {
            assert_eq!(status(&f).state, OperationState::Cancelled);
        }
    }
}

#[test]
fn controlled_native_successor_cleanup_rejects_wrong_observation_and_rolls_back_failed_facts() {
    use crate::{
        NativeOfferObservation, NativeOfferRecoveryObservation, NativeOfferRuntimeObservation,
    };
    struct Observation {
        wrong: bool,
    }
    impl NativeOfferRecoveryObservation for Observation {
        fn observe_retired(
            &self,
            id: &str,
            generation: u64,
        ) -> connect_protocol::Result<NativeOfferObservation> {
            Ok(NativeOfferObservation::Retired {
                runtime_connection_id: if self.wrong {
                    "wrong-route".into()
                } else {
                    id.into()
                },
                generation,
            })
        }
    }
    struct OldLease;
    impl NativeOfferRuntimeObservation for OldLease {
        fn observe(&self) -> connect_protocol::Result<NativeOfferObservation> {
            unreachable!("foreign attempt cannot use current runtime cleanup")
        }
    }
    let f = Fixture::new();
    let (_, proof) = prepared(&f, 600);
    f.store
        .accept_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .recover_native_offer_cleanup(&Observation { wrong: true })
            .unwrap(),
        0
    );
    let path = f.store.path().to_owned();
    drop(proof);
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let store = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        store
            .settle_native_offer_cleanup(&lease, &OldLease)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let snapshot = || {
        let c = store.connection().unwrap();
        c.query_row("SELECT o.state,c.state,e.state,(SELECT COUNT(*) FROM journal) FROM operations o JOIN native_offer_operations n ON n.operation_record_id=o.record_id JOIN connections c ON c.id=o.connection_id JOIN operation_effects e ON e.id=n.effect_id WHERE o.record_id=?1",[lease.record_id()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?))).unwrap()
    };
    let before = snapshot();
    assert!(store
        .recover_native_offer_cleanup(&Observation { wrong: true })
        .is_err());
    assert_eq!(snapshot(), before);
    store.connection().unwrap().execute_batch("CREATE TRIGGER reject_native_cleanup_fact BEFORE INSERT ON journal WHEN NEW.kind='connection_retired' BEGIN SELECT RAISE(ABORT,'injected cleanup journal failure'); END;").unwrap();
    assert!(store
        .recover_native_offer_cleanup(&Observation { wrong: false })
        .is_err());
    assert_eq!(snapshot(), before);
    store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_native_cleanup_fact;")
        .unwrap();
    assert_eq!(
        store
            .recover_native_offer_cleanup(&Observation { wrong: false })
            .unwrap(),
        1
    );
    let after = snapshot();
    assert_eq!(
        (&after.0, &after.1, &after.2),
        (
            &"failed".to_owned(),
            &"retired".to_owned(),
            &"failed".to_owned()
        )
    );
    assert_eq!(
        store
            .recover_native_offer_cleanup(&Observation { wrong: false })
            .unwrap(),
        0
    );
    assert_eq!(snapshot(), after);
}

#[test]
fn native_transport_selection_preserves_verified_endpoint_and_refuses_cross_transport_replay() {
    let f = Fixture::new();
    let mut request = f.native_request(600);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["substrate"] = json!("webtransport");
    let wt = extra_endpoint_for_transport(&f, EndpointTransport::Webtransport);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webtransport)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Quic)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 0);
    drop(proof);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-webtransport".into(),
                endpoint_id: wt.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-store-fixture-not-physical-media".into(),
            },
        )
        .unwrap();
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webtransport)
            .unwrap(),
        wt
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Quic)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webrtc)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, None)
            .unwrap_err()
            .code,
        "unsupported-media-profile"
    );
    assert_eq!(f.count("connections"), 0);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Quic))
            .unwrap_err()
            .code,
        "unsupported-media-profile"
    );
    assert_eq!(f.count("connections"), 0);
    let record = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Webtransport))
        .unwrap();
    assert_eq!(record.state, OperationState::Accepted);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, None)
            .unwrap(),
        record
    );
    let projected = f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(projected.connection.endpoint_id, wt);
    assert_eq!(
        projected.endpoint_transport,
        EndpointTransport::Webtransport
    );
    assert_eq!(projected.endpoint_address, "https://localhost:4451/connect");
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Quic)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(
                &f.keys[0],
                &proof,
                Some(EndpointTransport::Webtransport)
            )
            .unwrap(),
        record
    );
    assert_eq!(f.count("connections"), 1);
    assert_eq!(f.count("native_offer_operations"), 1);
    assert_eq!(f.count("operation_effects"), 0);
    drop(proof);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-webtransport".into(),
                endpoint_id: wt,
                expected_revision: 1,
            },
        )
        .unwrap();
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webtransport)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 1);
    assert_eq!(f.count("operation_effects"), 0);
}

fn web_rtc_request(f: &Fixture) -> PeerAuthorityRequest {
    let mut request = f.native_request(600);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["substrate"] = json!("websocket+webrtc");
    intent.offer["substrate_setup"] =
        json!({"kind":"websocket+webrtc","sdp":"v=0\r\ns=synthetic-store-contract-only\r\n"});
    request
}
fn verify_web_rtc(f: &Fixture, endpoint: &str) {
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-web-rtc".into(),
                endpoint_id: endpoint.into(),
                expected_revision: 0,
                evidence_ref: "controlled-store-fixture-not-physical-media".into(),
            },
        )
        .unwrap();
}
#[test]
fn web_rtc_preparation_requires_verified_exact_device_transport_and_never_reports_media() {
    let f = Fixture::new();
    let request = web_rtc_request(&f);
    let endpoint = extra_endpoint_for_transport(&f, EndpointTransport::Webrtc);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webrtc)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Quic)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 0);
    drop(proof);
    verify_web_rtc(&f, &endpoint);
    let proof = f.admit(&request);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    // A verified endpoint belonging to this same Device but another technology
    // cannot be smuggled through direct preparation either.
    assert_eq!(
        f.store
            .prepare_connection_with_peer(
                &f.keys[0],
                intent.clone(),
                f.endpoints[0].clone(),
                &proof
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 0);
    let original = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Webrtc))
        .unwrap();
    assert_eq!(original.state, OperationState::Accepted);
    assert!(original.result.is_none());
    let projection = f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(projection.endpoint_transport, EndpointTransport::Webrtc);
    assert_eq!(projection.connection.endpoint_id, endpoint);
    assert_eq!(projection.device_id, "device-0");
    assert_eq!(projection.connection.state, "preparing");
    assert_eq!(projection.connection.revision, 0);
    assert_eq!(projection.offer, intent.offer);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, None)
            .unwrap(),
        original
    );
    assert_eq!(f.count("connections"), 1);
    assert_eq!(f.count("native_offer_operations"), 1);
    assert_eq!(f.count("operation_effects"), 0);
}
#[test]
fn web_rtc_original_sdp_replay_cannot_select_new_intent_or_revoked_endpoint() {
    let f = Fixture::new();
    let request = web_rtc_request(&f);
    let endpoint = extra_endpoint_for_transport(&f, EndpointTransport::Webrtc);
    verify_web_rtc(&f, &endpoint);
    let proof = f.admit(&request);
    let original = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Webrtc))
        .unwrap();
    drop(proof);
    let mut changed = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut changed.command else {
        panic!()
    };
    intent.offer["substrate_setup"]["sdp"] = json!("v=0\r\ns=changed-under-original-id\r\n");
    let changed_proof = f.admit(&changed);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(
                &f.keys[0],
                &changed_proof,
                Some(EndpointTransport::Webrtc)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(changed_proof);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, None)
            .unwrap(),
        original
    );
    drop(proof);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-web-rtc".into(),
                endpoint_id: endpoint,
                expected_revision: 1,
            },
        )
        .unwrap();
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .project_connection_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    // The journal remains readable under the owner's current scope. Its old
    // accepted receipt cannot authorize current endpoint/attempt creation.
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof, None)
            .unwrap(),
        original
    );
    assert_eq!(
        f.store
            .select_native_endpoint_with_peer(&f.keys[0], &proof, EndpointTransport::Webrtc)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .begin_native_offer_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 1);
    assert_eq!(f.count("operation_effects"), 0);
}
#[test]
fn direct_preparation_cannot_pair_a_native_offer_with_a_verified_web_rtc_endpoint() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let endpoint = extra_endpoint_for_transport(&f, EndpointTransport::Webrtc);
    verify_web_rtc(&f, &endpoint);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .prepare_connection_with_peer(&f.keys[0], intent.clone(), endpoint, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 0);
    assert_eq!(f.count("operation_effects"), 0);
}
