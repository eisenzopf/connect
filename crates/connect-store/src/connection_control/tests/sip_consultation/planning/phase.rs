//! Custodian metadata only; no physical call, IO permit or reader is fabricated.
use super::*;
use crate::PrivateInitialTelephonePhase::*;

impl Scene {
    pub(super) fn phase(&self) -> Result<crate::PrivateInitialTelephonePhase> {
        self.f.store.private_initial_telephone_phase(&self.phone)
    }
}

#[test]
fn private_sip_phase_tracks_original_claim_application_and_sealed_lifetime_without_idle_readers() {
    let s = Scene::with_controller(Some("phase-controller"));
    assert_eq!(s.phase().unwrap(), Quarantined);
    let public = s.f.store.session_ready_origins.lock().unwrap().len();
    let receipt = s.applied();
    assert_eq!(s.phase().unwrap(), Prepared);
    assert!(s
        .f
        .store
        .try_private_initial_telephone_admission_read(s.phone.clone())
        .is_err());
    assert!(s.packets().is_err());
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert_eq!(s.phase().unwrap(), Applied);
    assert!(s.packets().unwrap().current());
    s.f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.root.credential],
        )
        .unwrap();
    assert_eq!(s.phase().unwrap(), Established);
    let writer = [AuthorityScope::conversation(&s.f.cid, AuthorityMode::Write).unwrap()];
    drop(s.f.store.try_acquire_authority(&writer).unwrap());
    assert_eq!(
        s.f.store.session_ready_origins.lock().unwrap().len(),
        public
    );
    assert_eq!(s.f.count("initial_telephone_operations"), 1);
}

#[test]
fn private_sip_phase_keeps_presence_after_media_withdrawal_but_current_route_still_required() {
    let s = Scene::with_controller(Some("phase-purpose"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    for key in [&s.f.keys[0], &s.policy.endpoint_credential_id] {
        let mut operations = s.f.store.revalidate(key).unwrap().scope.operations;
        operations.retain(|p| p != "connection.offer" && p != "stream.subscribe");
        s.f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![key, serde_json::to_string(&operations).unwrap()],
            )
            .unwrap();
    }
    assert!(s.packets().is_err());
    assert_eq!(s.phase().unwrap(), Established);
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.policy.route_credential_id],
        )
        .unwrap();
    assert!(s.phase().is_err());
}

#[test]
fn private_sip_phase_rechecks_original_physical_authority_and_pending_controller() {
    for sealed in [false, true] {
        for fault in 0..5 {
            let s = Scene::with_controller(Some("phase-physical"));
            let receipt = s.captured();
            if sealed {
                s.f.store
                    .complete_native_consultation(&receipt)
                    .unwrap()
                    .unwrap();
            }
            let (sql, key) = match fault {
                0 => (
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    s.root.credential.as_str(),
                ),
                1 => (
                    "UPDATE devices SET revoked=1 WHERE id=?1",
                    "private-sip-device",
                ),
                2 => (
                    "UPDATE credentials SET recipients='[]' WHERE id=?1",
                    s.policy.route_credential_id.as_str(),
                ),
                3 => (
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    s.phone.connection_id(),
                ),
                _ => (
                    "UPDATE participants SET active=0 WHERE participant_id=?1",
                    s.f.pids[2].as_str(),
                ),
            };
            s.f.store.connection().unwrap().execute(sql, [key]).unwrap();
            assert_eq!(
                s.phase().is_ok(),
                sealed && fault == 0,
                "sealed={sealed}, fault={fault}"
            );
        }
    }
}

#[test]
fn private_sip_phase_cannot_bootstrap_original_call_in_another_runtime() {
    let s = Scene::with_controller(Some("phase-backup"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    let path = s.f._dir.path().join("phase-backup.sqlite3");
    s.f.store.backup(&path).unwrap();
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert!(restored.private_initial_telephone_phase(&s.phone).is_err());
    assert_eq!(s.phase().unwrap(), Established);
}
