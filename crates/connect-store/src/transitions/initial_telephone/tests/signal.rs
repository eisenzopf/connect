//! Controlled Store boundaries only; no SIP/native delivery is inferred.
use super::*;
use InitialTelephoneSignalDirection::{FromTelephone, ToTelephone};

fn answered(f: &Fixture) -> Arc<InitialTelephoneMediaOrigin> {
    let signal = Arc::new(f.signal());
    f.store
        .record_initial_telephone_answer(
            &signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap(),
        )
        .unwrap()
        .media
        .unwrap()
}
fn connected(f: &Fixture) -> Arc<InitialTelephoneMediaOrigin> {
    let origin = answered(f);
    f.store
        .record_initial_telephone_connected(
            origin.clone(),
            &ReadyFixture(InitialTelephoneReadyObservation {
                runtime_connection_id: origin.runtime_connection_id().into(),
                call_id: origin.sip_call_id().into(),
                participant_id: f.parts[1].clone(),
                membership_epoch: origin.source().target.membership_epoch,
                generation: 1,
                stream_revision: 1,
                participant_revision: 1,
                streams: vec!["telephone-audio".into()],
            }),
        )
        .unwrap();
    origin
}
fn permissions(f: &Fixture, names: &[&str]) {
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![f.keys[1], serde_json::to_string(names).unwrap()],
        )
        .unwrap();
}

#[test]
fn initial_signal_requires_each_named_direction_and_never_borrows_media_permissions() {
    let f = Fixture::new();
    let origin = connected(&f);
    for (names, send, receive) in [
        (vec!["connection.offer", "stream.subscribe"], false, false),
        (vec!["dtmf.send"], true, false),
        (vec!["dtmf.receive"], false, true),
        (vec!["dtmf.send", "dtmf.receive"], true, true),
    ] {
        permissions(&f, &names);
        for (direction, allowed) in [(FromTelephone, send), (ToTelephone, receive)] {
            let read = f
                .store
                .try_initial_telephone_signal_read(origin.clone(), direction);
            assert_eq!(read.is_ok(), allowed, "{names:?} {direction:?}: {read:?}");
            if let Ok(read) = read {
                assert!(read.current());
                assert_eq!(read.direction(), direction);
                assert_eq!(read.member().participant_id, f.parts[1]);
                assert_eq!(read.origin().sip_call_id(), origin.sip_call_id());
                assert!(read.expires_at() <= crate::now() + 30);
            }
        }
        if send || receive {
            assert!(f
                .store
                .try_initial_telephone_media_read(origin.clone())
                .is_err());
        }
    }
}

#[test]
fn initial_signal_does_not_reuse_answer_or_registration_as_connected_authority() {
    let f = Fixture::new();
    let origin = answered(&f);
    for direction in [FromTelephone, ToTelephone] {
        assert!(f
            .store
            .try_initial_telephone_signal_read(origin.clone(), direction)
            .is_err());
    }
}

#[test]
fn initial_signal_codec_change_does_not_supply_or_remove_named_signal_permission() {
    let f = Fixture::new();
    let origin = connected(&f);
    // Changing current audio preferences can invalidate the old media codec.
    // It neither grants nor revokes this existing call's independent signal.
    f.store.connection().unwrap().execute(
        "UPDATE session_presence_capabilities SET capabilities_json=?3 WHERE session_id=?1 AND participant_id=?2",
        params![f.request.session_id, f.parts[1], json!({"audio":["g.711-a"]}).to_string()],
    ).unwrap();
    assert!(f
        .store
        .try_initial_telephone_media_read(origin.clone())
        .is_err());
    permissions(&f, &["dtmf.send"]);
    assert!(f
        .store
        .try_initial_telephone_signal_read(origin.clone(), FromTelephone)
        .is_ok());
    assert!(f
        .store
        .try_initial_telephone_signal_read(origin, ToTelephone)
        .is_err());
}

#[test]
fn initial_signal_retains_current_authority_only_for_its_bounded_read() {
    let f = Fixture::new();
    let origin = connected(&f);
    let read = f
        .store
        .try_initial_telephone_signal_read(origin.clone(), FromTelephone)
        .unwrap();
    let scopes = [AuthorityScope::identity(&f.ids[1], AuthorityMode::Write).unwrap()];
    assert!(f.store.try_acquire_authority(&scopes).is_err());
    drop(read);
    assert!(f.store.try_acquire_authority(&scopes).is_ok());
    f.store
        .revoke(&f.store.revalidate(&f.keys[2]).unwrap(), &f.keys[1])
        .unwrap();
    assert!(f
        .store
        .try_initial_telephone_signal_read(origin, FromTelephone)
        .is_err());
}

#[test]
fn initial_signal_rechecks_physical_endpoint_route_and_participant_lifetime() {
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
        let f = Fixture::new();
        let origin = connected(&f);
        let c = f.store.connection().unwrap();
        match fault {
            "device" => c.execute("UPDATE devices SET revoked=1 WHERE id='initial-phone-device'", []),
            "endpoint" => c.execute("UPDATE endpoints SET revision=revision+1 WHERE id='initial-phone'", []),
            "generation" => c.execute("UPDATE connections SET generation=generation+1 WHERE id=?1", [origin.connection_id()]),
            "runtime" => c.execute("UPDATE connections SET observed_runtime_id='another-runtime' WHERE id=?1", [origin.connection_id()]),
            "connection" => c.execute("UPDATE connections SET state='preparing' WHERE id=?1", [origin.connection_id()]),
            "route" => c.execute("UPDATE credentials SET operations='[]' WHERE id=?1", [&f.policy.route_credential_id]),
            "member" => c.execute("UPDATE participants SET active=0 WHERE participant_id=?1", [&f.parts[1]]),
            "presence" => c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND participant_id=?2", params![f.request.session_id, f.parts[1]]),
            "session" => c.execute("UPDATE sessions SET state='ended' WHERE id=?1", [&f.request.session_id]),
            "audience" => c.execute("UPDATE credentials SET recipients='[]' WHERE id=?1", [&f.keys[1]]),
            _ => unreachable!(),
        }.unwrap();
        drop(c);
        for direction in [FromTelephone, ToTelephone] {
            assert!(
                f.store
                    .try_initial_telephone_signal_read(origin.clone(), direction)
                    .is_err(),
                "{fault} {direction:?}"
            );
        }
    }
}
