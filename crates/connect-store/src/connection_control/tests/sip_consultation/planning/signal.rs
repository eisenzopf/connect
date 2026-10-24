//! Controlled current signal readers and immutable receive custody, not wire IO.
use super::*;
use crate::{SipSignalOrigin, TelephoneSignalDirection};
use TelephoneSignalDirection::{FromTelephone, ToTelephone};

impl Scene {
    pub(super) fn grant_signal_permissions(&self) {
        for key in [&self.f.keys[0], &self.policy.endpoint_credential_id] {
            let mut operations = self.f.store.revalidate(key).unwrap().scope.operations;
            operations.extend(["dtmf.send".into(), "dtmf.receive".into()]);
            self.f
                .store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    params![key, serde_json::to_string(&operations).unwrap()],
                )
                .unwrap();
        }
    }
    pub(super) fn signal(
        &self,
        native: usize,
        direction: TelephoneSignalDirection,
    ) -> Result<crate::SipNativeSignalRead> {
        self.f.store.try_sip_native_signal_read(
            &SipSignalOrigin::Initial(self.phone.clone()),
            self.native[native].clone(),
            direction,
        )
    }
}
fn sealed() -> Scene {
    let s = Scene::with_controller(Some("private-sip-signal"));
    s.grant_signal_permissions();
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    s
}
fn without(s: &Scene, key: &str, removed: &[&str]) -> Vec<String> {
    let original = s.f.store.revalidate(key).unwrap().scope.operations;
    let operations: Vec<_> = original
        .iter()
        .filter(|p| !removed.contains(&p.as_str()))
        .collect();
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![key, serde_json::to_string(&operations).unwrap()],
        )
        .unwrap();
    original
}

#[test]
fn private_sip_signal_requires_applied_pair_then_sealed_current_physical_lifetime() {
    let s = Scene::with_controller(Some("private-signal-pending"));
    s.grant_signal_permissions();
    let phone = SipSignalOrigin::Initial(s.phone.clone());
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.f.store.try_sip_signal_read(&phone, direction).is_err());
        assert!(s.signal(0, direction).is_err());
    }
    let receipt = s.captured();
    for direction in [FromTelephone, ToTelephone] {
        assert!(s
            .f
            .store
            .try_sip_signal_read(&phone, direction)
            .unwrap()
            .current());
        assert!(s.signal(0, direction).unwrap().current());
        for other in [1, 2] {
            assert!(s.signal(other, direction).is_err());
        }
    }
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.root.credential],
        )
        .unwrap();
    assert!(s.signal(0, FromTelephone).is_err());
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    for direction in [FromTelephone, ToTelephone] {
        let read = s.signal(0, direction).unwrap();
        assert!(read.current());
        assert_eq!(
            read.native().target().connection.connection_id,
            s.native[0].connection_id()
        );
        assert_eq!(read.member().participant_id, s.f.pids[2]);
    }
    assert!(s
        .f
        .store
        .try_initial_telephone_signal_read(
            s.phone.clone(),
            crate::InitialTelephoneSignalDirection::FromTelephone
        )
        .is_err());
    assert!(s
        .f
        .store
        .try_initial_telephone_media_read(s.phone.clone())
        .is_err());
}

#[test]
fn private_sip_signal_independent_named_directions_withdraw_and_recover() {
    for (native, permission, denied, allowed) in [
        (true, "dtmf.receive", FromTelephone, ToTelephone),
        (true, "dtmf.send", ToTelephone, FromTelephone),
        (false, "dtmf.send", FromTelephone, ToTelephone),
        (false, "dtmf.receive", ToTelephone, FromTelephone),
    ] {
        let s = sealed();
        let key = if native {
            &s.f.keys[0]
        } else {
            &s.policy.endpoint_credential_id
        };
        let original = without(&s, key, &[permission]);
        assert!(s.signal(0, denied).is_err());
        assert!(s.signal(0, allowed).unwrap().current());
        s.f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![key, serde_json::to_string(&original).unwrap()],
            )
            .unwrap();
        for direction in [FromTelephone, ToTelephone] {
            assert!(s.signal(0, direction).unwrap().current());
        }
    }
}

#[test]
fn private_sip_signal_survives_media_right_and_codec_preference_withdrawal_without_media_grant() {
    let s = sealed();
    for key in [&s.f.keys[0], &s.policy.endpoint_credential_id] {
        without(&s, key, &["connection.offer", "stream.subscribe"]);
    }
    let c = s.f.store.connection().unwrap();
    for pid in [&s.f.pids[0], &s.f.pids[2]] {
        c.execute(
            "UPDATE session_presence_capabilities SET capabilities_json=?2 WHERE participant_id=?1",
            params![pid, json!({"audio":["g.711-a"]}).to_string()],
        )
        .unwrap();
    }
    drop(c);
    for direction in [FromTelephone, ToTelephone] {
        assert!(s.signal(0, direction).unwrap().current());
        assert!(s
            .f
            .store
            .try_sip_signal_read(&SipSignalOrigin::Initial(s.phone.clone()), direction)
            .unwrap()
            .current());
    }
    assert!(s.f.store.try_native_media_read(&s.native[0], None).is_err());
    assert!(s
        .f
        .store
        .try_initial_telephone_media_read(s.phone.clone())
        .is_err());
}

#[test]
fn private_sip_signal_rechecks_current_original_physical_principals_and_ceilings() {
    for fault in 0..10 {
        let s = sealed();
        let c = s.f.store.connection().unwrap();
        let (sql, id) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            1 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            3 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            4 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            5 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                s.policy.endpoint_id.as_str(),
            ),
            6 => (
                "UPDATE devices SET revoked=1 WHERE id=?1",
                "private-sip-device",
            ),
            7 => (
                "UPDATE connections SET generation=generation+1 WHERE id=?1",
                s.phone.connection_id(),
            ),
            8 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                s.f.pids[2].as_str(),
            ),
            _ => (
                "UPDATE connections SET observed_runtime_id='foreign' WHERE id=?1",
                s.phone.connection_id(),
            ),
        };
        c.execute(sql, [id]).unwrap();
        drop(c);
        for direction in [FromTelephone, ToTelephone] {
            assert!(s.signal(0, direction).is_err(), "fault {fault}");
            assert!(
                s.f.store
                    .try_sip_signal_read(&SipSignalOrigin::Initial(s.phone.clone()), direction)
                    .is_err(),
                "fault {fault}"
            );
        }
    }
}

#[test]
fn private_sip_signal_owns_complete_interval_and_rejects_partial_coverage() {
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
    let c = s.f.store.connection().unwrap();
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
        .private_initial_telephone_signal_in(
            &c,
            s.phone.clone(),
            crate::InitialTelephoneSignalDirection::FromTelephone,
            partial,
            crate::now() + 5
        )
        .is_err());
    drop(c);
    let read = Arc::new(s.signal(0, FromTelephone).unwrap());
    let clone = read.clone();
    drop(read);
    let writer = [AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()];
    assert!(s.f.store.try_acquire_authority(&writer).is_err());
    drop(clone);
    drop(s.f.store.try_acquire_authority(&writer).unwrap());
}

#[test]
fn private_sip_received_info_and_rfc_freeze_only_original_native_partner_and_recheck_delivery() {
    use connect_protocol::received_dtmf::{RtpObservation, SourceKind, SourceObservation};
    for kind in [SourceKind::SipInfo, SourceKind::SipRfc4733] {
        let s = sealed();
        let observation = SourceObservation {
            kind,
            observation_id: "private-sip-observed-digit".into(),
            received_at: chrono::Utc::now(),
            digits: "5".into(),
            duration_ms: 100,
            rtp: if kind == SourceKind::SipInfo {
                None
            } else {
                Some(RtpObservation {
                    ssrc: 1,
                    payload_type: 101,
                    clock_rate_hz: 8000,
                    rtp_timestamp: 42,
                    extended_timestamp: 42,
                    last_segment_timestamp: 42,
                    duration_samples: 800,
                    event: 5,
                    start_observed: true,
                })
            },
        };
        let candidates = s
            .native
            .iter()
            .enumerate()
            .map(|(i, origin)| crate::NativeDtmfRecipient {
                origin: origin.clone(),
                registration_id: format!("private-candidate-registration-{i}"),
            })
            .collect();
        let batch = match s
            .f
            .store
            .prepare_received_dtmf(
                SipSignalOrigin::Initial(s.phone.clone()),
                observation.clone(),
                candidates,
            )
            .unwrap()
        {
            crate::ReceivedDtmfPreparation::New(batch) => batch,
            _ => panic!("expected fresh private observation"),
        };
        assert_eq!(batch.targets().len(), 1);
        assert_eq!(
            batch.targets()[0].origin().connection_id(),
            s.native[0].connection_id()
        );
        assert_eq!(batch.observation().digits, observation.digits);
        drop(
            s.f.store
                .try_received_dtmf_read(&batch.targets()[0])
                .unwrap(),
        );
        without(&s, &s.f.keys[0], &["dtmf.receive"]);
        assert!(s
            .f
            .store
            .try_received_dtmf_read(&batch.targets()[0])
            .is_err());
        assert!(!s
            .f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT attempted FROM received_dtmf_recipients WHERE operation_record_id=?1",
                [batch.record_id()],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
}
