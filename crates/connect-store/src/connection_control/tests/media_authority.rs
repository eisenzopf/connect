//! Current Store authority only. No physical/media readiness is claimed here.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeMediaOrigin};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
mod packet_reads;
impl Fixture {
    fn media_attempt(
        &self,
        actor: usize,
        proof: &crate::PeerAuthorityGuard,
    ) -> crate::NativeOfferLease {
        self.store
            .accept_native_offer_with_peer(&self.keys[actor], proof)
            .unwrap();
        self.store
            .begin_native_offer_with_peer(&self.keys[actor], proof)
            .unwrap()
            .unwrap()
            .0
    }
    fn origin(&self, actor: usize, slot: &str) -> Arc<NativeMediaOrigin> {
        let request = self.native_request_for(actor, 600, slot);
        let proof = self.admit(&request);
        let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
            panic!()
        };
        self.store
            .prepare_connection_with_peer(
                &self.keys[actor],
                intent.clone(),
                self.endpoints[actor].clone(),
                &proof,
            )
            .unwrap();
        let lease = self.media_attempt(actor, &proof);
        self.store
            .native_media_origin_with_peer(&self.keys[actor], &proof, &lease)
            .unwrap()
    }
}
#[test]
fn current_media_origins_hold_no_idle_grant_and_delivery_readers_fence_queued_writers() {
    let f = Fixture::new();
    let target = f.origin(0, "target");
    let source = f.origin(1, "source");
    let writer_scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap();
    let idle = f
        .store
        .try_acquire_authority(&[writer_scope.clone()])
        .unwrap();
    drop(idle);
    let read = f
        .store
        .try_native_media_read(&target, Some(&source))
        .unwrap();
    assert!(read.current());
    assert_eq!(read.target().connection.participant_id, f.pids[0]);
    assert_eq!(read.source().unwrap().connection.participant_id, f.pids[1]);
    assert_eq!(
        source.runtime_connection_id(),
        read.source().unwrap().runtime_connection_id
    );
    assert_eq!(read.target().connection.state, "preparing");
    let peer = Store::open_administration(f.store.path(), f.store.limits.clone()).unwrap();
    let scope = writer_scope.clone();
    let pending = std::thread::spawn(move || {
        peer.acquire_authority(&[scope], &crate::AuthorityWait::new(Duration::from_secs(2)))
    });
    crate::authority_fences::test_wait_for_writer(&f.store, &writer_scope);
    let start = Instant::now();
    assert_eq!(
        f.store
            .try_native_media_read(&target, Some(&source))
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(!pending.is_finished());
    drop(read);
    drop(pending.join().unwrap().unwrap());
    assert!(f
        .store
        .try_native_media_read(&target, Some(&source))
        .is_ok());
}
#[test]
fn current_media_control_reuses_exact_invocation_without_blocking_existing_media_readers() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let proof = f.admit(&request);
    let intent = f.request(0, "native").intent;
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent.clone(), f.endpoints[0].clone(), &proof)
        .unwrap();
    let lease = f.media_attempt(0, &proof);
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    let held = f
        .store
        .native_media_read_with_peer(&origin, None, &proof)
        .unwrap();
    assert!(held.current());
    assert!(f.store.try_native_media_read(&origin, None).is_ok());
    let scope = AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap();
    let peer = Store::open_administration(f.store.path(), f.store.limits.clone()).unwrap();
    let writer_scope = scope.clone();
    let pending = std::thread::spawn(move || {
        peer.acquire_authority(
            &[writer_scope],
            &crate::AuthorityWait::new(Duration::from_secs(2)),
        )
    });
    crate::authority_fences::test_wait_for_writer(&f.store, &scope);
    // A real writer now owns admission. The in-flight exact invocation must
    // finish under its original reader, without reentering behind that writer.
    let continued = f
        .store
        .native_media_read_with_peer(&origin, None, &proof)
        .unwrap();
    assert!(continued.current());
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(continued);
    drop(held);
    drop(proof);
    drop(pending.join().unwrap().unwrap());
    let mut changed = request;
    changed.command =
        crate::PeerCommand::NativeConnectionOffer(f.request(0, "other-intent").intent);
    let other = f.admit(&changed);
    assert_eq!(
        f.store
            .native_media_read_with_peer(&origin, None, &other)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    drop(other);
    assert!(f.store.try_native_media_read(&origin, None).is_ok());
}
#[test]
fn current_media_refuses_sql_contention_and_foreign_store_without_wait_or_side_effect() {
    let f = Fixture::new();
    let origin = f.origin(0, "target");
    let count = f.count("operations");
    let c = f.store.connection().unwrap();
    let start = Instant::now();
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(c);
    assert_eq!(f.count("operations"), count);
    let foreign = Fixture::new();
    assert_eq!(
        foreign
            .store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(f.store.try_native_media_read(&origin, None).is_ok());
}
#[test]
fn current_media_rechecks_source_credential_key_device_and_endpoint_retirement() {
    for kind in ["credential", "key", "device", "endpoint"] {
        let f = Fixture::new();
        let target = f.origin(0, "target");
        let source = f.origin(1, "source");
        assert!(f
            .store
            .try_native_media_read(&target, Some(&source))
            .is_ok());
        let command = match kind {
            "credential" => ProvisioningCommand::RevokeCredential {
                operation_id: "retire-source".into(),
                credential_id: f.keys[1].clone(),
            },
            "key" => ProvisioningCommand::RevokeSigningKey {
                operation_id: "retire-source".into(),
                key_id: "native-key-1".into(),
                expected_revision: 0,
            },
            "device" => ProvisioningCommand::RevokeDevice {
                operation_id: "retire-source".into(),
                device_id: "device-1".into(),
                expected_revision: 0,
            },
            _ => ProvisioningCommand::RevokeEndpoint {
                operation_id: "retire-source".into(),
                endpoint_id: f.endpoints[1].clone(),
                expected_revision: 0,
            },
        };
        f.store.execute_provisioning(&f.admin, command).unwrap();
        assert_eq!(
            f.store
                .try_native_media_read(&target, Some(&source))
                .err()
                .unwrap()
                .code,
            if kind == "credential" {
                "unauthorized"
            } else {
                "forbidden"
            },
            "{kind}"
        );
        assert!(
            f.store.try_native_media_read(&target, None).is_ok(),
            "{kind} affected another endpoint"
        );
    }
}
#[test]
fn current_media_leave_fences_only_its_current_slot_and_preserves_another_owner() {
    let f = Fixture::new();
    let target = f.origin(0, "target");
    let source = f.origin(1, "source");
    f.store
        .execute(
            &f.keys[1],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "source-leave".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: Some(f.pids[1].clone()),
                    expected_revision: Some(3),
                    action: Action::Leave {
                        participant_id: f.pids[1].clone(),
                    },
                },
            },
        )
        .unwrap();
    assert!(f.store.try_native_media_read(&target, None).is_ok());
    assert_eq!(
        f.store
            .try_native_media_read(&target, Some(&source))
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .try_native_media_read(&source, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}
#[test]
fn current_media_origin_cannot_extend_the_original_signed_source_deadline() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let mut proof = f.admit(&request);
    proof.limit_expiry(crate::now() + 2).unwrap();
    let intent = f.request(0, "native").intent;
    f.store
        .prepare_connection_with_peer(&f.keys[0], intent, f.endpoints[0].clone(), &proof)
        .unwrap();
    let lease = f.media_attempt(0, &proof);
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    assert!(f.store.try_native_media_read(&origin, None).is_ok());
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(f.store.credential_metadata(&f.keys[0]).is_ok());
}

#[test]
fn current_media_accepts_its_process_observation_and_refuses_another_runtime_epoch() {
    let f = Fixture::new();
    let target = f.origin(0, "target");
    let source = f.origin(1, "source");
    assert!(f.store.try_native_media_read(&target, None).is_ok());
    // These controlled process observations grant no physical media readiness.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET observed_runtime_id=?2 WHERE id=?1",
            params![target.connection_id(), f.store.runtime_epoch()],
        )
        .unwrap();
    assert!(f.store.try_native_media_read(&target, None).is_ok());
    let foreign = Fixture::new();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET observed_runtime_id=?2 WHERE id=?1",
            params![target.connection_id(), foreign.store.runtime_epoch()],
        )
        .unwrap();
    assert_eq!(
        f.store
            .try_native_media_read(&target, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(f.store.try_native_media_read(&source, None).is_ok());
}

#[test]
fn current_media_join_does_not_expand_an_original_senders_recipient_ceiling() {
    let mut f = Fixture::new();
    let identities = f
        .keys
        .iter()
        .map(|key| f.store.revalidate(key).unwrap().identity_id)
        .collect::<Vec<_>>();
    f.keys[1] = f
        .store
        .issue(
            &identities[1],
            Scope::collaboration(identities.clone()),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let source = f.origin(1, "source");
    let old_target = f.origin(0, "old-target");
    let ProvisioningOutcome::Identity { identity_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Identity {
                operation_id: "later-identity".into(),
                alias: "later-identity".into(),
                kind: IdentityKind::Human,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let key = f
        .store
        .issue(
            &identity_id,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Endpoint {
                operation_id: "later-endpoint".into(),
                identity_id: identity_id.clone(),
                device: DeviceSpec {
                    id: "device-4".into(),
                    kind: "desktop".into(),
                    platform: "isolated-store-fixture".into(),
                },
                transport: EndpointTransport::Quic,
                address: "quic://localhost:4450".into(),
                capabilities: vec!["audio".into()],
                secret_ref: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-later-endpoint".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-fixture".into(),
            },
        )
        .unwrap();
    let ProvisioningOutcome::Participant { participant_id, .. } = f
        .store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::Participant {
                operation_id: "later-member".into(),
                conversation_id: f.cid.clone(),
                identity_id,
                role: "participant".into(),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "invite-later".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(3),
                    action: Action::Invite {
                        participants: vec![participant_id.clone()],
                        medium: "voice".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    let revision = crate::session_control::view(&f.store.connection().unwrap(), &f.cid, "session")
        .unwrap()
        .topology_revision;
    f.store
        .execute(
            &key,
            &Operation::SessionControl {
                request: Request {
                    operation_id: "accept-later".into(),
                    conversation_id: f.cid.clone(),
                    session_id: "session".into(),
                    by_participant: None,
                    expected_revision: Some(revision),
                    action: Action::Accept {
                        capabilities: json!({"audio":["opus"]}),
                    },
                },
            },
        )
        .unwrap();
    f.keys.push(key);
    f.pids.push(participant_id);
    f.endpoints.push(endpoint_id);
    let later = f.origin(4, "later-target");
    assert!(f.store.try_native_media_read(&source, None).is_ok());
    assert!(f
        .store
        .try_native_media_read(&old_target, Some(&source))
        .is_ok());
    assert!(f.store.try_native_media_read(&later, None).is_ok());
    assert_eq!(
        f.store
            .try_native_media_read(&later, Some(&source))
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

#[test]
fn native_wire_cancellation_fences_media_origin_and_held_control_without_retiring_another_slot() {
    let mut f = Fixture::new();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("operation.cancel".into());
    f.keys[0] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    let other = f.origin(1, "other");
    let request = f.native_request(600);
    let proof = f.admit(&request);
    f.store
        .prepare_connection_with_peer(
            &f.keys[0],
            f.request(0, "native").intent,
            f.endpoints[0].clone(),
            &proof,
        )
        .unwrap();
    let lease = f.media_attempt(0, &proof);
    let target = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    assert!(f
        .store
        .native_media_read_with_peer(&target, None, &proof)
        .is_ok());
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "cancel-native".into(),
                target_operation_id: "prepare-native".into(),
                reason: None,
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .try_native_media_read(&target, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .native_media_read_with_peer(&target, None, &proof)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .native_media_origin_with_peer(&f.keys[0], &proof, &lease)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert!(f.store.try_native_media_read(&other, None).is_ok());
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM connections WHERE id=?1",
            [target.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "preparing"); // Unknown effect still consumes its own slot.
}

#[test]
fn native_media_origin_cannot_renew_an_accepted_sources_deadline_with_a_fresh_proof() {
    let f = Fixture::new();
    let request = f.native_request(600);
    let mut original = f.admit(&request);
    original.limit_expiry(crate::now() + 2).unwrap();
    f.store
        .prepare_connection_with_peer(
            &f.keys[0],
            f.request(0, "native").intent,
            f.endpoints[0].clone(),
            &original,
        )
        .unwrap();
    let lease = f.media_attempt(0, &original);
    let fresh = f.admit(&request);
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[0], &fresh, &lease)
        .unwrap();
    drop(original);
    let read = f
        .store
        .native_media_read_with_peer(&origin, None, &fresh)
        .unwrap();
    assert!(read.current());
    std::thread::sleep(Duration::from_millis(2100));
    assert!(fresh.current());
    assert!(!read.current());
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .native_media_read_with_peer(&origin, None, &fresh)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

#[test]
fn native_packet_source_reads_keep_subscription_permission_separate_and_current() {
    let mut f = Fixture::new();
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("stream.subscribe".into());
    f.keys[0] = f
        .store
        .issue(&actor.identity_id, scope, crate::now() + 600)
        .unwrap()
        .id;
    let target = f.origin(0, "receiver");
    let source = f.origin(1, "publisher");
    let read = f
        .store
        .try_native_media_read(&target, Some(&source))
        .unwrap();
    assert!(read.permits_subscriptions());
    drop(read);
    // Controlled current-policy narrowing under the actual owning Identity
    // writer. This is a Store authorization fixture, not physical IO evidence.
    let held = f
        .store
        .acquire_authority(
            &[AuthorityScope::identity(&actor.identity_id, AuthorityMode::Write).unwrap()],
            &crate::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![
                f.keys[0],
                serde_json::to_string(&Scope::collaboration(vec!["*".into()]).operations).unwrap()
            ],
        )
        .unwrap();
    drop(held);
    let current = f
        .store
        .try_native_media_read(&target, Some(&source))
        .unwrap();
    assert!(current.current());
    assert!(!current.permits_subscriptions());
}
