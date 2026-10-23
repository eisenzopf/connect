//! Controlled journal captures; physical movement is qualified by production fixtures.
use super::*;
mod signal;
fn prepared(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    key: &str,
) -> Arc<crate::TelephoneReplacementResourcesCapture> {
    let resources = captured(f, origin, sources, key);
    for phase in [Step::Attach, Step::Activate, Step::Readiness] {
        let read = f
            .store
            .try_telephone_replacement_scene_read(resources.clone(), sources)
            .unwrap();
        let a = f
            .store
            .begin_telephone_replacement_phase(&read, phase)
            .unwrap();
        let name = match phase {
            Step::Attach => "attach",
            Step::Activate => "activate",
            Step::Readiness => "readiness",
        };
        let mut capture = a.applied(observation(&read, name)).unwrap();
        if phase == Step::Activate {
            let mut revisions = std::collections::BTreeMap::new();
            for source in sources {
                let before:u64=f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[source.connection_id()],|r|r.get(0)).unwrap();
                revisions.insert(source.runtime_connection_id().into(), before + 1);
            }
            revisions.insert(read.audio().candidate.clone(), 3);
            capture = capture
                .with_audience(read.capture_activation_revisions(revisions).unwrap())
                .unwrap();
        }
        drop(read);
        f.store
            .record_telephone_replacement_phase(&capture)
            .unwrap();
    }
    resources
}
fn attempt(
    f: &Fixture,
    origin: &NativeReplacementOrigin,
    sources: &[Arc<crate::NativeMediaOrigin>],
    key: &str,
) -> (
    Arc<crate::TelephoneReplacementCommitAttempt>,
    crate::TelephoneReplacementCommitted,
) {
    let resources = prepared(f, origin, sources, key);
    let read = f
        .store
        .try_telephone_replacement_scene_read(resources, sources)
        .unwrap();
    let a = f.store.begin_telephone_replacement_commit(&read).unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_commit(&read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let mut revisions = std::collections::BTreeMap::new();
    for source in sources {
        let before:u64=f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[source.connection_id()],|r|r.get(0)).unwrap();
        revisions.insert(source.runtime_connection_id().into(), before + 1);
    }
    revisions.insert(read.audio().candidate.clone(), 4);
    let capture = a
        .committed(
            a.snapshot().expected_movement.clone(),
            &a.snapshot().media.stream_id,
            4,
            read.capture_commit_revisions(revisions).unwrap(),
        )
        .unwrap();
    drop(read);
    (a, capture)
}
#[test]
fn telephone_commit_requires_confirmed_private_readiness_and_never_begins_twice() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let resources = captured(&f, &origin, &sources, &key);
    let read = f
        .store
        .try_telephone_replacement_scene_read(resources, &sources)
        .unwrap();
    assert_eq!(
        f.store
            .begin_telephone_replacement_commit(&read)
            .unwrap_err()
            .code,
        "not-ready"
    );
    assert_eq!(f.count("telephone_replacement_commits"), 0);
}
#[test]
fn telephone_commit_records_actual_capture_before_projection_and_established_read_ignores_old_credentials(
) {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (a, capture) = attempt(&f, &origin, &sources, &key);
    assert!(f
        .store
        .project_telephone_replacement_committed(&capture)
        .is_err());
    let known = f
        .store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    assert_eq!(known.state, OperationState::Preparing);
    let established = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    let read = f
        .store
        .try_telephone_media_read(established.clone())
        .unwrap();
    assert!(read.current());
    assert_ne!(read.member().identity_id, read.route_actor().identity_id);
    assert_eq!(
        read.snapshot().runtime_connection_id,
        a.snapshot().runtime_connection_id
    );
    drop(read);
    let completed = f
        .store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    assert_eq!(completed.state, OperationState::Completed);
    let revision = completed.revision;
    f.store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    assert_eq!(
        f.store
            .record_telephone_replacement_committed(&capture)
            .unwrap()
            .revision,
        revision
    );
    let old = caller(&f, &origin);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id IN (?1,?2)",
            params![old, f.keys[0]],
        )
        .unwrap();
    assert!(f
        .store
        .try_telephone_media_read(established.clone())
        .unwrap()
        .current());
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&key])
        .unwrap();
    assert!(f.store.try_telephone_media_read(established).is_err());
    assert_eq!(f.count("telephone_replacement_commits"), 1);
    assert_eq!(f.count("telephone_connection_lifetimes"), 1);
}
#[test]
fn telephone_commit_uncertainty_late_known_result_and_projection_rollback_never_reexecute() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (a, capture) = attempt(&f, &origin, &sources, &key);
    assert_eq!(
        f.store
            .record_telephone_replacement_commit_uncertain(&a.uncertain())
            .unwrap()
            .state,
        OperationState::Unknown
    );
    assert_eq!(
        f.store
            .record_telephone_replacement_committed(&capture)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER deny_telephone_lifetime BEFORE INSERT ON telephone_connection_lifetimes BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .project_telephone_replacement_committed(&capture)
        .is_err());
    assert_eq!(f.count("telephone_connection_lifetimes"), 0);
    let exists: bool = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1)",
            [&a.snapshot().connection_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER deny_telephone_lifetime")
        .unwrap();
    let established = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    assert!(f
        .store
        .try_telephone_media_read(established)
        .unwrap()
        .current());
    assert!(f
        .store
        .record_telephone_replacement_commit_uncertain(&a.uncertain())
        .is_err());
}
struct Absent {
    id: String,
    generation: u64,
}
impl crate::TelephoneConnectionRuntimeObservation for Absent {
    fn observe_absent(&self, id: &str, generation: u64) -> Result<()> {
        if self.id == id && self.generation == generation {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
#[test]
fn telephone_established_cleanup_requires_exact_absence_preserves_retained_and_is_idempotent() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &key);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let established = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    let before: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [sources[1].connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    let s = established.snapshot();
    assert!(f
        .store
        .settle_telephone_connection_cleanup(
            &established,
            &Absent {
                id: s.runtime_connection_id.clone(),
                generation: s.generation + 1
            }
        )
        .is_err());
    let absent = Absent {
        id: s.runtime_connection_id.clone(),
        generation: s.generation,
    };
    f.store
        .settle_telephone_connection_cleanup(&established, &absent)
        .unwrap();
    let count = f.count("journal");
    f.store
        .settle_telephone_connection_cleanup(&established, &absent)
        .unwrap();
    assert_eq!(f.count("journal"), count);
    assert!(f.store.try_telephone_media_read(established).is_err());
    let after: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [sources[1].connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn telephone_established_pair_requires_independent_named_subscribe_and_both_recipient_ceilings() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &key);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let established = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    let pair = f
        .store
        .try_telephone_native_media_read(established.clone(), sources[1].clone())
        .unwrap();
    assert!(pair.current());
    assert!(pair.permits_telephone_receive());
    let native_permission = pair.native().permits_subscriptions();
    assert_eq!(pair.permits_native_receive(), native_permission);
    let logical = pair.telephone().member().identity_id.clone();
    drop(pair);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations='[\"sip_dial\"]' WHERE id=?1",
            [&key],
        )
        .unwrap();
    let pair = f
        .store
        .try_telephone_native_media_read(established.clone(), sources[1].clone())
        .unwrap();
    assert!(!pair.permits_telephone_receive());
    assert_eq!(pair.permits_native_receive(), native_permission);
    drop(pair);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![key, json!([logical]).to_string()],
        )
        .unwrap();
    assert!(f
        .store
        .try_telephone_media_read(established.clone())
        .unwrap()
        .current());
    assert!(f
        .store
        .try_telephone_native_media_read(established.clone(), sources[1].clone())
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients='[\"*\"]' WHERE id=?1",
            [&key],
        )
        .unwrap();
    let native_actor = {
        let c = f.store.connection().unwrap();
        f.store.native_media_route_actor(&c, &sources[1]).unwrap()
    };
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients='[]' WHERE id=?1",
            [&native_actor.credential_id],
        )
        .unwrap();
    assert!(f
        .store
        .try_telephone_native_media_read(established, sources[1].clone())
        .is_err());
}

fn native_selection(
    f: &Fixture,
    source: &crate::NativeMediaOrigin,
    operation: &str,
) -> crate::PeerAuthorityRequest {
    // Reuse the actual already-enrolled Device/key facts from ready(1).
    // Calling native_request_for again would provision a different public key.
    let mut request = crate::PeerAuthorityRequest {
        credentials: vec![f.keys[1].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-1".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-1".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::Binding,
    };
    let revision = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [source.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    request.command = crate::PeerCommand::NativeStream(connect_protocol::native_streams::Intent {
        operation_id: operation.into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: source.connection_id().into(),
        expected_revision: Some(revision),
        by_participant: None,
        command: connect_protocol::native_streams::Command::Subscribe,
        payload: json!({"subscriptions":[{"from_participant":f.pids[0],"kinds":["audio"]}]}),
    });
    request
}
fn grant_native_subscription(f: &Fixture) {
    f.store.connection().unwrap().execute("UPDATE credentials SET operations=json_insert(operations,'$[#]','stream.subscribe') WHERE id=?1",[&f.keys[1]]).unwrap();
}
#[test]
fn telephone_stream_pair_uses_exact_signed_attempt_and_independent_receive_permissions() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &key);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let telephone = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    grant_native_subscription(&f);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations='[\"sip_dial\"]' WHERE id=?1",
            [&key],
        )
        .unwrap();
    let request = native_selection(&f, &sources[1], "select-actual-telephone");
    let proof = f.admit(&request);
    f.store
        .accept_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap()
        .unwrap();
    let pair = f
        .store
        .telephone_native_stream_read_with_peer(telephone.clone(), &sources[1], &proof, &lease)
        .unwrap();
    assert!(pair.current() && pair.permits_native_receive());
    assert!(!pair.permits_telephone_receive());
    assert_eq!(
        pair.telephone().snapshot().runtime_connection_id,
        telephone.snapshot().runtime_connection_id
    );
    drop(pair);
    let other = native_selection(&f, &sources[1], "another-stream-intent");
    let other_proof = f.admit(&other);
    assert_eq!(
        f.store
            .telephone_native_stream_read_with_peer(telephone, &sources[1], &other_proof, &lease)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn telephone_stream_pair_reuses_complete_original_interval_behind_a_queued_route_writer() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &key);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let telephone = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    grant_native_subscription(&f);
    let request = native_selection(&f, &sources[1], "held-telephone-selection");
    let proof = f.admit(&request);
    f.store
        .accept_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap()
        .unwrap();
    let owner = telephone.snapshot().route_owner_id.clone();
    let write = crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| {
            let actor = f.store.revalidate(&f.admin)?;
            f.store.revoke(&actor, &key)
        });
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        // A nested acquisition would refuse this queued writer. The exact
        // original peer interval already contains its independent route owner.
        let pair = f
            .store
            .telephone_native_stream_read_with_peer(telephone.clone(), &sources[1], &proof, &lease)
            .unwrap();
        assert!(pair.current() && pair.permits_native_receive());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(pair);
        drop(proof);
        writer.join().unwrap().unwrap();
    });
    assert!(f
        .store
        .try_telephone_native_media_read(telephone, sources[1].clone())
        .is_err());
}

#[test]
fn telephone_source_native_replacement_uses_independent_current_origin_and_generation_three() {
    let (f, origin, sources, _, route) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &route);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let phone = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    assert_eq!(
        f.store
            .record_telephone_replacement_committed(&capture)
            .unwrap()
            .state,
        OperationState::Completed
    );
    let operator = crate::connection_control::tests::replacement_sources::key(&f);
    let target = crate::connection_control::tests::replacement_sources::endpoint(
        &f,
        vec!["audio".into()],
        "telephone-return",
    );
    let topology: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "telephone-return-native".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(phone.snapshot().connection_id.clone()),
        expected_revision: topology,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target.clone()),
            candidate_connid: None,
            retained_connid: sources[1].connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&operator, command.clone())
        .unwrap();
    let next = f
        .store
        .native_replacement_origin(&operator, command)
        .unwrap();
    let old = caller(&f, &origin);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &old).unwrap();
    f.store.revoke(&admin, &f.keys[0]).unwrap();
    let source = crate::NativeReplacementSourceOrigin::Telephone(phone.clone());
    let read = f
        .store
        .try_native_replacement_preparation_read(&next, &source, &sources[1])
        .unwrap();
    assert!(read.current());
    assert!(read.source().native().is_none());
    assert!(read.source().telephone().is_some());
    assert_eq!(
        read.source().session().actor.identity_id,
        phone.snapshot().identity_id
    );
    assert_ne!(
        read.source().telephone().unwrap().route_actor().identity_id,
        read.source().session().actor.identity_id
    );
    assert_eq!(read.context().source_generation, 2);
    assert_eq!(read.context().target_transport, EndpointTransport::Quic);
    assert_eq!(read.context().target_endpoint_id, target);
    let lease = f
        .store
        .prepare_native_replacement(
            read,
            crate::ReplacementPreparationRequest {
                candidate_connection_id: "return-native-next".into(),
                participant_revision: 2,
                timeout_seconds: 30,
                telephone_policy: None,
                confirmation: crate::ReplacementConfirmation::MediaOnly,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(lease.plan().unwrap()["generation"], 3);
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    assert!(read.current());
    assert!(read.source().telephone().is_some());
    drop(read);
    // The retired native origin cannot be substituted for the current SIP source.
    assert!(f
        .store
        .try_native_replacement_preparation_read(
            &next,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1]
        )
        .is_err());
    // Qualify the exact signed target interval, including an independent route
    // writer queued after intake. No nested source read may barge behind it.
    let read = f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .unwrap();
    let (attempt, read) = f
        .store
        .begin_native_replacement_preparation(&lease, read)
        .unwrap();
    let receipt = attempt
        .prepared(lease.plan().unwrap()["expected_preparation"].clone())
        .unwrap();
    drop(read);
    f.store
        .record_native_replacement_prepared(&receipt)
        .unwrap();
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let identity = f.store.revalidate(&operator).unwrap().identity_id;
    let target_key = f
        .store
        .issue(
            &identity,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap();
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "telephone-return-signing-key".into(),
                identity_id: identity,
                credential_id: target_key.id.clone(),
                device_id: "device-telephone-return".into(),
                key_id: "telephone-return-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    let mut intent = f.request(0, "telephone-return-target").intent;
    intent.operation_id = next.request().operation_id.clone();
    intent.connection_id = "return-native-next".into();
    intent.offer["replacement_preparation"] = json!({"profile":"connect-replacement-preparation-v1","operation_record_id":next.record_id()});
    let request = crate::PeerAuthorityRequest {
        credentials: vec![target_key.id.clone()],
        device: crate::PeerDeviceClaim {
            id: "device-telephone-return".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "telephone-return-key".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(intent),
    };
    let proof = f.admit(&request);
    let owner = phone.snapshot().route_owner_id.clone();
    let write = crate::AuthorityScope::identity(&owner, crate::AuthorityMode::Write).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| f.store.revoke(&admin, &route));
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        let read = f
            .store
            .native_replacement_allocation_read_with_peer(&receipt, &target_key.id, &proof)
            .unwrap();
        assert!(read.current());
        assert!(read.source().source().telephone().is_some());
        assert_eq!(read.source().context().source_generation, 2);
        drop(read);
        drop(proof);
        writer.join().unwrap().unwrap();
    });
    assert!(f
        .store
        .try_native_replacement_preparation_read(&next, &source, &sources[1])
        .is_err());
    assert!(f
        .store
        .native_replacement_preparation_attempt_read(&lease)
        .is_err());
    assert_eq!(f.store.connection().unwrap().query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='allocate' AND started=1",[next.record_id()],|r|r.get::<_,u64>(0)).unwrap(),0);
}

// Controlled Store grants only; actual roles/audio are exercised by the public
// configured SIP/native server fixture.
fn mixed_role_sources() -> (
    Fixture,
    Vec<Arc<crate::NativeMediaOrigin>>,
    Arc<crate::TelephoneMediaOrigin>,
    String,
) {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let (f, origin, mut sources, _, route) = setup_runtime(true);
    crate::connection_control::tests::replacement_sources::endpoint(
        &f,
        vec!["audio".into()],
        "role-extra",
    );
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "role-extra-key".into(),
                identity_id: identity,
                credential_id: f.keys[0].clone(),
                device_id: "device-role-extra".into(),
                key_id: "role-extra-key".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
                not_before: crate::now() - 1,
                expires_at: crate::now() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    let request = crate::PeerAuthorityRequest {
        credentials: vec![f.keys[0].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-role-extra".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "role-extra-key".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(f.request(0, "role-extra").intent),
    };
    let extra = crate::connection_control::tests::consultation_reads::ready_request(&f, 0, request);
    sources.push(extra.clone());
    let (_, capture) = attempt(&f, &origin, &sources, &route);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let phone = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    (f, vec![extra], phone, route)
}
#[test]
fn telephone_role_read_requires_complete_current_native_and_phone_sources_without_surrogate() {
    let (f, native, phone, _) = mixed_role_sources();
    let s = phone.snapshot();
    assert!(f
        .store
        .try_session_role_media_read(
            &[],
            &[phone.clone()],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch
        )
        .is_err());
    assert!(f
        .store
        .try_session_role_media_read(
            &native,
            &[],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch
        )
        .is_err());
    assert!(f
        .store
        .try_session_role_media_read(
            &native,
            &[phone.clone()],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch + 1
        )
        .is_err());
    let read = f
        .store
        .try_session_role_media_read(
            &native,
            &[phone.clone()],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch,
        )
        .unwrap();
    assert!(read.current());
    assert!(read.native(&s.runtime_connection_id).is_none());
    let sip = read.telephone(&s.runtime_connection_id).unwrap();
    let peer = read.native(native[0].runtime_connection_id()).unwrap();
    assert_eq!(sip.role(), peer.target().presence_role);
    assert_eq!(
        sip.session().topology_revision,
        peer.target().session.session.topology_revision
    );
    assert_eq!(
        sip.snapshot().participant_id,
        peer.target().connection.participant_id
    );
    assert_ne!(
        sip.route_actor().identity_id,
        peer.target().session.actor.identity_id
    );
    assert!(f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &s.route_owner_id,
            crate::AuthorityMode::Write
        )
        .unwrap()])
        .is_err());
    assert!(f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &s.identity_id,
            crate::AuthorityMode::Write
        )
        .unwrap()])
        .is_err());
}
#[test]
fn telephone_role_read_retains_one_complete_interval_behind_queued_route_revocation() {
    let (f, native, phone, route) = mixed_role_sources();
    let s = phone.snapshot();
    let read = f
        .store
        .try_session_role_media_read(
            &native,
            &[phone.clone()],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch,
        )
        .unwrap();
    let write =
        crate::AuthorityScope::identity(&s.route_owner_id, crate::AuthorityMode::Write).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| {
            let admin = f.store.revalidate(&f.admin)?;
            f.store.revoke(&admin, &route)
        });
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        assert!(read.current());
        assert!(read.telephone(&s.runtime_connection_id).unwrap().current());
        assert!(read
            .native(native[0].runtime_connection_id())
            .unwrap()
            .current());
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert!(f
        .store
        .try_session_role_media_read(
            &native,
            &[phone.clone()],
            &s.conversation_id,
            &s.session_id,
            &s.participant_id,
            s.membership_epoch
        )
        .is_err());
}
