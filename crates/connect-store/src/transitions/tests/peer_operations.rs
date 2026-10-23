//! Installed operation proof, shared ledger and SQL admission failure boundaries.
use super::*;
use crate::{
    AuthorityMode, AuthorityScope, AuthorityWait, PeerAuthorityRequest, PeerCommand,
    PeerDataCommand, PeerDeviceClaim, PeerOperationClaim,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::provisioning::{ProvisioningCommand, SigningAlgorithm};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::time::{Duration, Instant};
fn request(f: &Fixture, action: OperationAction) -> PeerAuthorityRequest {
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.keys[0],
            ProvisioningCommand::SigningKey {
                operation_id: "peer-enroll".into(),
                identity_id: f.ids[0].clone(),
                credential_id: f.keys[0].clone(),
                device_id: "dev-0".into(),
                key_id: "peer-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 300,
                expected_revision: None,
            },
        )
        .unwrap();
    PeerAuthorityRequest {
        credentials: vec![f.keys[0].clone()],
        device: PeerDeviceClaim {
            id: "dev-0".into(),
            kind: "fixture".into(),
            platform: "fixture".into(),
        },
        key_id: "peer-key".into(),
        algorithm: "EdDSA".into(),
        command: PeerCommand::Operation(PeerOperationClaim {
            action,
            conversation_id: Some(f.cid.clone()),
            session_id: Some(f.sid.clone()),
            connection_id: None,
        }),
    }
}
fn admit(f: &Fixture, request: &PeerAuthorityRequest) -> crate::PeerAuthorityGuard {
    f.store
        .admit_peer_authority(request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap()
}
fn get(id: &str) -> OperationAction {
    OperationAction::Get {
        operation_id: id.into(),
    }
}

#[test]
fn http_transition_command_replay_preserves_expiry_and_checks_operation_resource_claims() {
    let f = Fixture::new();
    let r = f.request("http-command");
    let command = TransitionCommand {
        payload_extensions: Default::default(),
        operation_id: r.operation_id,
        conversation_id: r.conversation_id,
        session_id: r.session_id,
        connection_id: r.connection_id,
        expected_revision: r.expected_revision,
        intent: r.intent,
    };
    let first = f
        .store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        f.store
            .admit_transition_command(&f.keys[0], command)
            .unwrap()
            .expires_at,
        first.expires_at
    );
    let claim = PeerOperationClaim {
        action: get("http-command"),
        conversation_id: Some(f.cid.clone()),
        session_id: Some(f.sid.clone()),
        connection_id: None,
    };
    assert_eq!(
        f.store
            .execute_operation_action_claim(&f.keys[0], claim.clone())
            .unwrap()
            .record_id,
        first.record_id
    );
    for field in 0..3 {
        let mut bad = claim.clone();
        match field {
            0 => bad.conversation_id = Some("foreign".into()),
            1 => bad.session_id = Some("foreign".into()),
            _ => bad.connection_id = Some("conn-bob".into()),
        };
        assert_eq!(
            f.store
                .execute_operation_action_claim(&f.keys[0], bad)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
}
#[test]
fn canonical_operation_proof_covers_real_target_sources_and_refuses_spoofed_resource_assertions() {
    let f = Fixture::new();
    let row = f.admit("target");
    let request = request(&f, get("target"));
    let proof = admit(&f, &request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[
            AuthorityScope::identity(&f.ids[0], AuthorityMode::Write).unwrap(),
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap(),
            AuthorityScope::identity(&f.ids[1], AuthorityMode::Read).unwrap()
        ]
    ));
    assert_eq!(
        f.store
            .execute_operation_action_with_peer(&f.keys[0], get("target"), &proof)
            .unwrap()
            .record_id,
        row.record_id
    );
    drop(proof);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for field in 0..3 {
        let mut bad = request.clone();
        let PeerCommand::Operation(claim) = &mut bad.command else {
            panic!()
        };
        match field {
            0 => claim.conversation_id = Some("foreign".into()),
            1 => claim.session_id = Some("foreign".into()),
            _ => claim.connection_id = Some("conn-bob".into()),
        }
        assert!(f
            .store
            .admit_peer_authority(&bad, &AuthorityWait::new(Duration::from_millis(10)))
            .is_err());
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn canonical_operation_proof_is_not_substitutable_for_another_action_target_credential_or_root() {
    let f = Fixture::new();
    f.admit("target");
    let request = request(&f, get("target"));
    let proof = admit(&f, &request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let cancel = OperationAction::Cancel {
        operation_id: "cancel".into(),
        target_operation_id: "target".into(),
        reason: None,
    };
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], cancel, &proof)
        .is_err());
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[1], get("target"), &proof)
        .is_err());
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("other"), &proof)
        .is_err());
    let other = Fixture::new();
    other.admit("target");
    assert!(other
        .store
        .execute_operation_action_with_peer(&other.keys[0], get("target"), &proof)
        .is_err());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(proof);
    let mut data = request;
    data.command = PeerCommand::Data {
        conversation_id: f.cid.clone(),
        command: PeerDataCommand::MessageHistory,
    };
    let read = admit(&f, &data);
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("target"), &read)
        .is_err());
}
#[test]
fn canonical_operation_cancel_replay_and_attributed_reconciliation_share_current_durable_receipts()
{
    let f = Fixture::new();
    f.admit("target");
    let action = OperationAction::Cancel {
        operation_id: "cancel".into(),
        target_operation_id: "target".into(),
        reason: Some("stop".into()),
    };
    let mut request = request(&f, action.clone());
    let proof = admit(&f, &request);
    let receipt = f
        .store
        .execute_operation_action_with_peer(&f.keys[0], action.clone(), &proof)
        .unwrap();
    assert_eq!(
        f.store
            .execute_operation_action_with_peer(&f.keys[0], action.clone(), &proof)
            .unwrap(),
        receipt
    );
    drop(proof);
    assert_eq!(f.get("target").state, OperationState::Cancelled);
    let evidence = f
        .store
        .record_transition_evidence(
            &f.keys[0],
            "target",
            "cancelled-fact",
            EvidenceSource::OperatorAttestation,
            ReconcileOutcome::Cancelled,
            None,
        )
        .unwrap();
    let reconcile = OperationAction::Reconcile {
        operation_id: "reconcile".into(),
        target_operation_id: "target".into(),
        outcome: ReconcileOutcome::Cancelled,
        evidence_id: evidence,
    };
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.action = reconcile.clone();
    let proof = admit(&f, &request);
    let decision = f
        .store
        .execute_operation_action_with_peer(&f.keys[0], reconcile.clone(), &proof)
        .unwrap();
    assert_eq!(decision.state, OperationState::Completed);
    assert_eq!(
        f.store
            .execute_operation_action_with_peer(&f.keys[0], reconcile, &proof)
            .unwrap(),
        decision
    );
}
#[test]
fn canonical_operation_held_get_does_not_reenter_behind_queued_key_retirement() {
    let f = Fixture::new();
    f.admit("target");
    let request = request(&f, get("target"));
    let proof = admit(&f, &request);
    let peer = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    let admin = f.keys[0].clone();
    let writer = std::thread::spawn(move || {
        peer.execute_provisioning(
            &admin,
            ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire".into(),
                key_id: "peer-key".into(),
                expected_revision: 0,
            },
        )
    });
    crate::authority_fences::test_wait_for_writer(
        &f.store,
        &AuthorityScope::identity(&f.ids[0], AuthorityMode::Write).unwrap(),
    );
    let start = Instant::now();
    assert_eq!(
        f.store
            .execute_operation_action_with_peer(&f.keys[0], get("target"), &proof)
            .unwrap()
            .state,
        OperationState::Accepted
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(proof);
    writer.join().unwrap().unwrap();
    assert!(f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(10)))
        .is_err());
}
#[test]
fn canonical_operation_source_expiry_during_sql_wait_rolls_back_lazy_recovery_and_decision() {
    let f = Fixture::new();
    f.admit("target");
    let lease = f.claim("target");
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET lease_expires_at=0 WHERE operation_id='target'",
            [],
        )
        .unwrap();
    let mut request = request(&f, get("target"));
    for is_cancel in [false, true] {
        let action = if is_cancel {
            OperationAction::Cancel {
                operation_id: "cancel".into(),
                target_operation_id: "target".into(),
                reason: None,
            }
        } else {
            get("target")
        };
        let PeerCommand::Operation(claim) = &mut request.command else {
            panic!()
        };
        claim.action = action.clone();
        let mut proof = admit(&f, &request);
        proof.limit_expiry(crate::now() + 4).unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let connection = f.store.connection().unwrap();
        let store = f.store.clone();
        let actor = f.keys[0].clone();
        let expiry = proof.expires_at();
        let (entered, observed) = std::sync::mpsc::channel();
        let pending = std::thread::spawn(move || {
            entered.send(()).unwrap();
            store.execute_operation_action_with_peer(&actor, action, &proof)
        });
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            crate::now() < expiry,
            "source must still be live while SQL is held"
        );
        while crate::now() < expiry {
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(connection);
        assert!(pending.join().unwrap().is_err());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    drop(lease);
}
#[test]
fn canonical_operation_get_retains_pending_invitation_projection_identity_until_delivery() {
    use connect_protocol::membership::{MembershipAction, MembershipRequest};
    let f = Fixture::new();
    let outsider = f.store.create_identity("outside").unwrap().id;
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "invite-outside".into(),
                conversation_id: f.cid.clone(),
                expected_revision: 0,
                change: MembershipAction::Invite {
                    identity_id: outsider.clone(),
                    kind: connect_protocol::provisioning::IdentityKind::Human,
                    role: "participant".into(),
                    expires_at: crate::now() + 300,
                },
            },
        )
        .unwrap();
    let mut request = request(&f, get("invite-outside"));
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.session_id = None;
    let proof = admit(&f, &request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[AuthorityScope::identity(&outsider, AuthorityMode::Read).unwrap()]
    ));
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("invite-outside"), &proof)
        .is_ok());
    assert_eq!(
        f.store
            .acquire_authority(
                &[AuthorityScope::identity(&outsider, AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_millis(15))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
}

#[test]
fn canonical_operation_lazy_get_recovers_possible_submission_once_without_reclaim_or_resubmit() {
    let f = Fixture::new();
    f.admit("target");
    let lease = f.claim("target");
    let effect = f.effect(&lease, "possible-creation");
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET lease_expires_at=0 WHERE operation_id='target'",
            [],
        )
        .unwrap();
    let request = request(&f, get("target"));
    let proof = admit(&f, &request);
    let recovered = f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("target"), &proof)
        .unwrap();
    assert_eq!(recovered.state, OperationState::Unknown);
    assert_eq!(recovered.effects[0].id, effect.id);
    assert_eq!(recovered.effects[0].attempt, 1);
    assert_eq!(recovered.effects[0].state, EffectState::Unknown);
    assert_eq!(
        f.store
            .execute_operation_action_with_peer(&f.keys[0], get("target"), &proof)
            .unwrap(),
        recovered
    );
    drop(proof);
    assert!(f
        .store
        .claim_transition(&f.keys[0], "target", 60)
        .unwrap()
        .is_none());
}
#[test]
fn canonical_operation_decisions_reject_nested_mutations_and_bound_malformed_source_depth_and_cycles(
) {
    let f = Fixture::new();
    f.admit("target");
    let first = f.cancel("decision", "target");
    assert_eq!(
        f.store
            .execute_operation_action(
                &f.keys[0],
                OperationAction::Cancel {
                    operation_id: "nested".into(),
                    target_operation_id: "decision".into(),
                    reason: None
                }
            )
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut request = request(&f, get("decision"));
    let proof = admit(&f, &request);
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("decision"), &proof)
        .is_ok());
    drop(proof);
    // Explicit malformed persistent-state injection, not an alleged public
    // creation path: a deep/cyclic decision graph must fail before recursive
    // provenance traversal. Public mutation above already rejects such nesting.
    let mut previous = first.record_id.clone();
    {
        let c = f.store.connection().unwrap();
        for n in 0..32 {
            let record = format!("malformed-record-{n}");
            let operation = format!("malformed-{n}");
            c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,intent_json,state,result_json,created_at,updated_at) SELECT ?1,tenant_id,owner_id,?2,digest,command_type,conversation_id,session_id,intent_json,state,result_json,created_at,updated_at FROM operations WHERE record_id=?3",params![record,operation,first.record_id]).unwrap();
            c.execute(
                "INSERT INTO operation_decisions VALUES(?1,?2,'cancel')",
                params![record, previous],
            )
            .unwrap();
            previous = record;
        }
    }
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.action = get("malformed-31");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(15)))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store.connection().unwrap().execute("UPDATE operation_decisions SET target_record_id=operation_record_id WHERE operation_record_id=?1",[&previous]).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(15)))
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operation_decisions SET action='reconcile' WHERE operation_record_id=?1",
            [&previous],
        )
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(15)))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    let target_record = f.get("target").record_id;
    f.store
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO operation_decisions VALUES(?1,?2,'cancel')",
            params![target_record, first.record_id],
        )
        .unwrap();
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.action = get("target");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(15)))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn canonical_operation_get_keeps_nonconversation_provisioning_projection_source_current() {
    let f = Fixture::new();
    let outcome = f
        .store
        .execute_provisioning(
            &f.keys[0],
            ProvisioningCommand::Identity {
                operation_id: "new-identity".into(),
                alias: "new-identity".into(),
                kind: connect_protocol::provisioning::IdentityKind::Human,
                display_name: None,
            },
        )
        .unwrap();
    let connect_protocol::provisioning::ProvisioningOutcome::Identity { identity_id, .. } = outcome
    else {
        panic!()
    };
    let mut request = request(&f, get("new-identity"));
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.conversation_id = None;
    claim.session_id = None;
    let proof = admit(&f, &request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[AuthorityScope::identity(&identity_id, AuthorityMode::Read).unwrap()]
    ));
    assert!(f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("new-identity"), &proof)
        .is_ok());
    assert_eq!(
        f.store
            .acquire_authority(
                &[AuthorityScope::identity(&identity_id, AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_millis(15))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
}
#[test]
fn canonical_operation_get_requires_both_recovery_permission_and_current_original_command_scope() {
    let f = Fixture::new();
    f.admit("target");
    let request = request(&f, get("target"));
    let meta = f.store.credential_metadata(&f.keys[0]).unwrap();
    let key = f
        .store
        .resolve_peer_signing_key(&f.keys[0], &meta.tenant_id, "peer-key", "EdDSA")
        .unwrap();
    for (n, only) in ["operation.get", "session.consult"].into_iter().enumerate() {
        let narrow = f
            .store
            .issue(
                &f.ids[0],
                Scope {
                    operations: vec![only.into()],
                    recipients: vec!["*".into()],
                },
                crate::now() + 300,
            )
            .unwrap();
        f.store
            .execute_provisioning(
                &f.keys[0],
                ProvisioningCommand::SigningKey {
                    operation_id: format!("bind-{n}"),
                    identity_id: f.ids[0].clone(),
                    credential_id: narrow.id.clone(),
                    device_id: "dev-0".into(),
                    key_id: "peer-key".into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(&key.public_key),
                    not_before: key.not_before,
                    expires_at: key.expires_at,
                    expected_revision: Some(n as u64),
                },
            )
            .unwrap();
        let mut denied = request.clone();
        denied.credentials = vec![narrow.id];
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert!(f
            .store
            .admit_peer_authority(&denied, &AuthorityWait::new(Duration::from_millis(15)))
            .is_err());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}

#[test]
fn canonical_message_receipt_proof_retains_typed_scope_and_refuses_extra_session_assertion() {
    let f = Fixture::new();
    let outcome = f
        .store
        .execute(
            &f.keys[0],
            &connect_protocol::Operation::Send {
                request_id: "core-peer-message".into(),
                conversation_id: f.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("receipt authority"),
            },
        )
        .unwrap();
    let mut request = request(&f, get("core-peer-message"));
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.session_id = None;
    let proof = admit(&f, &request);
    let record = f
        .store
        .execute_operation_action_with_peer(&f.keys[0], get("core-peer-message"), &proof)
        .unwrap();
    assert_eq!(record.result, Some(serde_json::to_value(outcome).unwrap()));
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[
            AuthorityScope::identity(&f.ids[1], AuthorityMode::Read).unwrap(),
            AuthorityScope::identity(&f.ids[0], AuthorityMode::Write).unwrap(),
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
        ]
    ));
    drop(proof);
    let PeerCommand::Operation(claim) = &mut request.command else {
        panic!()
    };
    claim.session_id = Some(f.sid.clone());
    assert_eq!(
        f.store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)))
            .unwrap_err()
            .code,
        "forbidden"
    );
}

fn transition_command(
    request: TransitionRequest,
) -> connect_protocol::transitions::TransitionCommand {
    let mut command = connect_protocol::transitions::TransitionCommand {
        payload_extensions: Default::default(),
        operation_id: request.operation_id,
        conversation_id: request.conversation_id,
        session_id: request.session_id,
        connection_id: request.connection_id,
        expected_revision: request.expected_revision,
        intent: request.intent,
    };
    command.normalize().unwrap();
    command
}
#[test]
fn canonical_transition_admission_reuses_exact_proof_and_never_renews_original_expiry() {
    let f = Fixture::new();
    let command = transition_command(f.request("canonical-transition"));
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command.clone());
    let proof = admit(&f, &signed);
    let first = f
        .store
        .admit_transition_with_peer(&f.keys[0], command.clone(), &proof)
        .unwrap();
    assert_eq!(first.state, OperationState::Accepted);
    assert!(first.result.is_none());
    assert!(first.expires_at.unwrap() <= crate::now() + MAX_OPERATION_TTL);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[
            AuthorityScope::identity(&f.ids[0], AuthorityMode::Write).unwrap(),
            AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap(),
            AuthorityScope::identity(&f.ids[1], AuthorityMode::Read).unwrap(),
            AuthorityScope::identity(&f.ids[2], AuthorityMode::Read).unwrap(),
        ]
    ));
    drop(proof);
    std::thread::sleep(Duration::from_millis(1100));
    let proof = admit(&f, &signed);
    let replay = f
        .store
        .admit_transition_with_peer(&f.keys[0], command.clone(), &proof)
        .unwrap();
    assert_eq!(replay, first);
    drop(proof);
    assert_eq!(
        f.store
            .admit_transition(&f.keys[0], command.with_expiry(first.expires_at.unwrap()))
            .unwrap(),
        first
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='canonical-transition'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}
#[test]
fn canonical_transition_proof_binds_every_resource_and_intent_without_fallback() {
    let f = Fixture::new();
    let command = transition_command(f.request("bound-transition"));
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command.clone());
    let proof = admit(&f, &signed);
    for change in 0..5 {
        let mut other = command.clone();
        match change {
            0 => other.operation_id = "different-operation".into(),
            1 => other.conversation_id = "conv_foreign".into(),
            2 => other.session_id = "session_foreign".into(),
            3 => other.expected_revision = 9,
            _ => {
                if let TransitionIntent::Consult { hold_treatment, .. } = &mut other.intent {
                    *hold_treatment = HoldTreatment::ConfiguredMedia;
                }
            }
        }
        assert_eq!(
            f.store
                .admit_transition_with_peer(&f.keys[0], other, &proof)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    drop(proof);
    signed.command = PeerCommand::Binding;
    let binding = admit(&f, &signed);
    assert_eq!(
        f.store
            .admit_transition_with_peer(&f.keys[0], command, &binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE operation_id='bound-transition'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn canonical_transition_source_readers_block_target_writers_but_not_unrelated_identity() {
    let f = Fixture::new();
    let outsider = f
        .store
        .create_identity("unrelated-transition-owner")
        .unwrap();
    let revision = f
        .store
        .membership_snapshot(&f.keys[0], &f.cid)
        .unwrap()
        .revision;
    f.store
        .execute_membership(
            &f.keys[0],
            connect_protocol::membership::MembershipRequest {
                operation_id: "add-unselected-member".into(),
                conversation_id: f.cid.clone(),
                expected_revision: revision,
                change: connect_protocol::membership::MembershipAction::Add {
                    identity_id: outsider.id.clone(),
                    kind: connect_protocol::provisioning::IdentityKind::Human,
                    role: "participant".into(),
                },
            },
        )
        .unwrap();
    let command = transition_command(f.request("transition-readers"));
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command.clone());
    let proof = admit(&f, &signed);
    assert!(!f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[AuthorityScope::identity(&outsider.id, AuthorityMode::Read).unwrap()]
    ));
    f.store
        .admit_transition_with_peer(&f.keys[0], command.clone(), &proof)
        .unwrap();
    for identity in &f.ids[1..] {
        assert_eq!(
            f.store
                .acquire_authority(
                    &[AuthorityScope::identity(identity, AuthorityMode::Write).unwrap()],
                    &AuthorityWait::new(Duration::from_millis(15))
                )
                .unwrap_err()
                .code,
            "authority-busy"
        );
    }
    let unrelated = f
        .store
        .acquire_authority(
            &[AuthorityScope::identity(&outsider.id, AuthorityMode::Write).unwrap()],
            &AuthorityWait::new(Duration::from_millis(100)),
        )
        .unwrap();
    drop(unrelated);
    assert_eq!(
        f.store
            .admit_transition_with_peer(&f.keys[0], command, &proof)
            .unwrap()
            .state,
        OperationState::Accepted
    );
    // Readers remain owned for the caller's delivery lifetime after commit.
    assert_eq!(
        f.store
            .acquire_authority(
                &[AuthorityScope::identity(&f.ids[2], AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_millis(15))
            )
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(proof);
    f.store
        .acquire_authority(
            &[AuthorityScope::identity(&f.ids[2], AuthorityMode::Write).unwrap()],
            &AuthorityWait::new(Duration::from_millis(100)),
        )
        .unwrap();
}
#[test]
fn canonical_transition_replay_uses_existing_http_expiry_without_operation_get_permission() {
    let f = Fixture::new();
    let http = f.request("http-first-transition");
    let first = f.store.admit_transition(&f.keys[0], http.clone()).unwrap();
    let command = transition_command(http);
    let mut signed = request(&f, get("unused"));
    signed.command = PeerCommand::Transition(command.clone());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations='[\"session.consult\"]' WHERE id=?1",
            [&f.keys[0]],
        )
        .unwrap();
    let proof = admit(&f, &signed);
    assert_eq!(
        f.store
            .admit_transition_with_peer(&f.keys[0], command, &proof)
            .unwrap(),
        first
    );
}
