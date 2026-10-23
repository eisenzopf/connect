//! Controlled Store/Core-result witnesses, never actual native rotation evidence.
use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;
fn setup() -> (
    Fixture,
    PeerAuthorityRequest,
    Arc<crate::NativeMediaOrigin>,
    String,
) {
    let (f, mut request, _, origin) = parent_permissions_origin_extra(
        true,
        true,
        false,
        false,
        true,
        &["dtmf.send", "dtmf.receive"],
    );
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let next = f
        .store
        .issue(&actor.identity_id, actor.scope.clone(), crate::now() + 900)
        .unwrap();
    let key = f
        .store
        .resolve_peer_signing_key(
            &f.keys[0],
            "tnt_default",
            &request.key_id,
            &request.algorithm,
        )
        .unwrap();
    let revision: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT revision FROM peer_signing_keys WHERE key_id=?1",
            [&request.key_id],
            |r| r.get(0),
        )
        .unwrap();
    use base64::Engine;
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-media-refresh-next".into(),
                identity_id: actor.identity_id,
                credential_id: next.id.clone(),
                device_id: request.device.id.clone(),
                key_id: request.key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(revision),
            },
        )
        .unwrap();
    request.credentials = vec![f.keys[0].clone(), next.id.clone()];
    request.command = PeerCommand::Binding;
    (f, request, origin, next.id)
}
fn admission(
    f: &Fixture,
    request: &PeerAuthorityRequest,
    origin: &Arc<crate::NativeMediaOrigin>,
) -> (
    crate::PeerAuthorityGuard,
    crate::NativeCredentialRefreshPlan,
) {
    f.store
        .admit_native_credential_refresh(
            request,
            std::slice::from_ref(origin),
            &AuthorityWait::new(Duration::from_secs(2)),
        )
        .unwrap()
}
#[test]
fn credential_refresh_observed_epoch_preserves_source_ids_and_rejects_old_origin() {
    let (f, request, origin, next) = setup();
    let original_offer = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT credential_id FROM native_offer_operations WHERE operation_record_id=?1",
            [origin.offer_record_id()],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    let (proof, plan) = admission(&f, &request, &origin);
    let target = &plan.targets()[0];
    let renewed = target.next().clone();
    assert_eq!(target.expected_credential_revision(), 1);
    assert_eq!(renewed.credential_epoch(), 1);
    assert_eq!(renewed.connection_id(), origin.connection_id());
    assert_eq!(renewed.offer_record_id(), origin.offer_record_id());
    assert!(f.store.try_native_media_read(&renewed, None).is_err());
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "original-auth-refresh")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    assert!(f
        .store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .is_err());
    assert!(f
        .store
        .record_native_credential_refresh_stopped(&attempt, false)
        .is_err());
    let receipt = attempt
        .applied(BTreeMap::from([(origin.offer_record_id().into(), 2)]))
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    drop(proof);
    assert!(f.store.try_native_media_read(&origin, None).is_err());
    assert!(f
        .store
        .try_native_media_read(&renewed, None)
        .unwrap()
        .current());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT credential_id FROM native_admission_sources WHERE operation_record_id=?1",
            [origin.offer_record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        next
    );
    assert_eq!(
        c.query_row(
            "SELECT credential_id FROM native_offer_operations WHERE operation_record_id=?1",
            [origin.offer_record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original_offer
    );
    assert!(c
        .execute(
            "UPDATE native_media_credential_bindings SET credential_id=?1",
            [&original_offer]
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM native_media_credential_bindings", [])
        .is_err());
}
#[test]
fn credential_refresh_uncertain_submission_never_lends_either_epoch_or_resubmits() {
    let (f, request, origin, _) = setup();
    let (proof, plan) = admission(&f, &request, &origin);
    let next = plan.targets()[0].next().clone();
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "uncertain-auth-refresh")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    f.store
        .record_native_credential_refresh_stopped(&attempt, true)
        .unwrap();
    f.store
        .record_native_credential_refresh_stopped(&attempt, true)
        .unwrap();
    assert!(f.store.try_native_media_read(&origin, None).is_err());
    assert!(f.store.try_native_media_read(&next, None).is_err());
    assert!(f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "new-refresh-cannot-retry")
        .is_err());
    // Only the original captured result can settle metadata; it does not IO.
    let observed = attempt
        .applied(BTreeMap::from([(origin.offer_record_id().into(), 2)]))
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&observed)
        .unwrap();
    drop(proof);
    assert!(f
        .store
        .try_native_media_read(&next, None)
        .unwrap()
        .current());
}
#[test]
fn credential_refresh_refusal_before_submission_keeps_original_authority() {
    let (f, request, origin, _) = setup();
    let (proof, plan) = admission(&f, &request, &origin);
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "not-submitted-auth-refresh")
        .unwrap();
    f.store
        .record_native_credential_refresh_stopped(&attempt, false)
        .unwrap();
    assert!(f
        .store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .is_err());
    drop(proof);
    assert!(f
        .store
        .try_native_media_read(&origin, None)
        .unwrap()
        .current());
    let (proof, plan) = admission(&f, &request, &origin);
    assert!(f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "fresh-explicit-auth-refresh")
        .is_ok());
}
#[test]
fn credential_refresh_requires_exact_old_credential_device_key_and_current_successor() {
    for mode in ["old", "device", "key", "revoked", "duplicate"] {
        let (f, mut request, origin, next) = setup();
        match mode {
            "old" => request.credentials[0] = next.clone(),
            "device" => request.device.id = "unbound-device".into(),
            "key" => request.key_id = "unbound-key".into(),
            "revoked" => {
                let actor = f.store.revalidate(&f.admin).unwrap();
                f.store.revoke(&actor, &next).unwrap();
            }
            "duplicate" => {}
            _ => unreachable!(),
        }
        let origins = if mode == "duplicate" {
            vec![origin.clone(), origin]
        } else {
            vec![origin]
        };
        assert!(f
            .store
            .admit_native_credential_refresh(
                &request,
                &origins,
                &AuthorityWait::new(Duration::from_secs(1))
            )
            .is_err());
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM native_media_credential_bindings",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
    }
}
#[test]
fn credential_refresh_wrong_core_observation_cannot_publish_successor_metadata() {
    let (f, request, origin, _) = setup();
    let (proof, plan) = admission(&f, &request, &origin);
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "wrong-observation-refresh")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    assert!(attempt
        .applied(BTreeMap::from([(origin.offer_record_id().into(), 3)]))
        .is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM native_media_credential_bindings WHERE state='applied'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    drop(proof);
    assert_eq!(
        f.store
            .try_native_media_read(&origin, None)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
}

#[test]
fn credential_refresh_new_stream_intent_uses_observed_successor_not_predecessor() {
    let (f, request, origin, next) = setup();
    let mut stream = request.clone();
    stream.credentials = vec![next.clone()];
    stream.command = PeerCommand::NativeStream(Intent {
        operation_id: "signal-after-observed-rotation".into(),
        conversation_id: origin.conversation_id().into(),
        session_id: origin.session_id().into(),
        connection_id: origin.connection_id().into(),
        expected_revision: Some(1),
        by_participant: None,
        command: Command::Dtmf,
        payload: json!({"digits":"9#","duration_ms":100,"method":"info"}),
    });
    assert!(admit(&f, &stream).is_err());
    let (proof, plan) = admission(&f, &request, &origin);
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "source-rotation-for-command")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    let receipt = attempt
        .applied(BTreeMap::from([(origin.offer_record_id().into(), 2)]))
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    drop(proof);
    let successor = admit(&f, &stream).unwrap();
    let accepted = f
        .store
        .accept_native_stream_with_peer(&next, &successor)
        .unwrap();
    assert_eq!(
        accepted.state,
        connect_protocol::transitions::OperationState::Accepted
    );
    drop(successor);
    stream.credentials = vec![f.keys[0].clone()];
    let PeerCommand::NativeStream(wire) = &mut stream.command else {
        panic!()
    };
    wire.operation_id = "old-credential-cannot-publish-new-signal".into();
    assert!(admit(&f, &stream).is_err());
}

#[test]
fn credential_refresh_restart_classifies_submission_without_reusing_custody() {
    for started in [false, true] {
        let (f, request, origin, _) = setup();
        let (proof, plan) = admission(&f, &request, &origin);
        let attempt = f
            .store
            .prepare_native_credential_refresh(&plan, &proof, "before-restart-refresh")
            .unwrap();
        if started {
            f.store
                .begin_native_credential_refresh(&attempt, &plan, &proof)
                .unwrap();
        }
        let path = f.store.path.clone();
        drop(proof);
        drop(plan);
        drop(f.store);
        let store = Store::open(&path, Limits::default()).unwrap();
        assert_eq!(store.recover_native_credential_refreshes(1).unwrap(), 1);
        assert_eq!(store.recover_native_credential_refreshes(1).unwrap(), 0);
        let state: String = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM native_media_credential_bindings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if started { "uncertain" } else { "refused" });
        assert!(store
            .record_native_credential_refresh_stopped(&attempt, false)
            .is_err());
        assert!(store.try_native_media_read(&origin, None).is_err());
    }
}

#[test]
fn credential_refresh_does_not_strand_an_original_pending_native_control() {
    let (f, request, origin, _) = setup();
    let mut control = request.clone();
    control.credentials = vec![f.keys[0].clone()];
    control.command = PeerCommand::NativeStream(Intent {
        operation_id: "pending-control-before-refresh".into(),
        conversation_id: origin.conversation_id().into(),
        session_id: origin.session_id().into(),
        connection_id: origin.connection_id().into(),
        expected_revision: Some(1),
        by_participant: None,
        command: Command::Dtmf,
        payload: serde_json::json!({"digits":"1","duration_ms":100,"method":"info"}),
    });
    let proof = admit(&f, &control).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    drop(proof);
    let error = f
        .store
        .admit_native_credential_refresh(
            &request,
            &[origin],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap_err();
    assert_eq!(error.code, "authority-busy");
    let count: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM native_media_credential_bindings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn credential_refresh_signal_only_epoch_does_not_need_or_restore_media_permission() {
    let (f, request, origin, next) = setup();
    let c = f.store.connection().unwrap();
    for credential in &request.credentials {
        c.execute(
            "UPDATE credentials SET operations='[\"connect:control\",\"dtmf.send\"]' WHERE id=?1",
            [credential],
        )
        .unwrap();
    }
    drop(c);
    assert!(f.store.try_native_media_read(&origin, None).is_err());
    assert!(f.store.try_native_connection_current(&origin).unwrap());
    let (proof, plan) = admission(&f, &request, &origin);
    let renewed = plan.targets()[0].next().clone();
    let attempt = f
        .store
        .prepare_native_credential_refresh(&plan, &proof, "signal-only-metadata-rotation")
        .unwrap();
    f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    let receipt = attempt
        .applied(BTreeMap::from([(origin.offer_record_id().into(), 2)]))
        .unwrap();
    f.store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    drop(proof);
    assert_eq!(renewed.credential_epoch(), 1);
    assert!(f.store.try_native_media_read(&renewed, None).is_err());
    assert!(f.store.try_native_connection_current(&renewed).unwrap());
    assert!(f.store.try_native_connection_current(&origin).is_err());
    let mut signal = request;
    signal.credentials = vec![next.clone()];
    signal.command = PeerCommand::NativeStream(connect_protocol::native_streams::Intent {
        operation_id: "signal-only-after-rotation".into(),
        conversation_id: renewed.conversation_id().into(),
        session_id: renewed.session_id().into(),
        connection_id: renewed.connection_id().into(),
        expected_revision: Some(1),
        by_participant: None,
        command: connect_protocol::native_streams::Command::Dtmf,
        payload: json!({"digits":"9#","duration_ms":100,"method":"info"}),
    });
    let proof = admit(&f, &signal).unwrap();
    assert_eq!(
        f.store
            .project_native_stream_with_peer(&next, &proof)
            .unwrap()
            .parent_record_id,
        origin.offer_record_id()
    );
    drop(proof);
    let admin = f.store.revalidate(&f.admin).unwrap();
    f.store.revoke(&admin, &next).unwrap();
    assert!(f.store.try_native_connection_current(&renewed).is_err());
}
