//! Controlled packet authority; these tests do not send physical RTP.
use super::*;

impl Scene {
    pub(super) fn packets(&self) -> Result<crate::InitialTelephoneMediaRead> {
        self.f
            .store
            .try_private_initial_telephone_media_read(self.phone.clone())
    }
    pub(super) fn packet_pair(&self, native: usize) -> Result<crate::InitialTelephoneNativeRead> {
        self.f
            .store
            .try_initial_telephone_native_read(self.phone.clone(), self.native[native].clone())
    }
}
fn sealed() -> Scene {
    let s = Scene::with_controller(Some("packet-controller"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    s
}

#[test]
fn private_sip_packets_require_recorded_application_and_never_register_public_readiness() {
    let s = Scene::with_controller(Some("packet-application"));
    let public = s.f.store.session_ready_origins.lock().unwrap().len();
    assert!(s.read().unwrap().current()); // Planning is allowed before packets.
    assert!(s.packets().is_err());
    assert!(s.packet_pair(0).is_err());
    let receipt = s.applied();
    assert!(s.packets().is_err()); // A Core receipt must first be recorded.
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    let read = s.packets().unwrap();
    assert!(read.current() && read.is_private_audio() && !read.is_private_admission());
    assert!(read.may_receive());
    drop(read);
    assert!(s.packet_pair(0).unwrap().current());
    for other in [1, 2] {
        assert!(s.packet_pair(other).is_err());
    }
    assert!(s
        .f
        .store
        .try_initial_telephone_media_read(s.phone.clone())
        .is_err());
    assert_eq!(
        s.f.store.session_ready_origins.lock().unwrap().len(),
        public
    );
}

#[test]
fn private_sip_packets_survive_ended_controller_only_after_sealing() {
    let s = Scene::with_controller(Some("packet-lifetime"));
    let receipt = s.captured();
    assert!(s.packets().unwrap().current());
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.root.credential],
        )
        .unwrap();
    assert!(s.packets().is_err());
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    assert!(s.packets().unwrap().current());
    assert!(s.packet_pair(0).unwrap().current());
    assert!(s
        .f
        .store
        .try_private_initial_telephone_admission_read(s.phone.clone())
        .is_err());
}

#[test]
fn private_sip_packets_require_current_media_rights_and_physical_authority() {
    for fault in 0..12 {
        let s = sealed();
        assert!(s.packets().unwrap().current());
        let (sql, key) = match fault {
            0 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            1 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            3 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            4 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            5 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            6 => (
                "UPDATE connections SET generation=generation+1 WHERE id=?1",
                s.phone.connection_id(),
            ),
            7 => (
                "UPDATE connections SET observed_runtime_id='foreign' WHERE id=?1",
                s.phone.connection_id(),
            ),
            8 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                s.policy.endpoint_id.as_str(),
            ),
            9 => (
                "UPDATE devices SET revoked=1 WHERE id=?1",
                "private-sip-device",
            ),
            10 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                s.f.pids[2].as_str(),
            ),
            _ => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[1].as_str(),
            ),
        };
        s.f.store.connection().unwrap().execute(sql, [key]).unwrap();
        assert!(s.packets().is_err(), "fault {fault}");
        assert!(s.packet_pair(0).is_err(), "pair fault {fault}");
    }
}

#[test]
fn private_sip_packet_reader_requires_complete_interval_until_last_drop() {
    let s = sealed();
    let route =
        s.f.store
            .revalidate(&s.policy.route_credential_id)
            .unwrap()
            .identity_id;
    let target =
        s.f.store
            .revalidate(&s.policy.endpoint_credential_id)
            .unwrap()
            .identity_id;
    let partial =
        s.f.store
            .try_acquire_authority(&[
                AuthorityScope::conversation(&s.f.cid, AuthorityMode::Read).unwrap(),
                AuthorityScope::identity(&route, AuthorityMode::Read).unwrap(),
                AuthorityScope::identity(&target, AuthorityMode::Read).unwrap(),
            ])
            .unwrap();
    assert!(s
        .f
        .store
        .private_initial_telephone_media_in(
            &s.f.store.connection().unwrap(),
            s.phone.clone(),
            partial,
            crate::now() + 5
        )
        .is_err());
    let read = Arc::new(s.packets().unwrap());
    let clone = read.clone();
    drop(read);
    let writers = [AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()];
    assert!(s.f.store.try_acquire_authority(&writers).is_err());
    drop(clone);
    drop(s.f.store.try_acquire_authority(&writers).unwrap());
}

#[test]
fn captured_private_sip_io_requires_live_ai_provider_before_and_after_sealing() {
    let s = Scene::with_controller(Some("packet-provider"));
    s.grant_signal_permissions();
    let receipt = s.captured();
    assert!(s.packets().unwrap().current());
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE participants SET kind='ai' WHERE participant_id=?1",
        [&s.f.pids[2]],
    )
    .unwrap();
    drop(c);
    assert!(s.packets().is_err());
    assert!(s
        .signal(0, crate::TelephoneSignalDirection::FromTelephone)
        .is_err());
    // Sealing cannot substitute for missing provider readiness either.
    assert!(s
        .f
        .store
        .complete_native_consultation(&receipt)
        .unwrap()
        .is_none());
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET kind='human' WHERE participant_id=?1",
            [&s.f.pids[2]],
        )
        .unwrap();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET kind='ai' WHERE participant_id=?1",
            [&s.f.pids[2]],
        )
        .unwrap();
    assert!(s.packets().is_err());
    assert!(s
        .signal(0, crate::TelephoneSignalDirection::FromTelephone)
        .is_err());
}
