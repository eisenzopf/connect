//! Controlled purpose/context and authority interval tests, not physical DTMF.
use super::*;
use crate::connection_control::private_consultation::{self as context, Purpose};

fn current(s: &Scene, purpose: Purpose) -> Result<context::Context> {
    let c = s.f.store.connection()?;
    context::registered_current(&s.f.store, &c, s.root.record_id(), purpose)
}
fn established() -> Scene {
    let s = Scene::with_controller(Some("purpose-controller"));
    let receipt = s.captured();
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    s
}
fn without_media(s: &Scene, key: &str) {
    let actor = s.f.store.revalidate(key).unwrap();
    let operations: Vec<_> = actor
        .scope
        .operations
        .into_iter()
        .filter(|p| p != "connection.offer" && p != "stream.subscribe")
        .collect();
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![key, serde_json::to_string(&operations).unwrap()],
        )
        .unwrap();
}

#[test]
fn private_sip_context_separates_signal_presence_from_media_rights() {
    for key in 0..3 {
        let s = established();
        let key = match key {
            0 => &s.f.keys[0],
            1 => &s.f.keys[1],
            _ => &s.policy.endpoint_credential_id,
        };
        without_media(&s, key);
        assert!(current(&s, Purpose::Signal).is_ok());
        assert!(current(&s, Purpose::Presence).is_ok());
        assert!(current(&s, Purpose::Media).is_err());
        assert!(context::observation_source_current(
            &s.f.store,
            &s.f.store.connection().unwrap(),
            s.native[0].connection_id()
        )
        .unwrap());
    }
}

#[test]
fn private_sip_signal_context_requires_captured_application_and_current_pending_execution() {
    let s = Scene::with_controller(Some("pending-purpose-controller"));
    assert!(current(&s, Purpose::Signal).is_err());
    assert!(current(&s, Purpose::Presence).is_ok());
    let receipt = s.captured();
    assert!(current(&s, Purpose::Signal).is_ok());
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.root.credential],
        )
        .unwrap();
    assert!(current(&s, Purpose::Signal).is_err());
    s.f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .unwrap();
    for purpose in [Purpose::Signal, Purpose::Presence, Purpose::Media] {
        assert!(current(&s, purpose).is_ok());
    }
    // A later credential cannot reopen execution or public/private admission.
    assert!(s.read().is_err());
    assert!(s
        .f
        .store
        .try_private_initial_telephone_admission_read(s.phone.clone())
        .is_err());
}

#[test]
fn private_sip_signal_context_preserves_physical_epoch_partner_and_recipient_fences() {
    for fault in 0..14 {
        let s = established();
        let c = s.f.store.connection().unwrap();
        let (sql, id) = match fault {
            0 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            1 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            2 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.endpoint_credential_id.as_str(),
            ),
            3 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
            4 => (
                "UPDATE credentials SET recipients='[]' WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            5 => (
                "UPDATE connections SET generation=generation+1 WHERE id=?1",
                s.phone.connection_id(),
            ),
            6 => (
                "UPDATE connections SET observed_runtime_id='foreign' WHERE id=?1",
                s.phone.connection_id(),
            ),
            7 => (
                "UPDATE endpoints SET revision=revision+1 WHERE id=?1",
                s.policy.endpoint_id.as_str(),
            ),
            8 => (
                "UPDATE devices SET revoked=1 WHERE id=?1",
                "private-sip-device",
            ),
            9 => (
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                s.f.pids[2].as_str(),
            ),
            10 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[0].as_str(),
            ),
            11 => (
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                s.f.keys[1].as_str(),
            ),
            12 => (
                "UPDATE session_participants SET state='left' WHERE participant_id=?1",
                s.f.pids[2].as_str(),
            ),
            _ => (
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                s.policy.route_credential_id.as_str(),
            ),
        };
        c.execute(sql, [id]).unwrap();
        drop(c);
        for purpose in [Purpose::Signal, Purpose::Presence, Purpose::Media] {
            assert!(current(&s, purpose).is_err(), "fault {fault}");
        }
    }
}

#[test]
fn native_primary_signal_read_owns_external_private_route_interval_and_releases_it() {
    let s = established();
    without_media(&s, &s.f.keys[0]);
    without_media(&s, &s.policy.endpoint_credential_id);
    without_media(&s, &s.policy.route_credential_id);
    // Ordinary native reads already fence every current Conversation member.
    // Remove this route owner from membership to test its independent scope.
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&s.f.pids[3]],
        )
        .unwrap();
    let route =
        s.f.store
            .revalidate(&s.policy.route_credential_id)
            .unwrap()
            .identity_id;
    let c = s.f.store.connection().unwrap();
    let (_, scopes, expiry) = s.f.store.native_signal_facts_at(&c, &s.native[0]).unwrap();
    let held = s.f.store.try_acquire_authority(&scopes).unwrap();
    let read =
        s.f.store
            .native_signal_read_in(&c, &s.native[0], held.clone(), expiry)
            .unwrap();
    drop(held);
    drop(c);
    assert!(read.current());
    let writer = [AuthorityScope::identity(&route, AuthorityMode::Write).unwrap()];
    assert!(s.f.store.try_acquire_authority(&writer).is_err());
    drop(read);
    drop(s.f.store.try_acquire_authority(&writer).unwrap());
    // A held remote has its own public context; it never picks up this private route.
    let c = s.f.store.connection().unwrap();
    let (_, scopes, expiry) = s.f.store.native_signal_facts_at(&c, &s.native[1]).unwrap();
    let held = s.f.store.try_acquire_authority(&scopes).unwrap();
    let read =
        s.f.store
            .native_signal_read_in(&c, &s.native[1], held, expiry)
            .unwrap();
    assert!(read.current());
    drop(s.f.store.try_acquire_authority(&writer).unwrap());
}
