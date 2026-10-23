//! Wire failure identity survives local refusal without claiming media/Connection.
use super::*;
use crate::{PeerAuthorityGuard, PeerAuthorityRequest, PeerCommand};
use connect_protocol::transitions::{OperationAction, OperationState};
fn rejected(f: &Fixture, stale: bool) -> (PeerAuthorityRequest, PeerAuthorityGuard) {
    let mut request = f.native_request(600);
    let PeerCommand::NativeConnectionOffer(wire) = &mut request.command else {
        panic!()
    };
    if stale {
        wire.expected_revision = Some(1)
    } else {
        wire.offer["streams_offered"][0]["kind"] = json!("video");
        wire.offer["streams_offered"][0]["codec_preferences"] = json!(["vp8"]);
    }
    let proof = f.admit(&request);
    (request, proof)
}
#[test]
fn native_preparation_refusals_keep_wire_failure_and_never_bind_or_allocate_a_connection() {
    for stale in [false, true] {
        let f = Fixture::new();
        let (request, proof) = rejected(&f, stale);
        let record = f
            .store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap();
        assert_eq!(
            (
                record.operation_id.as_str(),
                record.command_type.as_str(),
                record.state
            ),
            ("prepare-native", "connection.offer", OperationState::Failed)
        );
        assert_eq!(record.session_id.as_deref(), Some("session"));
        assert!(
            record.connection_id.is_none() && record.result.is_none() && record.effects.is_empty()
        );
        assert_eq!(record.expected_revision, Some(if stale { 1 } else { 0 }));
        assert_eq!(
            f.store
                .prepare_native_offer_with_peer(&f.keys[0], &proof)
                .unwrap(),
            record
        );
        assert_eq!(
            f.store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: "prepare-native".into()
                    }
                )
                .unwrap(),
            record
        );
        assert!(f
            .store
            .begin_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        assert_eq!(f.count("connections"), 0);
        assert_eq!(f.count("connection_offers"), 0);
        assert_eq!(f.count("native_offer_operations"), 0);
        assert_eq!(f.count("native_offer_rejections"), 1);
        assert_eq!(f.count("operation_effects"), 0);
        let c = f.store.connection().unwrap();
        let (private,problem):(String,String)=c.query_row("SELECT n.preparation_record_id,o.problem_json FROM native_offer_rejections n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.record_id=?1",[&record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_ne!(private, record.record_id);
        assert_eq!(
            serde_json::from_str::<connect_protocol::Error>(&problem)
                .unwrap()
                .code,
            if stale {
                "conflict"
            } else {
                "unsupported-media-profile"
            }
        );
        assert!(c
            .execute(
                "UPDATE operations SET state='accepted' WHERE record_id=?1",
                [&record.record_id]
            )
            .is_err());
        assert!(c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES('forbidden-effect',?1,'native-media','prepare-native','none','submitting',1,1)",[&record.record_id]).is_err());
        drop(c);
        let mut changed = request;
        let PeerCommand::NativeConnectionOffer(wire) = &mut changed.command else {
            panic!()
        };
        wire.offer["extension"] = json!(true);
        let changed = f.admit(&changed);
        assert_eq!(
            f.store
                .prepare_native_offer_with_peer(&f.keys[0], &changed)
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(f.count("native_offer_rejections"), 1);
    }
}
#[test]
fn native_rejection_storage_failure_rolls_back_wire_state_then_original_id_recovers() {
    let f = Fixture::new();
    let (_, proof) = rejected(&f, false);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_wire_rejection_outage BEFORE INSERT ON journal WHEN EXISTS(SELECT 1 FROM native_offer_rejections n WHERE n.operation_record_id=NEW.operation_record_id) BEGIN SELECT RAISE(ABORT,'controlled wire rejection outage'); END;").unwrap();
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(f.count("native_offer_rejections"), 0);
    assert_eq!(f.count("connections"), 0);
    let count: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='prepare-native'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_wire_rejection_outage")
        .unwrap();
    let record = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(record.state, OperationState::Failed);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        record
    );
    assert_eq!(f.count("native_offer_rejections"), 1);
}
#[test]
fn failed_wire_target_does_not_retarget_a_later_valid_slot_and_survives_reopen() {
    let f = Fixture::new();
    let (request, proof) = rejected(&f, false);
    let original = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    let mut valid = request.clone();
    let PeerCommand::NativeConnectionOffer(wire) = &mut valid.command else {
        panic!()
    };
    wire.operation_id = "later-valid-offer".into();
    wire.offer["streams_offered"][0]["kind"] = json!("audio");
    wire.offer["streams_offered"][0]["codec_preferences"] = json!(["opus"]);
    let next = f.admit(&valid);
    let accepted = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &next)
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    assert_eq!(accepted.connection_id.as_deref(), Some("connection-native"));
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        original
    );
    assert!(f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    let path = f.store.path().to_owned();
    let credential = f.keys[0].clone();
    drop(proof);
    drop(next);
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let store = Store::open(&path, Limits::default()).unwrap();
    let proof = store
        .admit_peer_authority(
            &request,
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    assert_eq!(
        store
            .prepare_native_offer_with_peer(&credential, &proof)
            .unwrap(),
        original
    );
    assert!(store
        .begin_native_offer_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
    drop(proof);
    drop(store);
    drop(_dir);
}

#[test]
fn native_failed_preparation_recovers_after_endpoint_retirement_without_expanding_read_authority() {
    let f = Fixture::new();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let narrow = f
        .store
        .issue(
            &identity,
            Scope::collaboration(vec![identity.clone()]),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let (request, proof) = rejected(&f, false);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_rejection_outage BEFORE INSERT ON native_offer_rejections BEGIN SELECT RAISE(ABORT,'controlled outage'); END;").unwrap();
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_rejection_outage")
        .unwrap();
    drop(proof);
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "retire-rejected-endpoint".into(),
                endpoint_id: f.endpoints[0].clone(),
                expected_revision: 0,
            },
        )
        .unwrap();
    let fresh = f.admit(&request);
    let failed = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &fresh)
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    assert!(failed.connection_id.is_none());
    assert!(f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &fresh)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .execute_operation_action(
                &narrow,
                OperationAction::Get {
                    operation_id: "prepare-native".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[1],
                OperationAction::Get {
                    operation_id: "prepare-native".into()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("connections"), 0);
    assert_eq!(f.count("operation_effects"), 0);
}
#[test]
fn concurrent_native_refusals_have_one_private_receipt_and_one_wire_failure() {
    let f = Fixture::new();
    let (request, first) = rejected(&f, false);
    let second = f.admit(&request);
    let results = std::thread::scope(|scope| {
        let store = &f.store;
        let credential = &f.keys[0];
        let a = scope.spawn(move || store.prepare_native_offer_with_peer(credential, &first));
        let b = scope.spawn(move || store.prepare_native_offer_with_peer(credential, &second));
        [a.join().unwrap().unwrap(), b.join().unwrap().unwrap()]
    });
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0].state, OperationState::Failed);
    assert_eq!(f.count("native_offer_rejections"), 1);
    assert_eq!(f.count("connections"), 0);
    let count:u64=f.store.connection().unwrap().query_row("SELECT COUNT(*) FROM operations WHERE command_type IN ('connection.offer','connection.prepare')",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
}
#[test]
fn conflicting_native_creation_does_not_bind_a_failure_to_another_participants_existing_slot() {
    let f = Fixture::new();
    let request = f.native_request_for(1, 600, "native");
    let proof = f.admit(&request);
    let other = f
        .store
        .prepare_native_offer_with_peer(&f.keys[1], &proof)
        .unwrap();
    assert_eq!(other.state, OperationState::Accepted);
    drop(proof);
    let request = f.native_request(600);
    let proof = f.admit(&request);
    let failed = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    assert!(failed.connection_id.is_none());
    assert_eq!(f.count("connections"), 1);
    let owner: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT participant_id FROM connections WHERE id='connection-native'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, f.pids[1]);
    assert!(f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
}

#[test]
fn trusted_native_planner_can_refuse_only_an_unattempted_offer_and_rolls_back_failed_fanout() {
    use crate::NativeOfferPreflightRefusal as Refusal;
    for attempted in [false, true] {
        let f = Fixture::new();
        let request = f.native_request(600);
        let proof = f.admit(&request);
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap();
        if attempted {
            f.store
                .begin_native_offer_with_peer(&f.keys[0], &proof)
                .unwrap()
                .unwrap();
            assert_eq!(
                f.store
                    .refuse_native_offer_before_effect_with_peer(
                        &f.keys[0],
                        &proof,
                        Refusal::InvalidParameters
                    )
                    .unwrap_err()
                    .code,
                "forbidden"
            );
            let state = f
                .store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: "prepare-native".into(),
                    },
                )
                .unwrap()
                .state;
            assert_eq!(state, OperationState::Preparing);
            assert_eq!(f.count("operation_effects"), 1);
        } else {
            f.store.connection().unwrap().execute_batch("CREATE TRIGGER fixture_planner_outage BEFORE INSERT ON journal WHEN NEW.kind='connection_retired' BEGIN SELECT RAISE(ABORT,'controlled planner outage'); END;").unwrap();
            assert_eq!(
                f.store
                    .refuse_native_offer_before_effect_with_peer(
                        &f.keys[0],
                        &proof,
                        Refusal::InvalidParameters
                    )
                    .unwrap_err()
                    .code,
                "storage-unavailable"
            );
            let state = f
                .store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Get {
                        operation_id: "prepare-native".into(),
                    },
                )
                .unwrap()
                .state;
            assert_eq!(state, OperationState::Accepted);
            let problem: Option<String> = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT problem_json FROM operations WHERE operation_id='prepare-native'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(problem.is_none());
            f.store
                .connection()
                .unwrap()
                .execute_batch("DROP TRIGGER fixture_planner_outage")
                .unwrap();
            let failed = f
                .store
                .refuse_native_offer_before_effect_with_peer(
                    &f.keys[0],
                    &proof,
                    Refusal::InvalidParameters,
                )
                .unwrap();
            assert_eq!(failed.state, OperationState::Failed);
            assert_eq!(
                f.store
                    .refuse_native_offer_before_effect_with_peer(
                        &f.keys[0],
                        &proof,
                        Refusal::InvalidParameters
                    )
                    .unwrap(),
                failed
            );
            assert_eq!(
                f.store
                    .refuse_native_offer_before_effect_with_peer(
                        &f.keys[0],
                        &proof,
                        Refusal::Capacity
                    )
                    .unwrap_err()
                    .code,
                "conflict"
            );
            assert_eq!(f.count("operation_effects"), 0);
            let state: String = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT state FROM connections WHERE id='connection-native'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "retired");
        }
    }
}
