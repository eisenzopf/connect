//! Controlled Store boundaries only; no SIP/native delivery is inferred.
use super::*;
use InitialTelephoneSignalDirection::{FromTelephone, ToTelephone};
mod audience_origin;

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

#[test]
fn initial_sip_audience_requires_exact_complete_owned_roster_without_native_provenance() {
    let f = Fixture::new();
    let origin = connected(&f);
    let c = f.store.connection().unwrap();
    let (scopes, _, _) = f
        .store
        .initial_telephone_audience_facts(&c, &origin)
        .unwrap();
    assert!(!scopes.is_empty());
    let (_, snapshot, _) = f
        .store
        .initial_telephone_audience_facts(&c, &origin)
        .unwrap();
    assert_eq!(snapshot["source_kind"], "initial-sip");
    assert_eq!(snapshot["record_id"], origin.record_id());
    assert_eq!(snapshot["sip_call_id"], origin.sip_call_id());
    assert_eq!(snapshot["generation"], 1);
    for key in [
        "key_id",
        "key_algorithm",
        "public_key_sha256",
        "signing_key_id",
    ] {
        assert!(
            snapshot.get(key).is_none(),
            "SIP source borrowed native provenance: {key}"
        );
    }
    let facts = |sources: &[crate::SipAudienceOrigin], native: Vec<String>| {
        f.store.sip_audience_facts(
            &c,
            &f.request.conversation_id,
            &f.request.session_id,
            sources,
            native,
        )
    };
    assert!(facts(&[crate::SipAudienceOrigin::Initial(origin.clone())], vec![]).is_ok());
    assert_eq!(
        facts(&[], vec![]).unwrap_err().code,
        "authority-unavailable"
    );
    assert!(facts(
        &[
            crate::SipAudienceOrigin::Initial(origin.clone()),
            crate::SipAudienceOrigin::Initial(origin.clone())
        ],
        vec![]
    )
    .is_err());
    assert!(facts(
        &[crate::SipAudienceOrigin::Initial(origin.clone())],
        vec![origin.connection_id().into()]
    )
    .is_err());
    assert!(f
        .store
        .sip_audience_facts(
            &c,
            "wrong-conversation",
            &f.request.session_id,
            &[crate::SipAudienceOrigin::Initial(origin.clone())],
            vec![]
        )
        .is_err());
    assert!(f
        .store
        .sip_audience_facts(
            &c,
            &f.request.conversation_id,
            "wrong-session",
            &[crate::SipAudienceOrigin::Initial(origin.clone())],
            vec![]
        )
        .is_err());
    let other = Fixture::new();
    assert!(other
        .store
        .initial_telephone_audience_facts(&other.store.connection().unwrap(), &origin)
        .is_err());
    // Facts retain no idle permission; the public credential writer can finish.
    drop(c);
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    f.store.revoke(&actor, &f.keys[1]).unwrap();
    assert!(f
        .store
        .initial_telephone_audience_facts(&f.store.connection().unwrap(), &origin)
        .is_err());
}
#[test]
fn initial_sip_audience_refuses_answer_only_without_actual_connected_observation() {
    let f = Fixture::new();
    let origin = answered(&f);
    assert!(f
        .store
        .initial_telephone_audience_facts(&f.store.connection().unwrap(), &origin)
        .is_err());
}
#[test]
fn initial_sip_audience_captured_revision_never_revives_retired_or_overwrites_newer_observation() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let origin = f
        .store
        .record_initial_telephone_answer(
            &signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap(),
        )
        .unwrap()
        .media
        .unwrap();
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
    let c = f.store.connection().unwrap();
    let (_, snapshot, _) = f
        .store
        .initial_telephone_audience_facts(&c, &origin)
        .unwrap();
    let revision = || {
        c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",
            [origin.connection_id()],
            |r| r.get::<_, u64>(0),
        )
        .unwrap()
    };
    f.store
        .record_initial_telephone_audience_revision(&c, &snapshot, f.store.runtime_epoch(), 1, 2)
        .unwrap();
    assert_eq!(revision(), 2);
    f.store
        .record_initial_telephone_audience_revision(&c, &snapshot, f.store.runtime_epoch(), 1, 2)
        .unwrap();
    assert_eq!(revision(), 2);
    f.store
        .record_initial_telephone_audience_revision(&c, &snapshot, "foreign-runtime", 2, 3)
        .unwrap();
    assert_eq!(revision(), 2);
    let mut native = snapshot.clone();
    native["source_kind"] = json!("native");
    assert!(f
        .store
        .record_initial_telephone_audience_revision(&c, &native, f.store.runtime_epoch(), 2, 3)
        .is_err());
    assert!(f
        .store
        .record_initial_telephone_audience_revision(&c, &snapshot, f.store.runtime_epoch(), 2, 4)
        .is_err());
    c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',5) WHERE id=?1",[origin.connection_id()]).unwrap();
    f.store
        .record_initial_telephone_audience_revision(&c, &snapshot, f.store.runtime_epoch(), 2, 3)
        .unwrap();
    assert_eq!(revision(), 5);
    drop(c);
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    let c = f.store.connection().unwrap();
    f.store
        .record_initial_telephone_audience_revision(&c, &snapshot, f.store.runtime_epoch(), 5, 6)
        .unwrap();
    let state: (String,u64,Option<String>,Option<String>) = c.query_row("SELECT state,generation,readiness_json,observed_runtime_id FROM connections WHERE id=?1",[origin.connection_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("retired".into(), 2, None, None));
}
