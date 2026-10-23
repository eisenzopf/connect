//! Controlled ledger source states test authorization, never physical readiness.
//! The production server suite owns real native resource/admission evidence.
use super::*;
use crate::{AuthorityWait, PeerAuthorityRequest, PeerCommand};
use connect_protocol::native_streams::{Command, Intent};
use std::time::Duration;
mod flow_facts;

fn parent(granted: bool, completed: bool) -> (Fixture, PeerAuthorityRequest, String) {
    parent_scoped(granted, completed, false)
}
fn parent_scoped(
    granted: bool,
    completed: bool,
    end: bool,
) -> (Fixture, PeerAuthorityRequest, String) {
    parent_media(granted, completed, end, false)
}
fn parent_media(
    granted: bool,
    completed: bool,
    end: bool,
    data: bool,
) -> (Fixture, PeerAuthorityRequest, String) {
    parent_permissions(granted, completed, end, data, true)
}
fn parent_permissions(
    granted: bool,
    completed: bool,
    end: bool,
    data: bool,
    send: bool,
) -> (Fixture, PeerAuthorityRequest, String) {
    let (f, request, record, _) = parent_permissions_origin(granted, completed, end, data, send);
    (f, request, record)
}
fn parent_permissions_origin(
    granted: bool,
    completed: bool,
    end: bool,
    data: bool,
    send: bool,
) -> (
    Fixture,
    PeerAuthorityRequest,
    String,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    parent_permissions_origin_extra(granted, completed, end, data, send, &[])
}
fn parent_permissions_origin_extra(
    granted: bool,
    completed: bool,
    end: bool,
    data: bool,
    send: bool,
    extra: &[&str],
) -> (
    Fixture,
    PeerAuthorityRequest,
    String,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    parent_permissions_origin_filtered(granted, completed, end, data, send, extra, &[])
}
fn parent_permissions_origin_filtered(
    granted: bool,
    completed: bool,
    end: bool,
    data: bool,
    send: bool,
    extra: &[&str],
    excluded: &[&str],
) -> (
    Fixture,
    PeerAuthorityRequest,
    String,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    let mut f = if data {
        Fixture::with_endpoint_capabilities(vec!["data".into()])
    } else {
        Fixture::new()
    };
    if granted || !send || !extra.is_empty() || !excluded.is_empty() {
        let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        if granted {
            scope.operations.extend([
                "stream.subscribe".into(),
                "stream.unsubscribe".into(),
                "operation.cancel".into(),
            ]);
        }
        if !send {
            scope.operations.retain(|operation| operation != "send");
        }
        scope
            .operations
            .extend(extra.iter().map(|scope| (*scope).to_owned()));
        if end {
            scope.operations.push("connection.end".into());
        }
        scope
            .operations
            .retain(|op| !excluded.contains(&op.as_str()));
        f.keys[0] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let mut request = f.native_request(600);
    if data {
        // Controlled source metadata, like the controlled readiness below;
        // no production Session negotiation or native port is claimed here.
        let c = f.store.connection().unwrap();
        c.execute(
            r#"UPDATE sessions SET media='["data"]' WHERE id='session'"#,
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE session_control SET capabilities_json=?1 WHERE session_id='session'",
            [json!({"data":["reliable-ordered"]}).to_string()],
        )
        .unwrap();
        c.execute("UPDATE session_presence_capabilities SET capabilities_json=?1 WHERE session_id='session'",[json!({"data":["reliable-ordered"]}).to_string()]).unwrap();

        let PeerCommand::NativeConnectionOffer(offer) = &mut request.command else {
            panic!()
        };
        offer.offer = json!({"substrate":"quic","capabilities":{"data_protocols":["json","binary"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"application","kind":"data","direction":"sendonly","codec_preferences":["json","binary"]}]});
    }

    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    if completed {
        // State fixture only. This does not construct an engine observation or
        // exercise Store completion, publish media or claim a runtime result.
        let mut c = f.store.connection().unwrap();
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [lease.effect_id()],
        )
        .unwrap();
        tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":[if data {"application"} else {"audio"}],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
        tx.execute(
            "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
            [lease.record_id()],
        )
        .unwrap();
        tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
        tx.commit().unwrap();
    }
    drop(proof);
    let record = lease.record_id().to_owned();
    let PeerCommand::NativeConnectionOffer(offer) = &request.command else {
        panic!()
    };
    let mut command = request.clone();
    command.command = PeerCommand::NativeStream(Intent {
        operation_id: "select-audio".into(),
        conversation_id: offer.conversation_id.clone(),
        session_id: offer.session_id.clone(),
        connection_id: offer.connection_id.clone(),
        expected_revision: Some(1),
        by_participant: None,
        command: Command::Subscribe,
        payload: json!({"subscriptions":[{"from_participant":f.pids[1],"kinds":["audio"]}]}),
    });
    (f, command, record, origin)
}
fn admit(f: &Fixture, request: &PeerAuthorityRequest) -> crate::Result<crate::PeerAuthorityGuard> {
    f.store
        .admit_peer_authority(request, &AuthorityWait::new(Duration::from_secs(1)))
}
#[test]
fn current_native_stream_source_binds_parent_and_reads_without_effect_or_receipt() {
    let (f, request, record) = parent(true, true);
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let proof = admit(&f, &request).unwrap();
    let view = f
        .store
        .project_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(view.parent_record_id, record);
    assert_eq!(view.parent_operation_id, "prepare-native");
    assert_eq!(view.stream_revision, 1);
    assert_eq!(
        view.connection.connection.connection_id,
        "connection-native"
    );
    assert_eq!(proof.native_member(), Some((f.pids[0].as_str(), 1)));
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    // Expiring the historical full operation result must not erase the
    // immutable offer/source mapping for a still-current native lifetime.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET intent_json=NULL,tombstoned=1 WHERE record_id=?1",
            [&record],
        )
        .unwrap();
    assert_eq!(
        f.store
            .project_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .parent_record_id,
        record
    );
    let mut binding = request.clone();
    binding.command = PeerCommand::Binding;
    let binding = admit(&f, &binding).unwrap();
    assert_eq!(
        f.store
            .project_native_stream_with_peer(&f.keys[0], &binding)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert_eq!(
        foreign
            .project_native_stream_with_peer(&f.keys[0], &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}
#[test]
fn preparing_source_and_creation_permission_cannot_authorize_subscription() {
    for (granted, completed) in [(true, false), (false, true)] {
        let (f, request, _) = parent(granted, completed);
        let before = (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal"),
        );
        assert_eq!(admit(&f, &request).err().unwrap().code, "forbidden");
        assert_eq!(
            before,
            (
                f.count("operations"),
                f.count("operation_effects"),
                f.count("journal")
            )
        );
    }
}
#[test]
fn source_assertions_device_and_retirement_are_rechecked_before_projection() {
    let (f, request, _) = parent(true, true);
    for field in ["conversation", "session", "connection", "actor", "device"] {
        let mut wrong = request.clone();
        let PeerCommand::NativeStream(intent) = &mut wrong.command else {
            panic!()
        };
        match field {
            "conversation" => intent.conversation_id = "another-room".into(),
            "session" => intent.session_id = "another-session".into(),
            "connection" => intent.connection_id = "another-connection".into(),
            "actor" => {
                intent.by_participant = Some(f.pids[1].clone());
                intent.payload["by_participant"] = json!(f.pids[1]);
            }
            "device" => wrong.device.id = "another-device".into(),
            _ => unreachable!(),
        }
        assert!(admit(&f, &wrong).is_err(), "{field}");
    }
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "source-leave".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(3),
                    action: Action::Leave {
                        participant_id: f.pids[0].clone(),
                    },
                },
            },
        )
        .unwrap();
    assert!(admit(&f, &request).is_err());
}

fn operation(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "select-audio".into(),
            },
        )
        .unwrap()
}
fn cancel(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "cancel-selection".into(),
                target_operation_id: "select-audio".into(),
                reason: Some("fixture cancellation".into()),
            },
        )
        .unwrap()
}
#[test]
fn native_stream_identity_retains_extensions_and_only_one_attempt_and_open_control() {
    use connect_protocol::transitions::OperationState;
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    let before = f.count("operations");
    let anchor = f
        .store
        .conversation_events(&f.keys[0], &f.cid, Default::default())
        .unwrap()
        .cursor;
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    let replay = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(accepted, replay);
    assert_eq!(f.count("operations"), before + 1);
    let mut changed = request.clone();
    let PeerCommand::NativeStream(wire) = &mut changed.command else {
        panic!()
    };
    wire.payload["future_extension"] = json!({"revision":2});
    let different = admit(&f, &changed).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &different)
            .unwrap_err()
            .code,
        "conflict"
    );
    let PeerCommand::NativeStream(wire) = &mut changed.command else {
        panic!()
    };
    wire.operation_id = "competing-selection".into();
    let competing = admit(&f, &changed).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &competing)
            .unwrap_err()
            .code,
        "conflict"
    );
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(started.state, OperationState::Preparing);
    assert_eq!(started.effects.len(), 1);
    assert_eq!(started.effects[0].id, lease.effect_id());
    let snapshot = f
        .store
        .conversation_snapshot(&f.keys[0], &f.cid, Default::default())
        .unwrap();
    assert!(snapshot.items.iter().any(|item| matches!(item,connect_protocol::events::SnapshotItem::Operation(record) if record.record_id == started.record_id && record.state == OperationState::Preparing)));
    let events = f
        .store
        .conversation_events(
            &f.keys[0],
            &f.cid,
            connect_protocol::events::EventQuery {
                after: Some(anchor),
                ..Default::default()
            },
        )
        .unwrap();
    let states = events
        .events
        .iter()
        .filter_map(|event| match &event.data {
            connect_protocol::events::JournalData::Operation {
                operation_id,
                state,
                ..
            } if operation_id == "select-audio" => Some(*state),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![OperationState::Accepted, OperationState::Preparing]
    );
    f.store
        .validate_native_stream_attempt(&f.keys[0], &proof, &lease)
        .unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        started
    );
    assert_eq!(f.count("operation_effects"), 2); // original parent + exactly one control
    let c = f.store.connection().unwrap();
    for sql in [
        "UPDATE native_stream_operations SET generation=generation+1",
        "UPDATE native_stream_operations SET effect_started=0",
        "DELETE FROM native_stream_operations",
    ] {
        assert!(c.execute(sql, []).is_err());
    }
}
#[test]
fn native_stream_cancellation_distinguishes_no_attempt_and_uncertain_attempt() {
    use connect_protocol::transitions::OperationState;
    for started in [false, true] {
        let (f, request, _) = parent(true, true);
        let proof = admit(&f, &request).unwrap();
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let lease = if started {
            Some(
                f.store
                    .begin_native_stream_with_peer(&f.keys[0], &proof)
                    .unwrap()
                    .unwrap()
                    .0,
            )
        } else {
            None
        };
        let decision = cancel(&f);
        assert_eq!(cancel(&f), decision);
        let status = operation(&f);
        assert_eq!(
            status.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Cancelled
            }
        );
        assert!(status.cancel_requested);
        assert_eq!(status.effects.len(), usize::from(started));
        if let Some(lease) = lease {
            assert!(f
                .store
                .validate_native_stream_attempt(&f.keys[0], &proof, &lease)
                .is_err());
        }
        assert!(f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        let mut next = request.clone();
        let PeerCommand::NativeStream(wire) = &mut next.command else {
            panic!()
        };
        wire.operation_id = "next-selection".into();
        let next_proof = admit(&f, &next).unwrap();
        assert_eq!(
            f.store
                .accept_native_stream_with_peer(&f.keys[0], &next_proof)
                .is_err(),
            started
        );
    }
}
#[test]
fn native_stream_stale_revision_and_preflight_refusal_are_retained_without_effects() {
    use connect_protocol::transitions::OperationState;
    for revision in [0, 1, 2] {
        let (f, mut request, _) = parent(true, true);
        let stale = revision != 1;
        if stale {
            let PeerCommand::NativeStream(wire) = &mut request.command else {
                panic!()
            };
            wire.expected_revision = Some(revision);
        }
        let proof = admit(&f, &request).unwrap();
        let accepted = f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        if stale {
            assert_eq!(accepted.state, OperationState::Failed);
        } else {
            let refused = f
                .store
                .refuse_native_stream_before_effect_with_peer(
                    &f.keys[0],
                    &proof,
                    connect_protocol::Error::new(
                        "unsupported-media-profile",
                        "No negotiated route for this selector",
                    ),
                )
                .unwrap();
            assert_eq!(refused.state, OperationState::Failed);
        }
        let status = operation(&f);
        assert_eq!(status.state, OperationState::Failed);
        assert!(status.effects.is_empty());
        assert_eq!(
            f.store
                .accept_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap(),
            status
        );
        assert!(f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        assert_eq!(f.count("operation_effects"), 1);
    }
}
#[test]
fn native_stream_status_survives_parent_retirement_but_live_execution_does_not() {
    use connect_protocol::transitions::OperationState;
    let (f, request, parent) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    drop(proof);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
            [&parent],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='retired',generation=generation+1 WHERE id='connection-native'",[]).unwrap();
    }
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        accepted
    );
    assert_eq!(operation(&f).state, OperationState::Accepted);
    assert!(f
        .store
        .project_native_stream_with_peer(&f.keys[0], &proof)
        .is_err());
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
    cancel(&f);
    assert_eq!(operation(&f).state, OperationState::Cancelled);
}
#[test]
fn native_stream_journal_and_receipt_reservations_protect_cancellation_at_capacity() {
    use connect_protocol::transitions::OperationState;
    for started in [false, true] {
        let (mut f, request, _) = parent(true, true);
        let proof = admit(&f, &request).unwrap();
        let operations = f.count("operations");
        let journal = f.count("journal");
        // Existing completed parent keeps its independent five-event cleanup
        // reservation. This command reserves six facts, including acceptance.
        f.store.limits.operation_receipts = operations + 2;
        f.store.limits.journal_events = journal + 5 + 6;
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        if started {
            f.store
                .begin_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap()
                .unwrap();
        }
        let denied = Operation::Send {
            request_id: "reserve-intruder".into(),
            conversation_id: f.cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all("reserve check"),
        };
        assert_eq!(
            f.store.execute(&f.keys[0], &denied).unwrap_err().code,
            "capacity-exceeded"
        );
        cancel(&f);
        assert_eq!(f.count("operations"), operations + 2);
        assert_eq!(
            operation(&f).state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Cancelled
            }
        );
    }
}

#[test]
fn native_stream_expired_attempt_becomes_unknown_and_keeps_its_effect_identity() {
    use connect_protocol::transitions::OperationState;
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET lease_expires_at=?2 WHERE record_id=?1",
            params![lease.record_id(), crate::now() - 1],
        )
        .unwrap();
    assert_eq!(f.store.recover_native_streams(1).unwrap(), 1);
    assert_eq!(f.store.recover_native_streams(1).unwrap(), 0);
    let status = operation(&f);
    assert_eq!(status.state, OperationState::Unknown);
    assert_eq!(status.expires_at, started.expires_at);
    assert_eq!(status.effects[0].id, lease.effect_id());
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .validate_native_stream_attempt(&f.keys[0], &proof, &lease)
        .is_err());
    assert_eq!(
        f.store.recover_native_streams(0).unwrap_err().code,
        "invalid-request"
    );
}
#[test]
fn native_stream_successor_preserves_unknown_attempt_without_resubmission() {
    use connect_protocol::transitions::{OperationAction, OperationState};
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let epoch = f.store.runtime_epoch().to_owned();
    let path = f.store.path.clone();
    let credential = f.keys[0].clone();
    drop(proof);
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_ne!(successor.runtime_epoch(), epoch);
    assert_eq!(successor.recover_native_streams(256).unwrap(), 1);
    let status = successor
        .execute_operation_action(
            &credential,
            OperationAction::Get {
                operation_id: "select-audio".into(),
            },
        )
        .unwrap();
    assert_eq!(status.state, OperationState::Unknown);
    assert_eq!(status.record_id, started.record_id);
    assert_eq!(status.expires_at, started.expires_at);
    assert_eq!(status.effects[0].id, lease.effect_id());
    let proof = successor
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        successor
            .accept_native_stream_with_peer(&credential, &proof)
            .unwrap(),
        status
    );
    assert!(successor
        .begin_native_stream_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
    assert!(successor
        .validate_native_stream_attempt(&credential, &proof, &lease)
        .is_err());
}

#[test]
fn native_stream_recovery_key_can_read_but_cannot_execute_or_refuse_another_original_source() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    drop(proof);
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
    let recovery = f
        .store
        .issue(&actor.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id;
    let (public, not_before, expires_at): (Vec<u8>, i64, i64) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT public_key,not_before,expires_at FROM peer_signing_keys WHERE key_id=?1",
            [&request.key_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-recovery-key".into(),
                identity_id: actor.identity_id,
                credential_id: recovery.clone(),
                device_id: request.device.id.clone(),
                key_id: request.key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before,
                expires_at,
                expected_revision: Some(0),
            },
        )
        .unwrap();
    let mut alternate = request.clone();
    alternate.credentials = vec![recovery.clone()];
    let proof = admit(&f, &alternate).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&recovery, &proof)
            .unwrap(),
        accepted
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&recovery, &proof)
        .is_err());
    assert!(f
        .store
        .refuse_native_stream_before_effect_with_peer(
            &recovery,
            &proof,
            connect_protocol::Error::new(
                "unsupported-media-profile",
                "alternate preflight cannot reject original"
            )
        )
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
}
#[test]
fn native_stream_racing_attempts_and_failed_journal_commit_do_not_duplicate_an_effect() {
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_stream_attempt_fact BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' BEGIN SELECT RAISE(ABORT,'isolated attempt fact failure'); END;").unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .is_err());
    assert_eq!(f.count("operation_effects"), 1);
    assert_eq!(
        operation(&f).state,
        connect_protocol::transitions::OperationState::Accepted
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_stream_attempt_fact")
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|s| {
        let one = s.spawn(|| {
            barrier.wait();
            f.store
                .begin_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap()
        });
        let two = s.spawn(|| {
            barrier.wait();
            f.store
                .begin_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap()
        });
        let results = [one.join().unwrap(), two.join().unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_some()).count(), 1);
    });
    assert_eq!(f.count("operation_effects"), 2);
}

#[test]
fn captured_native_stream_outcome_records_once_after_cancel_and_never_revives_a_retired_parent() {
    // Controlled ledger outcomes exercise the trusted host recording boundary;
    // production server tests alone prove an actual native engine effect.
    use connect_protocol::transitions::OperationState;
    for retired in [false, true] {
        let (f, request, parent) = parent(true, true);
        let proof = admit(&f, &request).unwrap();
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, _) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        cancel(&f);
        assert_eq!(operation(&f).state, OperationState::Unknown);
        if retired {
            let c = f.store.connection().unwrap();
            c.execute(
                "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
                [&parent],
            )
            .unwrap();
            c.execute("UPDATE connections SET state='retired',generation=generation+1 WHERE id='connection-native'",[]).unwrap();
        }
        let receipt = lease.committed(2, true).unwrap();
        f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_stream_completion BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' BEGIN SELECT RAISE(ABORT,'isolated completion rollback'); END;").unwrap();
        assert!(f.store.complete_native_stream(&receipt).is_err());
        assert_eq!(operation(&f).state, OperationState::Unknown);
        f.store
            .connection()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_stream_completion")
            .unwrap();
        let complete = f.store.complete_native_stream(&receipt).unwrap();
        assert_eq!(complete.state, OperationState::Completed);
        assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
        assert!(complete.cancel_requested);
        let c = f.store.connection().unwrap();
        let (state,revision,stream):(String,u64,u64)=c.query_row("SELECT state,revision,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(revision, 1); // Stream revision is distinct from SQL Connection revision.
        assert_eq!(stream, if retired { 1 } else { 2 });
        assert_eq!(state, if retired { "retired" } else { "ready" });
    }
}

#[test]
fn owned_job_can_prove_it_stopped_before_entering_engine_without_retiring_the_parent() {
    use connect_protocol::transitions::{EffectState, OperationState};
    let (f, request, _) = parent(true, true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let failed = f
        .store
        .fail_native_stream_without_effect(
            lease,
            connect_protocol::Error::new(
                "authority-busy",
                "Current source unavailable before engine entry",
            ),
        )
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    assert_eq!(failed.effects[0].state, EffectState::Failed);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM connections WHERE id='connection-native'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "ready"
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
}

#[test]
fn additive_upgrade_requires_settled_current_native_slots_and_preserves_their_bindings() {
    for completed in [false, true] {
        let (f, _, parent) = parent(true, completed);
        let before=f.store.connection().unwrap().query_row("SELECT generation,revision,state,readiness_json FROM connections WHERE id='connection-native'",[],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?))).unwrap();
        let operation = Operation::SessionControl {
            request: Request {
                operation_id: "native-live-upgrade".into(),
                conversation_id: f.cid.clone(),
                session_id: "session".into(),
                expected_revision: Some(3),
                by_participant: None,
                action: Action::Upgrade {
                    media: vec!["video".into()],
                },
            },
        };
        let result = f.store.execute(&f.keys[0], &operation);
        assert_eq!(result.is_ok(), completed);
        let after=f.store.connection().unwrap().query_row("SELECT generation,revision,state,readiness_json FROM connections WHERE id='connection-native'",[],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?))).unwrap();
        assert_eq!(before, after);
        assert_eq!(f.count("native_offer_operations"), 1);
        assert_eq!(f.count("operation_effects"), 1);
        assert_eq!(
            parent,
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT operation_record_id FROM native_offer_operations",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap()
        );
        if completed {
            assert_eq!(
                f.store.execute(&f.keys[0], &operation).unwrap(),
                result.unwrap()
            );
            assert_eq!(
                f.store
                    .session_view(&f.keys[0], &f.cid, "session")
                    .unwrap()
                    .media,
                vec!["audio", "video"]
            );
        }
    }
}

fn end_request(granted: bool) -> (Fixture, PeerAuthorityRequest, String) {
    let (f, mut request, parent) = parent_scoped(true, true, granted);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::End;
    wire.expected_revision = None;
    wire.payload = json!({"reason_code":1000,"reason":"done","future":true});
    (f, request, parent)
}
#[test]
fn native_connection_end_requires_its_own_named_grant_and_retains_exact_optional_revision() {
    let (ungranted, request, _) = end_request(false);
    assert!(admit(&ungranted, &request).is_err());
    for revision in [None, Some(1), Some(2)] {
        let (f, mut request, _) = end_request(true);
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.expected_revision = revision;
        let proof = admit(&f, &request).unwrap();
        let accepted = f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        assert_eq!(
            accepted.state,
            if revision == Some(2) {
                connect_protocol::transitions::OperationState::Failed
            } else {
                connect_protocol::transitions::OperationState::Accepted
            }
        );
        assert_eq!(accepted.expected_revision, revision);
        assert_eq!(f.count("operation_effects"), 1);
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.expected_revision = if revision.is_none() { Some(1) } else { None };
        let changed = admit(&f, &request).unwrap();
        assert_eq!(
            f.store
                .accept_native_stream_with_peer(&f.keys[0], &changed)
                .unwrap_err()
                .code,
            "conflict"
        );
    }
}
struct EndObservation {
    id: String,
    generation: u64,
}
impl crate::NativeOfferRuntimeObservation for EndObservation {
    fn observe(&self) -> crate::Result<crate::NativeOfferObservation> {
        Ok(crate::NativeOfferObservation::Retired {
            runtime_connection_id: self.id.clone(),
            generation: self.generation,
        })
    }
}
#[test]
fn captured_native_connection_end_records_after_storage_failure_and_cancel_without_advancing_stream_revision(
) {
    // Controlled host observation tests receipt custody, not real engine IO.
    let (f, request, parent) = end_request(true);
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let observation = EndObservation {
        id: lease.runtime_connection_id().into(),
        generation: lease.generation(),
    };
    let receipt = lease.ended(&observation).unwrap();
    cancel(&f);
    assert_eq!(
        operation(&f).state,
        connect_protocol::transitions::OperationState::Unknown
    );
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
        [&parent],
    )
    .unwrap();
    c.execute("UPDATE connections SET state='retired',generation=generation+1 WHERE id='connection-native'",[]).unwrap();
    c.execute_batch("CREATE TRIGGER fail_end_completion BEFORE INSERT ON journal WHEN NEW.kind='operation_updated' BEGIN SELECT RAISE(ABORT,'isolated end recording failure'); END;").unwrap();
    drop(c);
    assert!(f.store.complete_native_stream(&receipt).is_err());
    assert_eq!(
        operation(&f).state,
        connect_protocol::transitions::OperationState::Unknown
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_end_completion")
        .unwrap();
    let completed = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(
        completed.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(
        completed.result,
        Some(json!({"connection_id":"connection-native","ended":true}))
    );
    assert_eq!(completed.effects[0].id, started.effects[0].id);
    assert!(completed.cancel_requested);
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), completed);
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap(),
        completed
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    let c = f.store.connection().unwrap();
    let (state, stream):(String,u64)=c.query_row("SELECT state,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id='connection-native'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((state.as_str(), stream), ("retired", 1));
}
#[test]
fn native_connection_end_cannot_use_subscription_success_or_another_generation_absence() {
    for wrong in [0, 1, 2] {
        let (f, request, _) = end_request(true);
        let proof = admit(&f, &request).unwrap();
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap();
        let (lease, _) = f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .unwrap();
        let observation = EndObservation {
            id: if wrong == 1 {
                "another-route".into()
            } else {
                lease.runtime_connection_id().into()
            },
            generation: lease.generation() + u64::from(wrong == 2),
        };
        assert!(if wrong == 0 {
            lease.committed(2, true)
        } else {
            lease.ended(&observation)
        }
        .is_err());
        assert_eq!(
            operation(&f).state,
            connect_protocol::transitions::OperationState::Preparing
        );
    }
}

fn data_request(completed: bool) -> (Fixture, PeerAuthorityRequest, String) {
    data_request_with_send(completed, true)
}
fn data_request_with_send(completed: bool, send: bool) -> (Fixture, PeerAuthorityRequest, String) {
    // Data publication uses the separate send grant; no stream.subscribe grant
    // is needed to publish one's own already accepted data source.
    let (f, mut request, parent) = parent_permissions(false, completed, false, true, send);
    let PeerCommand::NativeStream(intent) = &mut request.command else {
        panic!()
    };
    intent.operation_id = "publish-application".into();
    intent.expected_revision = None;
    intent.by_participant = Some(f.pids[0].clone());
    intent.command = Command::Data;
    intent.payload = json!({"from":f.pids[0],"msg_id":"application-message","label":"application","to":[f.pids[1]],"content_type":"application/json","body":"{\"hello\":true}","future":{"keep":true}});
    (f, request, parent)
}
#[test]
fn native_data_source_projection_owns_current_parent_and_exact_message_without_effect_or_receipt() {
    let (f, request, parent) = data_request(true);
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let proof = admit(&f, &request).unwrap();
    let projection = f
        .store
        .project_native_data_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(projection.source.parent_record_id, parent);
    assert_eq!(projection.message.message_id, "application-message");
    assert_eq!(projection.message.bytes, b"{\"hello\":true}");
    assert_eq!(projection.source_protocols, vec!["json", "binary"]);
    assert_eq!(projection.recipients.len(), 1);
    assert_eq!(
        (
            projection.recipients[0].participant_id.as_str(),
            projection.recipients[0].membership_epoch
        ),
        (f.pids[1].as_str(), 1)
    );
    assert_eq!(proof.native_member(), Some((f.pids[0].as_str(), 1)));
    assert!(!format!("{projection:?}").contains("hello"));

    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
}
#[test]
fn native_data_source_cannot_borrow_binding_authority_or_an_unready_native_parent() {
    let (f, mut request, _) = data_request(false);
    assert!(admit(&f, &request).is_err());
    request.command = PeerCommand::Binding;
    let proof = admit(&f, &request).unwrap();
    assert!(f
        .store
        .project_native_data_with_peer(&f.keys[0], &proof)
        .is_err());
}
#[test]
fn native_data_source_rechecks_message_label_actor_audience_and_revision() {
    let (f, request, _) = data_request(true);
    for field in [
        "label",
        "from",
        "to",
        "revision",
        "connection",
        "key",
        "device",
    ] {
        let mut invalid = request.clone();
        let PeerCommand::NativeStream(intent) = &mut invalid.command else {
            panic!()
        };
        match field {
            "label" => intent.payload["label"] = json!("not-observed"),
            "from" => {
                intent.by_participant = Some(f.pids[1].clone());
                intent.payload["from"] = json!(f.pids[1]);
            }
            "to" => intent.payload["to"] = json!(["another-conversation-participant"]),
            "revision" => intent.expected_revision = Some(2),
            "connection" => intent.connection_id = "another-native-source".into(),
            "key" => invalid.key_id = "another-key".into(),
            _ => invalid.device.id = "another-device".into(),
        }
        assert!(admit(&f, &invalid).is_err(), "{field}");
    }
}
#[test]
fn native_data_source_rechecks_every_profile_floor_and_observed_label_before_no_effect_planning() {
    for floor in 0..5 {
        let (f, request, _) = data_request(true);
        let proof = admit(&f, &request).unwrap();
        {
            let c = f.store.connection().unwrap();
            match floor {
                0=>c.execute(r#"UPDATE sessions SET media='["audio"]' WHERE id='session'"#,[]),
                1=>c.execute("UPDATE endpoints SET verification_state='revoked' WHERE id=?1",[&f.endpoints[0]]),
                2=>c.execute("UPDATE session_presence_capabilities SET capabilities_json='{}' WHERE session_id='session' AND participant_id=?1",[&f.pids[0]]),
                3=>c.execute("UPDATE session_control SET capabilities_json='{}' WHERE session_id='session'",[]),
                _=>c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.streams',json('[]')) WHERE id='connection-native'",[]),
            }.unwrap();
        }
        assert!(
            f.store
                .project_native_data_with_peer(&f.keys[0], &proof)
                .is_err(),
            "{floor}"
        );
        assert_eq!(f.count("operation_effects"), 1);
    }
}
#[test]
fn native_data_source_all_audience_is_current_present_epochs_and_refuses_a_left_explicit_recipient()
{
    let (f, mut request, _) = data_request(true);
    let PeerCommand::NativeStream(intent) = &mut request.command else {
        panic!()
    };
    intent.payload["to"] = json!("all");
    let proof = admit(&f, &request).unwrap();
    let all = f
        .store
        .project_native_data_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(all.recipients.len(), 4);
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE session_participants SET state='left' WHERE session_id='session' AND participant_id=?1",[&f.pids[1]]).unwrap();
    }
    assert_eq!(
        f.store
            .project_native_data_with_peer(&f.keys[0], &proof)
            .unwrap()
            .recipients
            .len(),
        3
    );
    let PeerCommand::NativeStream(intent) = &mut request.command else {
        panic!()
    };
    intent.payload["to"] = json!([f.pids[1]]);
    assert!(admit(&f, &request).is_err());
}

#[test]
fn native_data_source_creation_permission_does_not_grant_message_publication_permission() {
    let (f, request, _) = data_request_with_send(true, false);
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    assert_eq!(admit(&f, &request).err().unwrap().code, "forbidden");
    assert_eq!(
        before,
        (
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
}

mod capabilities;
mod codecs;
mod data_custody;
mod data_reads;
mod data_writes;
mod gates;
mod quality;
mod reconciliation;
mod selector_sources;

mod dtmf;
mod dtmf_recipients;

mod credential_refresh;

mod session_codecs;
