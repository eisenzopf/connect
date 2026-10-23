use super::*;
use connect_protocol::session_control::{Action, Request};
use serde_json::json;
fn request(f: &Fixture) -> Request {
    let target = f.store.create_identity("session-target").unwrap().id;
    let Outcome::Conversation(room) = f
        .store
        .execute(
            &f.user,
            &Operation::Create {
                request_id: "session-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![
                    target.clone()
                ]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let members = f
        .store
        .membership_snapshot(&f.user, &room.id)
        .unwrap()
        .participants;
    Request {
        operation_id: "session-invite".into(),
        conversation_id: room.id,
        session_id: "protected-session".into(),
        by_participant: Some(
            members
                .iter()
                .find(|m| m.identity_id == f.identity)
                .unwrap()
                .participant_id
                .clone(),
        ),
        expected_revision: None,
        action: Action::Invite {
            participants: vec![members
                .iter()
                .find(|m| m.identity_id == target)
                .unwrap()
                .participant_id
                .clone()],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["opus"]}),
        },
    }
}
#[test]
fn session_control_requires_exact_current_owned_peer_proof_and_replays_one_receipt() {
    let f = Fixture::new(600);
    let command = request(&f);
    let mut admission = f.request.clone();
    admission.command = PeerCommand::Binding;
    let binding = f.admit(&admission).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_session_control_with_peer(&f.user, command.clone(), &binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(binding);
    admission.command = PeerCommand::SessionControl(command.clone());
    let proof = f.admit(&admission).unwrap();
    let first = f
        .store
        .execute_session_control_with_peer(&f.user, command.clone(), &proof)
        .unwrap();
    assert_eq!(
        first.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(
        f.store
            .execute_session_control_with_peer(&f.user, command.clone(), &proof)
            .unwrap(),
        first
    );
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let mut changed = command.clone();
    if let Action::Invite { intent, .. } = &mut changed.action {
        *intent = "changed".into();
    }
    assert_eq!(
        f.store
            .execute_session_control_with_peer(&f.user, changed, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let foreign = Fixture::new(600);
    assert_eq!(
        foreign
            .store
            .execute_session_control_with_peer(&foreign.user, command, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}
#[test]
fn session_control_source_expiring_behind_sql_cannot_create_presence_audience_or_receipt() {
    let f = Fixture::new(3);
    let command = request(&f);
    let mut admission = f.request.clone();
    admission.command = PeerCommand::SessionControl(command.clone());
    let proof = f.admit(&admission).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let expiry = proof.expires_at();
    let lock = f.store.connection().unwrap();
    let store = f.store.clone();
    let credential = f.user.clone();
    let (started, ready) = std::sync::mpsc::sync_channel(0);
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        store.execute_session_control_with_peer(&credential, command, &proof)
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    while crate::now() <= expiry {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!worker.is_finished());
    drop(lock);
    assert_eq!(worker.join().unwrap().unwrap_err().code, "forbidden");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn session_projection_requires_completed_exact_intent_and_retains_current_epoch() {
    let f = Fixture::new(600);
    let command = request(&f);
    let mut admission = f.request.clone();
    admission.command = PeerCommand::SessionControl(command.clone());
    let proof = f.admit(&admission).unwrap();
    assert!(f.store.project_session_with_peer(&f.user, &proof).is_err());
    f.store
        .execute_session_control_with_peer(&f.user, command.clone(), &proof)
        .unwrap();
    let projection = f.store.project_session_with_peer(&f.user, &proof).unwrap();
    assert_eq!(projection.actor.identity_id, f.identity);
    assert_eq!(projection.actor.membership_epoch, 1);
    assert!(projection.present);
    assert_eq!(projection.session.id, command.session_id);
    assert_eq!(projection.conversation_revision, 0);
    assert!(f.store.project_session_with_peer(&f.admin, &proof).is_err());
    drop(proof);
    let mut changed = command;
    if let Action::Invite { intent, .. } = &mut changed.action {
        *intent = "different".into();
    }
    admission.command = PeerCommand::SessionControl(changed);
    let changed_proof = f.admit(&admission).unwrap();
    assert!(f
        .store
        .project_session_with_peer(&f.user, &changed_proof)
        .is_err());
}

#[test]
fn identity_invitation_peer_proof_seals_original_reference_and_current_projection() {
    let f = Fixture::new(600);
    let mut command = request(&f);
    let Action::Invite { participants, .. } = &mut command.action else {
        panic!()
    };
    let pid = participants[0].clone();
    let members = f
        .store
        .membership_snapshot(&f.user, &command.conversation_id)
        .unwrap();
    participants[0] = members
        .participants
        .iter()
        .find(|m| m.participant_id == pid)
        .unwrap()
        .identity_id
        .clone();
    let mut admission = f.request.clone();
    admission.command = PeerCommand::SessionControl(command.clone());
    let proof = f.admit(&admission).unwrap();
    let first = f
        .store
        .execute_session_control_with_peer(&f.user, command.clone(), &proof)
        .unwrap();
    assert_eq!(
        first.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(
        f.store
            .project_session_with_peer(&f.user, &proof)
            .unwrap()
            .session
            .id,
        command.session_id
    );
    let mut alias = command.clone();
    if let Action::Invite { participants, .. } = &mut alias.action {
        participants[0] = pid;
    }
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .execute_session_control_with_peer(&f.user, alias, &proof)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(
        f.store
            .execute_session_control_with_peer(&f.user, command, &proof)
            .unwrap(),
        first
    );
}

fn owned_invitation(f: &Fixture) -> (String, Request, crate::SessionInvitationDecisionClaim) {
    let host = f.store.create_identity("owned-inviter").unwrap().id;
    let host = f
        .store
        .issue(
            &host,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let Outcome::Conversation(room) = f
        .store
        .execute(
            &host,
            &Operation::Create {
                request_id: "owned-invitation-room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![f
                    .identity
                    .clone()]),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let invite = Request {
        operation_id: "owned-invitation-original".into(),
        conversation_id: room.id,
        session_id: "owned-invitation-session".into(),
        by_participant: None,
        expected_revision: None,
        action: Action::Invite {
            participants: vec![f.identity.clone()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities: json!({"audio":["opus"]}),
        },
    };
    f.store
        .execute(
            &host,
            &Operation::SessionControl {
                request: invite.clone(),
            },
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    let record: String = c
        .query_row(
            "SELECT record_id FROM operations WHERE operation_id='owned-invitation-original'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    let source = f
        .store
        .session_invitation_sources(&record)
        .unwrap()
        .remove(0);
    let claim = crate::SessionInvitationDecisionClaim {
        decision: crate::SessionInvitationDecision::Accept {
            by_participant: Some(source.target_participant_id().into()),
            capabilities: json!({"audio":["opus"]}),
        },
        source,
    };
    (host, invite, claim)
}
#[test]
fn owned_invitation_decision_needs_exact_fresh_peer_claim_and_no_caller_operation() {
    let f = Fixture::new(600);
    let (_, invite, claim) = owned_invitation(&f);
    let mut request = f.request.clone();
    request.command = PeerCommand::Binding;
    let proof = f.admit(&request).unwrap();
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(proof);
    request.command = PeerCommand::SessionInvitationDecision(claim.clone());
    let proof = f.admit(&request).unwrap();
    let c = f.store.connection().unwrap();
    let before: u64 = c
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    drop(c);
    let untouched = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| false)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), untouched);
    let original = f
        .store
        .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
        .unwrap();
    assert_eq!(original.session.state, "accepted");
    assert_eq!(original.session.topology_revision, 1);
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
            .unwrap(),
        original
    );
    let mut changed = claim.clone();
    changed.decision = crate::SessionInvitationDecision::Reject {
        by_participant: None,
        reason_code: 486,
        reason: "busy".into(),
    };
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, changed, &proof, &|| true)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(proof);
    let proof = f.admit(&request).unwrap();
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim, &proof, &|| true)
            .unwrap(),
        original
    );
    drop(proof);
    let c = f.store.connection().unwrap();
    let after: u64 = c
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    let ordinary_correlations:u64=c.query_row("SELECT COUNT(*) FROM journal WHERE conversation_id=?1 AND kind='session_control' AND json_extract(data_json,'$.action')='accept' AND json_extract(data_json,'$.operation_id') IS NOT NULL",[invite.conversation_id],|r|r.get(0)).unwrap();
    assert_eq!(ordinary_correlations, 0);
    assert!(c
        .execute(
            "UPDATE session_invitation_decisions SET decision_json='{}'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM session_invitation_decisions", [])
        .is_err());
}
#[test]
fn owned_invitation_reject_is_atomic_and_storage_recovery_retains_one_original_decision() {
    let f = Fixture::new(600);
    let (_, _, mut claim) = owned_invitation(&f);
    claim.decision = crate::SessionInvitationDecision::Reject {
        by_participant: Some(claim.source.target_participant_id().into()),
        reason_code: 486,
        reason: "busy".into(),
    };
    let mut request = f.request.clone();
    request.command = PeerCommand::SessionInvitationDecision(claim.clone());
    let proof = f.admit(&request).unwrap();
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TEMP TRIGGER reject_owned_journal BEFORE INSERT ON journal WHEN NEW.kind='session_control' BEGIN SELECT RAISE(ABORT,'injected owned decision failure'); END;").unwrap();
    drop(c);
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    let c = f.store.connection().unwrap();
    let count: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM session_invitation_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let state: String = c
        .query_row(
            "SELECT state FROM sessions WHERE id='owned-invitation-session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "invited");
    c.execute_batch("DROP TRIGGER reject_owned_journal")
        .unwrap();
    drop(c);
    let original = f
        .store
        .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
        .unwrap();
    assert_eq!(original.session.state, "rejected");
    assert_eq!(original.session.ended_reason.as_deref(), Some("busy"));
    assert_eq!(
        f.store
            .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
            .unwrap(),
        original
    );
    drop(proof);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE peer_signing_keys SET revoked=1 WHERE key_id='key'",
        [],
    )
    .unwrap();
    drop(c);
    assert_eq!(f.admit(&request).unwrap_err().code, "forbidden");
}
#[test]
fn owned_invitation_old_slot_and_spoofed_by_cannot_adopt_a_later_invitation() {
    let f = Fixture::new(600);
    let (host, invite, claim) = owned_invitation(&f);
    let mut request = f.request.clone();
    let mut spoofed = claim.clone();
    if let crate::SessionInvitationDecision::Accept { by_participant, .. } = &mut spoofed.decision {
        *by_participant = Some("other-participant".into());
    }
    request.command = PeerCommand::SessionInvitationDecision(spoofed);
    assert_eq!(f.admit(&request).unwrap_err().code, "forbidden");
    f.store
        .execute(
            &f.user,
            &Operation::SessionControl {
                request: Request {
                    operation_id: "ordinary-accept-original-source".into(),
                    action: Action::Accept {
                        capabilities: json!({"audio":["opus"]}),
                    },
                    expected_revision: Some(0),
                    ..invite.clone()
                },
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.user,
            &Operation::SessionControl {
                request: Request {
                    operation_id: "leave-accepted-source".into(),
                    action: Action::Leave {
                        participant_id: claim.source.target_participant_id().into(),
                    },
                    expected_revision: Some(1),
                    ..invite.clone()
                },
            },
        )
        .unwrap();
    f.store
        .execute(
            &host,
            &Operation::SessionControl {
                request: Request {
                    operation_id: "new-original-invitation".into(),
                    expected_revision: Some(2),
                    ..invite
                },
            },
        )
        .unwrap();
    request.command = PeerCommand::SessionInvitationDecision(claim);
    assert_eq!(f.admit(&request).unwrap_err().code, "forbidden");
}

fn relay_caller_request(
    f: &Fixture,
    credential: &str,
    source: crate::SessionInvitationSource,
) -> PeerAuthorityRequest {
    let caller = f.store.revalidate(credential).unwrap().identity_id;
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "relay-caller-device".into(),
                identity_id: caller.clone(),
                device: DeviceSpec {
                    id: "relay-caller-device".into(),
                    kind: "desktop".into(),
                    platform: "fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: "quic://localhost:4445".into(),
                capabilities: vec!["data".into()],
                secret_ref: None,
            },
        )
        .unwrap();
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "relay-caller-key".into(),
                identity_id: caller,
                credential_id: credential.into(),
                device_id: "relay-caller-device".into(),
                key_id: "relay-caller-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    PeerAuthorityRequest {
        credentials: vec![credential.into()],
        device: PeerDeviceClaim {
            id: "relay-caller-device".into(),
            kind: "desktop".into(),
            platform: "fixture".into(),
        },
        key_id: "relay-caller-key".into(),
        algorithm: "EdDSA".into(),
        command: PeerCommand::SessionInvitationRelayRead(source),
    }
}
#[test]
fn invitation_relays_require_original_caller_receipt_current_key_scope_and_exact_read_proof() {
    let f = Fixture::new(600);
    let (host, invite, claim) = owned_invitation(&f);
    let mut caller = relay_caller_request(&f, &host, claim.source.clone());
    assert_eq!(f.admit(&caller).unwrap_err().code, "forbidden");
    let mut receiver = f.request.clone();
    receiver.command = PeerCommand::SessionInvitationDecision(claim.clone());
    let proof = f.admit(&receiver).unwrap();
    let original = f
        .store
        .apply_session_invitation_decision(&f.user, claim.clone(), &proof, &|| true)
        .unwrap();
    drop(proof);
    caller.command = PeerCommand::Binding;
    let binding = f.admit(&caller).unwrap();
    assert_eq!(
        f.store
            .session_invitation_relay_with_peer(&claim.source, &host, &binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(binding);
    caller.command = PeerCommand::SessionInvitationRelayRead(claim.source.clone());
    let proof = f.admit(&caller).unwrap();
    let (op, outcome) = f
        .store
        .session_invitation_relay_with_peer(&claim.source, &host, &proof)
        .unwrap();
    assert_eq!(op, invite.operation_id);
    assert_eq!(outcome, original);
    drop(proof);
    receiver.command = PeerCommand::SessionInvitationRelayRead(claim.source.clone());
    assert_eq!(f.admit(&receiver).unwrap_err().code, "forbidden");
    let caller_identity = f.store.revalidate(&host).unwrap().identity_id;
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET recipients=?2 WHERE id=?1",
        rusqlite::params![host, serde_json::to_string(&vec![caller_identity]).unwrap()],
    )
    .unwrap();
    drop(c);
    assert_eq!(f.admit(&caller).unwrap_err().code, "forbidden");
}

#[test]
fn cancelled_invitation_observation_requires_its_exact_current_recipient_read_union() {
    let f = Fixture::new(600);
    let (host, invite, claim) = owned_invitation(&f);
    let mut request = f.request.clone();
    request.command = PeerCommand::SessionInvitationCancellationRead(claim.source.clone());
    assert_eq!(f.admit(&request).unwrap_err().code, "forbidden");
    f.store
        .execute(
            &host,
            &Operation::SessionControl {
                request: Request {
                    operation_id: "owned-cancel".into(),
                    action: Action::Cancel {
                        reason_code: 487,
                        reason: "request-cancelled".into(),
                    },
                    expected_revision: Some(0),
                    ..invite
                },
            },
        )
        .unwrap();
    request.command = PeerCommand::Binding;
    let binding = f.admit(&request).unwrap();
    assert_eq!(
        f.store
            .session_invitation_cancellation_with_peer(&claim.source, &f.user, &binding)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(binding);
    request.command = PeerCommand::SessionInvitationCancellationRead(claim.source.clone());
    let proof = f.admit(&request).unwrap();
    let original = f
        .store
        .session_invitation_cancellation_with_peer(&claim.source, &f.user, &proof)
        .unwrap();
    assert_eq!(original.reason_code, 487);
    assert_eq!(original.reason, "request-cancelled");
    assert_eq!(original.session_id, "owned-invitation-session");
    drop(proof);
    let mut decision = f.request.clone();
    decision.command = PeerCommand::SessionInvitationDecision(claim);
    assert_eq!(f.admit(&decision).unwrap_err().code, "forbidden");
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET operations='[\"send\"]' WHERE id=?1",
        [&f.user],
    )
    .unwrap();
    drop(c);
    assert_eq!(f.admit(&request).unwrap_err().code, "forbidden");
}
