//! Controlled established-call authority only; these tests do not assert SIP IO.
use super::*;
use crate::TelephoneSignalDirection::{FromTelephone, ToTelephone};

fn established() -> (
    Fixture,
    Arc<crate::TelephoneMediaOrigin>,
    Vec<Arc<crate::NativeMediaOrigin>>,
    String,
) {
    let (f, origin, sources, _, route) = setup_runtime(true);
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
    (f, phone, sources, route)
}
fn permissions(f: &Fixture, key: &str, names: &[&str]) {
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![key, serde_json::to_string(names).unwrap()],
        )
        .unwrap();
}

#[test]
fn established_signal_requires_independent_named_directions_without_codec_or_media_permission() {
    let (f, phone, _, route) = established();
    // Keep the actual established incarnation while changing the current audio
    // preferences. A signal read must not manufacture a media grant for it.
    f.store.connection().unwrap().execute(
        "UPDATE session_presence_capabilities SET capabilities_json=?3 WHERE session_id=?1 AND participant_id=?2",
        params![phone.snapshot().session_id, phone.snapshot().participant_id, json!({"audio":["g.711-a"]}).to_string()],
    ).unwrap();
    assert!(f.store.try_telephone_media_read(phone.clone()).is_err());
    for (names, send, receive) in [
        (
            vec!["sip_dial", "connection.offer", "stream.subscribe"],
            false,
            false,
        ),
        (vec!["sip_dial", "dtmf.send"], true, false),
        (vec!["sip_dial", "dtmf.receive"], false, true),
        (vec!["sip_dial", "dtmf.send", "dtmf.receive"], true, true),
        (vec!["dtmf.send", "dtmf.receive"], false, false),
    ] {
        permissions(&f, &route, &names);
        for (direction, expected) in [(FromTelephone, send), (ToTelephone, receive)] {
            let read = f.store.try_telephone_signal_read(phone.clone(), direction);
            assert_eq!(read.is_ok(), expected, "{names:?} {direction:?}: {read:?}");
            if let Ok(read) = read {
                assert!(read.current());
                assert_eq!(read.direction(), direction);
                assert_eq!(
                    read.member().participant_id,
                    phone.snapshot().participant_id
                );
                assert_eq!(read.snapshot().generation, 2);
                assert_ne!(read.route_actor().identity_id, read.member().identity_id);
                assert!(read.expires_at() <= crate::now() + 30);
            }
        }
    }
}

#[test]
fn established_signal_pair_requires_opposite_native_direction_and_both_recipient_ceilings() {
    let (f, phone, sources, route) = established();
    permissions(&f, &route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    let native_key = {
        let c = f.store.connection().unwrap();
        f.store
            .native_media_route_actor(&c, &sources[1])
            .unwrap()
            .credential_id
    };
    for (names, send, receive) in [
        (
            vec![
                "conversation.get",
                "subscribe",
                "connection.offer",
                "stream.subscribe",
            ],
            false,
            false,
        ),
        (
            vec!["conversation.get", "subscribe", "dtmf.receive"],
            true,
            false,
        ),
        (
            vec!["conversation.get", "subscribe", "dtmf.send"],
            false,
            true,
        ),
        (
            vec!["conversation.get", "subscribe", "dtmf.send", "dtmf.receive"],
            true,
            true,
        ),
    ] {
        permissions(&f, &native_key, &names);
        for (direction, expected) in [(FromTelephone, send), (ToTelephone, receive)] {
            let read = f.store.try_telephone_native_signal_read(
                phone.clone(),
                sources[1].clone(),
                direction,
            );
            assert_eq!(read.is_ok(), expected, "{names:?} {direction:?}: {read:?}");
            if let Ok(read) = read {
                assert!(read.current());
                assert_eq!(
                    read.phone().snapshot().connection_id,
                    phone.snapshot().connection_id
                );
                assert_eq!(
                    read.native().target().connection.connection_id,
                    sources[1].connection_id()
                );
                assert!(Arc::ptr_eq(read.interval(), read.native().interval()));
            }
        }
    }
    // The retired native source cannot stand in for either established endpoint.
    assert!(f
        .store
        .try_telephone_native_signal_read(phone.clone(), sources[0].clone(), FromTelephone)
        .is_err());
    for key in [&route, &native_key] {
        let recipients: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT recipients FROM credentials WHERE id=?1",
                [key],
                |r| r.get(0),
            )
            .unwrap();
        f.store
            .connection()
            .unwrap()
            .execute("UPDATE credentials SET recipients='[]' WHERE id=?1", [key])
            .unwrap();
        for direction in [FromTelephone, ToTelephone] {
            assert!(
                f.store
                    .try_telephone_native_signal_read(phone.clone(), sources[1].clone(), direction)
                    .is_err(),
                "{key} {direction:?}"
            );
        }
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET recipients=?2 WHERE id=?1",
                params![key, recipients],
            )
            .unwrap();
    }
    assert!(f
        .store
        .try_telephone_native_signal_read(phone, sources[1].clone(), FromTelephone)
        .unwrap()
        .current());
}

#[test]
fn established_signal_pair_rejects_same_participant_native_connection() {
    let (f, native, phone, route) = mixed_role_sources();
    permissions(&f, &route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    assert_eq!(native[0].participant_id(), phone.snapshot().participant_id);
    assert!(f
        .store
        .try_telephone_signal_read(phone.clone(), FromTelephone)
        .unwrap()
        .current());
    for direction in [FromTelephone, ToTelephone] {
        assert!(f
            .store
            .try_telephone_native_signal_read(phone.clone(), native[0].clone(), direction)
            .is_err());
    }
}

#[test]
fn established_signal_rechecks_actual_call_route_device_and_membership_lifetime() {
    for fault in [
        "device",
        "endpoint",
        "generation",
        "runtime",
        "connection",
        "route",
        "member",
        "presence",
        "session",
        "audience",
    ] {
        let (f, phone, _, route) = established();
        permissions(&f, &route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
        assert!(f
            .store
            .try_telephone_signal_read(phone.clone(), FromTelephone)
            .unwrap()
            .current());
        let s = phone.snapshot();
        let c = f.store.connection().unwrap();
        let rows = match fault {
            "device" => c.execute("UPDATE devices SET revoked=1 WHERE id=?1", [&s.device_id]),
            "endpoint" => c.execute("UPDATE endpoints SET verification_state='unverified' WHERE id=?1", [&s.endpoint_id]),
            "generation" => c.execute("UPDATE connections SET generation=generation+1 WHERE id=?1", [&s.connection_id]),
            "runtime" => c.execute("UPDATE connections SET observed_runtime_id='another-runtime' WHERE id=?1", [&s.connection_id]),
            "connection" => c.execute("UPDATE connections SET state='preparing' WHERE id=?1", [&s.connection_id]),
            "route" => c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&route]),
            "member" => c.execute("UPDATE participants SET active=0 WHERE participant_id=?1", [&s.participant_id]),
            "presence" => c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND participant_id=?2", params![s.session_id,s.participant_id]),
            "session" => c.execute("UPDATE sessions SET state='ended' WHERE id=?1", [&s.session_id]),
            "audience" => c.execute("UPDATE credentials SET recipients='[]' WHERE id=?1", [&route]),
            _ => unreachable!(),
        }.unwrap();
        assert!(rows > 0, "{fault}");
        drop(c);
        for direction in [FromTelephone, ToTelephone] {
            assert!(
                f.store
                    .try_telephone_signal_read(phone.clone(), direction)
                    .is_err(),
                "{fault} {direction:?}"
            );
        }
    }
}

#[test]
fn established_signal_pair_holds_one_bounded_interval_and_refuses_fresh_read_after_route_revocation(
) {
    let (f, phone, sources, route) = established();
    permissions(&f, &route, &["sip_dial", "dtmf.send"]);
    permissions(
        &f,
        &f.keys[1],
        &["conversation.get", "subscribe", "dtmf.receive"],
    );
    let read = f
        .store
        .try_telephone_native_signal_read(phone.clone(), sources[1].clone(), FromTelephone)
        .unwrap();
    let write = crate::AuthorityScope::identity(
        &phone.snapshot().route_owner_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    let native_write = crate::AuthorityScope::identity(
        &read.native().target().session.actor.identity_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    assert!(f
        .store
        .try_acquire_authority(&[native_write.clone()])
        .is_err());
    let admin = f.store.revalidate(&f.admin).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| f.store.revoke(&admin, &route));
        crate::authority_fences::test_wait_for_writer(&f.store, &write);
        assert!(read.current());
        assert!(f
            .store
            .try_telephone_native_signal_read(phone.clone(), sources[1].clone(), FromTelephone)
            .is_err());
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert!(f.store.try_acquire_authority(&[native_write]).is_ok());
    assert!(f
        .store
        .try_telephone_native_signal_read(phone, sources[1].clone(), FromTelephone)
        .is_err());
}
