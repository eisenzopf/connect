//! Controlled metadata custody. Physical quarantine is qualified by server tests.
use super::*;
fn candidate(f: &Fixture, origin: &crate::NativeConsultationOrigin) -> PrepareRequest {
    let mut request = f.request(2, "private-candidate");
    request.intent.offer["private_consultation"] =
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()});
    request
}
#[test]
fn private_native_leg_registration_precedes_allocation_and_pins_the_exact_original_context() {
    let (f, _, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    let before = (
        f.count("connections"),
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    assert_eq!(f.count("native_consultation_legs"), 1);
    assert_eq!(
        before,
        (
            f.count("connections"),
            f.count("operations"),
            f.count("operation_effects"),
            f.count("journal")
        )
    );
    let mut changed = request.clone();
    changed.intent.offer["extra"] = json!(true);
    assert!(f
        .store
        .register_native_consultation_leg(&origin, changed.intent, &request.endpoint_id)
        .is_err());
    assert!(f
        .store
        .register_native_consultation_leg(&origin, request.intent.clone(), &f.endpoints[1])
        .is_err());
    let mut av = request.clone();
    av.intent.offer["streams_offered"][0]["kind"] = json!("video");
    assert!(f
        .store
        .register_native_consultation_leg(&origin, av.intent, &request.endpoint_id)
        .is_err());
    let c = f.store.connection().unwrap();
    assert!(c
        .execute("DELETE FROM native_consultation_legs", [])
        .is_err());
    assert!(c
        .execute(
            "UPDATE native_consultation_legs SET endpoint_id=endpoint_id",
            []
        )
        .is_err());
}
#[test]
fn private_native_leg_preparation_hides_candidate_metadata_from_remote_and_unselected_members() {
    let (f, _, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    let mut unregistered = request.clone();
    unregistered.intent.connection_id = "unregistered-private".into();
    assert!(f.apply(2, unregistered).is_err());
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    f.apply(2, request.clone()).unwrap();
    for index in 0..4 {
        let snapshot = f
            .store
            .conversation_snapshot(&f.keys[index], &f.cid, Default::default())
            .unwrap();
        let events = f
            .store
            .conversation_events(&f.keys[index], &f.cid, Default::default())
            .unwrap();
        let raw = serde_json::to_string(&(snapshot, events)).unwrap();
        assert_eq!(
            raw.contains(&request.intent.connection_id),
            index == 0 || index == 2,
            "identity {index}"
        );
    }
    let c = f.store.connection().unwrap();
    assert!(super::super::private_consultation::observation_allowed(
        &c,
        &request.intent.connection_id,
        &[(f.pids[0].clone(), 1), (f.pids[2].clone(), 1)]
    )
    .unwrap());
    assert!(!super::super::private_consultation::observation_allowed(
        &c,
        &request.intent.connection_id,
        &[(f.pids[1].clone(), 1)]
    )
    .unwrap());
    assert!(!super::super::private_consultation::observation_allowed(
        &c,
        &request.intent.connection_id,
        &[(f.pids[2].clone(), 2)]
    )
    .unwrap());
}
#[test]
fn private_native_leg_refuses_changed_parent_authority_and_cannot_convert_an_existing_public_slot()
{
    let (f, sources, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    let mut old = request.clone();
    old.intent.connection_id = sources[2].connection_id().into();
    assert!(f
        .store
        .register_native_consultation_leg(&origin, old.intent, &request.endpoint_id)
        .is_err());
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    assert!(f.apply(2, request.clone()).is_err());
    assert!(f
        .store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .is_err());
    assert_eq!(f.count("connections"), 3);
}
#[test]
fn private_native_leg_fences_signed_candidate_and_physical_context_receive_views() {
    let (f, sources, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    let signed = crate::PeerAuthorityRequest {
        credentials: vec![f.keys[2].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-2".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-2".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(request.intent.clone()),
    };
    let proof = f.admit(&signed);
    f.store
        .prepare_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap()
        .unwrap();
    let candidate = f
        .store
        .native_media_origin_with_peer(&f.keys[2], &proof, &lease)
        .unwrap();
    let read = f.store.try_native_media_read(&candidate, None).unwrap();
    assert_eq!(
        read.target().private_consultation.as_deref(),
        Some(origin.record_id())
    );
    assert_eq!(read.target().endpoint_capabilities, vec!["audio"]);
    drop(read);
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[0]))
        .is_ok());
    // The old public context of the very same logical Participant is excluded.
    assert!(f
        .store
        .try_native_media_read(&candidate, Some(&sources[2]))
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[2], Some(&candidate))
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&sources[1], Some(&candidate))
        .is_err());
}

#[test]
fn private_native_leg_has_one_reserved_candidate_and_fences_changed_endpoint_revision() {
    let (f, _, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    let mut second = request.clone();
    second.intent.connection_id = "another-private-candidate".into();
    second.intent.operation_id = "another-private-offer".into();
    assert!(f
        .store
        .register_native_consultation_leg(&origin, second.intent, &request.endpoint_id)
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
            [&request.endpoint_id],
        )
        .unwrap();
    assert!(f
        .store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .is_err());
    assert!(f.apply(2, request).is_err());
    assert_eq!(f.count("native_consultation_legs"), 1);
    assert_eq!(f.count("connections"), 3);
}
#[test]
fn private_native_leg_parent_cancellation_refuses_later_candidate_preparation_and_observation_source(
) {
    let (f, _, command, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    f.store
        .execute_operation_action(
            &f.keys[0],
            connect_protocol::transitions::OperationAction::Cancel {
                target_operation_id: command.operation_id,
                operation_id: "cancel-registered-leg".into(),
                reason: None,
            },
        )
        .unwrap();
    assert!(f.apply(2, request.clone()).is_err());
    let c = f.store.connection().unwrap();
    assert!(
        !super::super::private_consultation::observation_source_current(
            &f.store,
            &c,
            &request.intent.connection_id
        )
        .unwrap()
    );
    drop(c);
    assert_eq!(f.count("connections"), 3);
}

#[test]
fn private_native_leg_owned_intervals_cannot_outlive_the_original_consultation_source() {
    let (f, _, _, origin) = consultation_reads::fixture();
    let request = candidate(&f, &origin);
    f.store
        .register_native_consultation_leg(&origin, request.intent.clone(), &request.endpoint_id)
        .unwrap();
    let expiry = crate::now() + 40;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.keys[0], expiry],
        )
        .unwrap();
    let signed = crate::PeerAuthorityRequest {
        credentials: vec![f.keys[2].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-2".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-2".into(),
        algorithm: "EdDSA".into(),
        command: crate::PeerCommand::NativeConnectionOffer(request.intent.clone()),
    };
    let proof = f.admit(&signed);
    assert_eq!(proof.expires_at(), expiry);
    f.store
        .prepare_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[2], &proof)
        .unwrap()
        .unwrap();
    let candidate = f
        .store
        .native_media_origin_with_peer(&f.keys[2], &proof, &lease)
        .unwrap();
    drop(proof);
    // Credential expiry can narrow independently of the already issued native
    // origin; fresh reads must clip again rather than rely on its old deadline.
    let narrowed = crate::now() + 5;
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![f.keys[0], narrowed],
        )
        .unwrap();
    let read = f.store.try_native_media_read(&candidate, None).unwrap();
    assert_eq!(read.expires_at(), narrowed);
    assert!(read.current());
}
