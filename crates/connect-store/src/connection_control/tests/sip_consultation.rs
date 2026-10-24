//! Controlled reservation and authorization evidence only; no physical SIP IO.
mod initial;
mod planning;
use super::super::private_consultation as private;
use super::*;
use crate::{AuthorityMode, AuthorityScope, InitialTelephonePolicy, NativeConsultationOrigin};

fn fixture() -> (Fixture, NativeConsultationOrigin, InitialTelephonePolicy) {
    let (f, _sources, origin, policy) = fixture_with_sources();
    (f, origin, policy)
}
fn fixture_with_sources() -> (
    Fixture,
    Vec<std::sync::Arc<crate::NativeMediaOrigin>>,
    NativeConsultationOrigin,
    InitialTelephonePolicy,
) {
    let (f, sources, _, origin) = consultation_reads::fixture();
    let target = f.store.revalidate(&f.keys[2]).unwrap().identity_id;
    let route = f.store.revalidate(&f.keys[3]).unwrap().identity_id;
    let route_admin = f
        .store
        .issue(&route, Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .store
        .execute_provisioning(
            &route_admin,
            ProvisioningCommand::Endpoint {
                operation_id: "private-sip-endpoint".into(),
                identity_id: target,
                device: DeviceSpec {
                    id: "private-sip-device".into(),
                    kind: "telephone".into(),
                    platform: "controlled-store-fixture".into(),
                },
                transport: EndpointTransport::Telephone,
                address: "+14155550123".into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("private-sip-route".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute_provisioning(
            &route_admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: "verify-private-sip-endpoint".into(),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "controlled-store-fixture".into(),
            },
        )
        .unwrap();
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope.operations.push("sip_dial".into());
    let route_credential = f.store.issue(&route, scope, crate::now() + 600).unwrap().id;
    let policy = InitialTelephonePolicy {
        endpoint_id,
        endpoint_credential_id: f.keys[2].clone(),
        route_credential_id: route_credential,
        route_reference: "private-sip-route".into(),
        sip_authority: "127.0.0.1:5099".into(),
        timeout_seconds: 30,
    };
    (f, sources, origin, policy)
}

fn counts(f: &Fixture) -> Vec<u64> {
    [
        "operations",
        "operation_effects",
        "journal",
        "connection_offers",
        "initial_telephone_operations",
        "native_connection_lifetimes",
        "sip_legs",
    ]
    .iter()
    .map(|name| f.count(name))
    .collect()
}

#[test]
fn sip_reservation_is_fresh_metadata_with_stable_replay_and_no_idle_readers_or_call() {
    let (f, origin, policy) = fixture();
    let before = counts(&f);
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap();
    assert_eq!(reservation.consultation_record_id(), origin.record_id());
    assert_eq!(reservation.participant_id(), f.pids[2]);
    assert_eq!(f.count("connections"), 4);
    assert_eq!(counts(&f), before);
    f.store
        .validate_sip_consultation_reservation(&reservation)
        .unwrap();
    let replay = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap();
    assert_eq!(replay.connection_id(), reservation.connection_id());
    assert_eq!(f.count("sip_consultation_reservations"), 1);
    let mut changed = policy.clone();
    changed.sip_authority = "127.0.0.1:5098".into();
    assert!(f
        .store
        .reserve_sip_consultation_leg(&origin, changed)
        .is_err());
    for key in [&f.keys[0], &f.keys[2], &policy.route_credential_id] {
        let actor = f.store.revalidate(key).unwrap();
        drop(
            f.store
                .try_acquire_authority(&[AuthorityScope::identity(
                    &actor.identity_id,
                    AuthorityMode::Write,
                )
                .unwrap()])
                .unwrap(),
        );
    }
    drop(
        f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&f.cid, AuthorityMode::Write).unwrap()
            ])
            .unwrap(),
    );
    let row: (String, u64, Option<String>, Option<String>) = f.store.connection().unwrap().query_row(
        "SELECT state,generation,observed_runtime_id,readiness_json FROM connections WHERE id=?1", [reservation.connection_id()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row, ("preparing".into(), 1, None, None));
    assert_eq!(counts(&f), before);
}

#[test]
fn sip_reservation_hides_metadata_and_never_asserts_a_current_physical_source() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
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
            raw.contains(reservation.connection_id()),
            index == 0 || index == 2,
            "identity {index}"
        );
    }
    let c = f.store.connection().unwrap();
    assert!(private::observation_allowed(
        &c,
        reservation.connection_id(),
        &[(f.pids[0].clone(), 1), (f.pids[2].clone(), 1)]
    )
    .unwrap());
    for selected in [&[(f.pids[1].clone(), 1)][..], &[(f.pids[2].clone(), 2)][..]] {
        assert!(!private::observation_allowed(&c, reservation.connection_id(), selected).unwrap());
    }
    assert!(
        !private::observation_source_current(&f.store, &c, reservation.connection_id()).unwrap()
    );
}

#[test]
fn sip_reservation_cannot_be_borrowed_by_public_or_private_native_offers() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
    let before = counts(&f);
    let mut offer = f.request(2, "borrowed-private-sip");
    offer.intent.connection_id = reservation.connection_id().into();
    assert!(f.apply(2, offer.clone()).is_err());
    offer.intent.offer["private_consultation"] =
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()});
    assert!(f
        .store
        .register_native_consultation_leg(&origin, offer.intent.clone(), &offer.endpoint_id)
        .is_err());
    assert!(f.apply(2, offer.clone()).is_err());
    offer.intent.connection_id = "other-native-private-candidate".into();
    assert!(f
        .store
        .register_native_consultation_leg(&origin, offer.intent, &offer.endpoint_id)
        .is_err());
    assert_eq!(f.count("native_consultation_legs"), 0);
    // Refusal retains the public command receipt, without allocating an offer,
    // creating an effect or exposing a journal event.
    let after = counts(&f);
    assert_eq!(after[0], before[0] + 1);
    assert_eq!(after[1..], before[1..]);
}

#[test]
fn native_candidate_prevents_a_second_sip_candidate_without_partial_reservation() {
    let (f, origin, policy) = fixture();
    let mut offer = f.request(2, "native-before-sip");
    offer.intent.offer["private_consultation"] =
        json!({"profile":"connect-private-audio-v1","operation_record_id":origin.record_id()});
    f.store
        .register_native_consultation_leg(&origin, offer.intent, &offer.endpoint_id)
        .unwrap();
    assert!(f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .is_err());
    assert_eq!(f.count("connections"), 3);
    assert_eq!(f.count("sip_consultation_reservations"), 0);
}

#[test]
fn sip_reservation_rechecks_each_authority_and_exact_endpoint_and_epoch() {
    for fault in 0..16 {
        let (f, origin, policy) = fixture();
        let reservation = f
            .store
            .reserve_sip_consultation_leg(&origin, policy.clone())
            .unwrap();
        let c = f.store.connection().unwrap();
        let (sql, id) = match fault {
            0 => ("UPDATE credentials SET revoked=1 WHERE id=?1",f.keys[0].as_str()),
            1 => ("UPDATE credentials SET revoked=1 WHERE id=?1",policy.endpoint_credential_id.as_str()),
            2 => ("UPDATE credentials SET revoked=1 WHERE id=?1",policy.route_credential_id.as_str()),
            3 => ("UPDATE credentials SET operations='[]' WHERE id=?1",policy.endpoint_credential_id.as_str()),
            4 => ("UPDATE credentials SET operations='[]' WHERE id=?1",policy.route_credential_id.as_str()),
            5 => ("UPDATE credentials SET recipients='[]' WHERE id=?1",policy.endpoint_credential_id.as_str()),
            6 => ("UPDATE credentials SET recipients='[]' WHERE id=?1",policy.route_credential_id.as_str()),
            7 => ("UPDATE endpoints SET revision=revision+1 WHERE id=?1",policy.endpoint_id.as_str()),
            8 => ("UPDATE endpoints SET verification_state='revoked' WHERE id=?1",policy.endpoint_id.as_str()),
            9 => ("UPDATE devices SET revoked=1 WHERE id=?1","private-sip-device"),
            10 => ("UPDATE identities SET revoked=1 WHERE id=(SELECT identity_id FROM credentials WHERE id=?1)",policy.route_credential_id.as_str()),
            11 => ("UPDATE participants SET active=0 WHERE participant_id=?1",f.pids[2].as_str()),
            12 => ("UPDATE session_participants SET state='left' WHERE participant_id=?1",f.pids[2].as_str()),
            13 => ("UPDATE connections SET generation=generation+1 WHERE id=?1",reservation.connection_id()),
            14 => ("UPDATE connections SET observed_runtime_id='foreign-runtime' WHERE id=?1",reservation.connection_id()),
            _ => ("UPDATE operations SET cancel_requested=1 WHERE record_id=?1",origin.record_id()),
        };
        c.execute(sql, [id]).unwrap();
        drop(c);
        assert!(
            f.store
                .validate_sip_consultation_reservation(&reservation)
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(f.count("sip_consultation_reservations"), 1);
        assert_eq!(f.count("initial_telephone_operations"), 0);
    }
}

#[test]
fn sip_reservation_capacity_refusal_and_unused_retirement_do_not_leak_slots() {
    let (mut f, origin, policy) = fixture();
    f.store.limits.connections = 3;
    let before = counts(&f);
    let error = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap_err();
    assert_eq!(error.code, "capacity-exceeded");
    assert_eq!(f.count("connections"), 3);
    assert_eq!(f.count("sip_consultation_reservations"), 0);
    f.store.limits.connections = 4;
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap();
    f.store
        .retire_unused_sip_consultation_reservation(&reservation)
        .unwrap();
    let used: u64 = f.store.connection().unwrap().query_row("SELECT count(*) FROM connections WHERE state NOT IN ('retired','failed','interrupted')",[],|r|r.get(0)).unwrap();
    assert_eq!(used, 3);
    let replay = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
    assert_eq!(replay.connection_id(), reservation.connection_id());
    assert!(f
        .store
        .validate_sip_consultation_reservation(&replay)
        .is_err());
    assert_eq!(counts(&f), before);
}

#[test]
fn sip_reservation_original_scope_must_authorize_both_private_identities() {
    for endpoint in [true, false] {
        for allowed in [0, 2] {
            // An Endpoint credential implicitly permits its own identity. A
            // ceiling naming the primary therefore authorizes both private peers.
            if endpoint && allowed == 0 {
                continue;
            }
            let (f, origin, policy) = fixture();
            let identity = f.store.revalidate(&f.keys[allowed]).unwrap().identity_id;
            let key = if endpoint {
                &policy.endpoint_credential_id
            } else {
                &policy.route_credential_id
            };
            f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE credentials SET recipients=?2 WHERE id=?1",
                    params![key, json!([identity]).to_string()],
                )
                .unwrap();
            assert!(f
                .store
                .reserve_sip_consultation_leg(&origin, policy)
                .is_err());
            assert_eq!(f.count("connections"), 3);
            assert_eq!(f.count("sip_consultation_reservations"), 0);
        }
    }
}

#[test]
fn sip_reservation_is_immutable_and_retirement_survives_revocation_without_resurrection() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap();
    let before = counts(&f);
    let c = f.store.connection().unwrap();
    assert!(c
        .execute(
            "UPDATE sip_consultation_reservations SET runtime_epoch=runtime_epoch",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM sip_consultation_reservations", [])
        .is_err());
    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
        .unwrap();
    drop(c);
    assert!(f
        .store
        .validate_sip_consultation_reservation(&reservation)
        .is_err());
    f.store
        .retire_unused_sip_consultation_reservation(&reservation)
        .unwrap();
    f.store
        .retire_unused_sip_consultation_reservation(&reservation)
        .unwrap();
    let c = f.store.connection().unwrap();
    let state: (String, u64) = c
        .query_row(
            "SELECT state,revision FROM connections WHERE id=?1",
            [reservation.connection_id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), 1));
    drop(c);
    assert!(f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .is_err());
    assert_eq!(counts(&f), before);
}

#[test]
fn sip_reservation_cleanup_refuses_foreign_owner_or_observed_physical_custody() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
    let foreign = Fixture::new();
    assert!(foreign
        .store
        .validate_sip_consultation_reservation(&reservation)
        .is_err());
    assert!(foreign
        .store
        .retire_unused_sip_consultation_reservation(&reservation)
        .is_err());
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE connections SET observed_runtime_id='controlled-observation' WHERE id=?1",
        [reservation.connection_id()],
    )
    .unwrap();
    drop(c);
    assert!(f
        .store
        .retire_unused_sip_consultation_reservation(&reservation)
        .is_err());
    let state: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT state FROM connections WHERE id=?1",
            [reservation.connection_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "preparing");
}

#[test]
fn sip_reservation_cannot_rebind_to_a_later_membership_epoch() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE participants SET active=0 WHERE participant_id=?1",
        [&f.pids[2]],
    )
    .unwrap();
    c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",[&f.pids[2]]).unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.keys[2]).unwrap();
    assert!(!private::visible(&c, &actor, reservation.connection_id()).unwrap());
    drop(c);
    assert!(f
        .store
        .validate_sip_consultation_reservation(&reservation)
        .is_err());
    f.store
        .retire_unused_sip_consultation_reservation(&reservation)
        .unwrap();
}

#[test]
fn sip_reservation_canonical_connection_and_route_owner_cannot_be_rewritten() {
    let (f, origin, policy) = fixture();
    let reservation = f
        .store
        .reserve_sip_consultation_leg(&origin, policy.clone())
        .unwrap();
    let c = f.store.connection().unwrap();
    for sql in [
        "UPDATE connections SET endpoint_id=(SELECT id FROM endpoints WHERE device_id='device-2') WHERE id=?1",
        "UPDATE connections SET membership_epoch=membership_epoch+1 WHERE id=?1",
        "UPDATE connections SET session_id='another-session' WHERE id=?1",
    ] {
        assert!(c.execute(sql,[reservation.connection_id()]).is_err());
    }
    assert!(c
        .execute(
            "UPDATE endpoints SET secret_ref='changed-route' WHERE id=?1",
            [&policy.endpoint_id]
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE endpoints SET route_owner_id=identity_id WHERE id=?1",
            [&policy.endpoint_id]
        )
        .is_err());
    drop(c);
    f.store
        .validate_sip_consultation_reservation(&reservation)
        .unwrap();
}

#[test]
fn sip_reservation_refuses_unknown_physical_custody_before_creating_a_slot() {
    let (f, origin, policy) = fixture();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='unknown' WHERE participant_id=?1",
            [&f.pids[2]],
        )
        .unwrap();
    let before = counts(&f);
    assert!(f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .is_err());
    assert_eq!(f.count("connections"), 3);
    assert_eq!(f.count("sip_consultation_reservations"), 0);
    assert_eq!(counts(&f), before);
}

#[test]
fn protected_target_reports_its_original_sip_candidate_without_native_registration_or_io() {
    let (f, origin, policy) = fixture();
    let before = f.store.native_consultation_target(&origin).unwrap();
    assert!(!before.registered && before.candidate_kind.is_none());
    let reserved = f
        .store
        .reserve_sip_consultation_leg(&origin, policy)
        .unwrap();
    let target = f.store.native_consultation_target(&origin).unwrap();
    assert!(target.registered);
    assert_eq!(target.candidate_kind.as_deref(), Some("sip"));
    assert_eq!(target.participant_id, reserved.participant_id());
    assert_eq!(target.identity_id, before.identity_id);
    assert_eq!(f.count("native_consultation_legs"), 0);
    assert_eq!(f.count("initial_telephone_operations"), 0);
}
