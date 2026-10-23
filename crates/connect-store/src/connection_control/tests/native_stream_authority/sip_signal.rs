//! Controlled Store mixed signal authority. No network delivery is claimed.
use super::*;
use crate::{InitialTelephoneMediaOrigin, InitialTelephoneSignalDirection::*};
use std::sync::Arc;

pub(super) fn fixture() -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    Arc<InitialTelephoneMediaOrigin>,
) {
    let (mut f, _, _, native) = parent_permissions_origin_extra(
        true,
        true,
        false,
        false,
        true,
        &["dtmf.send", "dtmf.receive"],
    );
    let identity = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    let route_owner = f.store.revalidate(&f.admin).unwrap().identity_id;
    f.keys[1] = f
        .store
        .issue(&identity, Scope::operator(), crate::now() + 600)
        .unwrap()
        .id;
    let request = {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('signal-phone-device',?1,'tnt_default','phone','isolated-fixture')", [&identity]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at,secret_ref,route_owner_id) VALUES('signal-phone',?1,'tnt_default','signal-phone-device','telephone','+15555550111','[\"audio\"]','verified','controlled-fixture',?2,'signal-trunk',?3)",params![identity,crate::now(),route_owner]).unwrap();
        let saved = crate::operations::receipts::receipt(&c, &route_owner, "invite")
            .unwrap()
            .unwrap();
        let Operation::SessionControl { request } = saved.operation else {
            panic!()
        };
        // The shared native fixture accepts every Participant in advance.
        // This controlled SIP leg instead supplies its own observed answer.
        c.execute("UPDATE session_participants SET state='invited' WHERE session_id='session' AND participant_id=?1", [&f.pids[1]]).unwrap();
        request
    };
    let origin = f
        .store
        .prepare_initial_telephone(
            &f.keys[0],
            request,
            &f.pids[1],
            crate::InitialTelephonePolicy {
                endpoint_id: "signal-phone".into(),
                endpoint_credential_id: f.keys[1].clone(),
                route_credential_id: f.admin.clone(),
                route_reference: "signal-trunk".into(),
                sip_authority: "127.0.0.1:5099".into(),
                timeout_seconds: 30,
            },
        )
        .unwrap();
    let allocation = Arc::new(
        f.store
            .begin_initial_telephone_allocation(&origin)
            .unwrap()
            .unwrap(),
    );
    f.store
        .record_initial_telephone_allocation(
            &allocation,
            "actual-sip-core",
            "actual-sip-call@fixture",
        )
        .unwrap();
    let signal = Arc::new(
        f.store
            .begin_initial_telephone_signaling(allocation)
            .unwrap()
            .unwrap(),
    );
    let captured = signal.capture_answer(signal.sip_call_id(), "opus").unwrap();
    let recorded = f.store.record_initial_telephone_answer(&captured).unwrap();
    let phone = recorded
        .media
        .expect(&format!("answer acceptance: {:?}", recorded.problem));
    struct Ready(crate::InitialTelephoneReadyObservation);
    impl crate::InitialTelephoneRuntimeObservation for Ready {
        fn observe(&self) -> connect_protocol::Result<crate::InitialTelephoneReadyObservation> {
            Ok(self.0.clone())
        }
    }
    f.store
        .record_initial_telephone_connected(
            phone.clone(),
            &Ready(crate::InitialTelephoneReadyObservation {
                runtime_connection_id: signal.runtime_connection_id().into(),
                call_id: signal.sip_call_id().into(),
                participant_id: f.pids[1].clone(),
                membership_epoch: 1,
                generation: 1,
                stream_revision: 1,
                participant_revision: 1,
                streams: vec!["telephone-audio".into()],
            }),
        )
        .unwrap();
    (f, native, phone)
}
fn operations(f: &Fixture, actor: usize, names: &[&str]) {
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![f.keys[actor], serde_json::to_string(names).unwrap()],
        )
        .unwrap();
}

#[test]
fn sip_native_signal_pair_requires_opposite_named_permissions_without_media_grants() {
    let (f, native, phone) = fixture();
    for direction in [FromTelephone, ToTelephone] {
        let (phone_permission, native_permission) = match direction {
            FromTelephone => ("dtmf.send", "dtmf.receive"),
            ToTelephone => ("dtmf.receive", "dtmf.send"),
        };
        operations(&f, 0, &[native_permission]);
        operations(&f, 1, &[phone_permission]);
        let read = f
            .store
            .try_initial_telephone_native_signal_read(phone.clone(), native.clone(), direction)
            .unwrap();
        assert!(read.current());
        assert_eq!(read.phone().direction(), direction);
        assert_eq!(read.phone().member().participant_id, f.pids[1]);
        assert_eq!(read.native().target().connection.participant_id, f.pids[0]);
        assert!(read.expires_at() <= crate::now() + 30);
        drop(read);
        assert!(f.store.try_native_media_read(&native, None).is_err());
        assert!(f
            .store
            .try_initial_telephone_media_read(phone.clone())
            .is_err());
        operations(&f, 0, &[phone_permission]);
        assert!(f
            .store
            .try_initial_telephone_native_signal_read(phone.clone(), native.clone(), direction)
            .is_err());
        operations(&f, 0, &[native_permission]);
        operations(&f, 1, &[native_permission]);
        assert!(f
            .store
            .try_initial_telephone_native_signal_read(phone.clone(), native.clone(), direction)
            .is_err());
    }
}

#[test]
fn sip_native_signal_pair_holds_both_identity_fences_and_queues_no_reader() {
    let (f, native, phone) = fixture();
    let read = f
        .store
        .try_initial_telephone_native_signal_read(phone.clone(), native.clone(), ToTelephone)
        .unwrap();
    for key in &f.keys[..2] {
        let identity = f.store.revalidate(key).unwrap().identity_id;
        let scopes =
            [crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap()];
        assert!(f.store.try_acquire_authority(&scopes).is_err());
    }
    drop(read);
    for key in &f.keys[..2] {
        let identity = f.store.revalidate(key).unwrap().identity_id;
        let scopes =
            [crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap()];
        assert!(f.store.try_acquire_authority(&scopes).is_ok());
    }
    operations(&f, 1, &[]);
    assert!(f
        .store
        .try_initial_telephone_native_signal_read(phone, native, ToTelephone)
        .is_err());
}

#[test]
fn sip_native_signal_pair_rechecks_native_physical_authority_and_both_recipient_ceilings() {
    for fault in [
        "native-recipients",
        "phone-recipients",
        "native-device",
        "native-key",
        "native-credential",
        "native-generation",
        "native-member",
        "native-endpoint",
    ] {
        let (f, native, phone) = fixture();
        let c = f.store.connection().unwrap();
        match fault {
            "native-recipients" => c.execute("UPDATE credentials SET recipients='[]' WHERE id=?1", [&f.keys[0]]),
            "phone-recipients" => c.execute("UPDATE credentials SET recipients='[]' WHERE id=?1", [&f.keys[1]]),
            "native-device" => c.execute("UPDATE devices SET revoked=1 WHERE id=(SELECT device_id FROM endpoints WHERE id=?1)", [&f.endpoints[0]]),
            "native-key" => c.execute("UPDATE peer_signing_keys SET revoked=1 WHERE key_id=(SELECT key_id FROM connection_offers WHERE connection_id=?1)", [native.connection_id()]),
            "native-credential" => c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]]),
            "native-generation" => c.execute("UPDATE connections SET generation=generation+1 WHERE id=?1", [native.connection_id()]),
            "native-member" => c.execute("UPDATE participants SET active=0 WHERE participant_id=?1", [&f.pids[0]]),
            "native-endpoint" => c.execute("UPDATE endpoints SET verification_state='revoked' WHERE id=?1", [&f.endpoints[0]]),
            _ => unreachable!(),
        }.unwrap();
        drop(c);
        for direction in [FromTelephone, ToTelephone] {
            assert!(
                f.store
                    .try_initial_telephone_native_signal_read(
                        phone.clone(),
                        native.clone(),
                        direction
                    )
                    .is_err(),
                "{fault} {direction:?}"
            );
        }
    }
}
