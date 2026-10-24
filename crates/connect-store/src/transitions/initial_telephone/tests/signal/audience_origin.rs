//! Current typed origin resolution only; these controlled captures are not SIP IO.
use super::*;

#[test]
fn initial_sip_retained_readiness_and_departure_require_exact_captured_observations() {
    let f = Fixture::new();
    let signal = Arc::new(f.signal());
    let media = f
        .store
        .record_initial_telephone_answer(
            &signal.capture_answer(signal.sip_call_id(), "pcmu").unwrap(),
        )
        .unwrap()
        .media
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        assert!(!crate::transitions::authority::retained_ready(
            &c,
            media.connection_id(),
            "ready",
            &f.store
        )
        .unwrap());
    }
    f.store
        .record_initial_telephone_connected(
            media.clone(),
            &ReadyFixture(InitialTelephoneReadyObservation {
                runtime_connection_id: media.runtime_connection_id().into(),
                call_id: media.sip_call_id().into(),
                participant_id: f.parts[1].clone(),
                membership_epoch: media.source().target.membership_epoch,
                generation: 1,
                stream_revision: 1,
                participant_revision: 1,
                streams: vec!["telephone-audio".into()],
            }),
        )
        .unwrap();
    drop(
        f.store
            .try_initial_telephone_media_read(media.clone())
            .unwrap(),
    );
    let binding = {
        let c = f.store.connection().unwrap();
        assert!(crate::transitions::authority::retained_ready(
            &c,
            media.connection_id(),
            "ready",
            &f.store
        )
        .unwrap());
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &media)
            .unwrap();
        assert!(!Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
        binding
    };
    assert!(f
        .store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(false))
        .is_err());
    f.store
        .settle_initial_telephone_cleanup(&signal, &AbsentFixture(true))
        .unwrap();
    let c = f.store.connection().unwrap();
    assert!(Store::recorded_sip_departure(&c, &binding, f.store.runtime_epoch()).unwrap());
    assert!(!Store::recorded_sip_departure(&c, &binding, "foreign-runtime").unwrap());
    let mut foreign = binding.clone();
    foreign["sip_call_id"] = json!("other-call");
    assert!(!Store::recorded_sip_departure(&c, &foreign, f.store.runtime_epoch()).unwrap());
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id
        )
        .is_err());
}

#[test]
fn initial_sip_captured_origin_is_metadata_and_requires_new_current_readers() {
    let f = Fixture::new();
    let origin = connected(&f);
    drop(
        f.store
            .try_initial_telephone_media_read(origin.clone())
            .unwrap(),
    );
    let selected =
        crate::ConsultationMediaOrigin::Sip(crate::SipAudienceOrigin::Initial(origin.clone()));
    assert_eq!(selected.connection_id(), origin.connection_id());
    assert_eq!(selected.participant_id(), f.parts[1]);
    assert_eq!(
        selected.membership_epoch(),
        origin.source().target.membership_epoch
    );
    let binding = {
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &origin)
            .unwrap();
        let (scopes, until) = f
            .store
            .sip_audience_snapshot_facts(
                &c,
                &binding,
                &f.request.conversation_id,
                &f.request.session_id,
            )
            .unwrap();
        assert!(!scopes.is_empty() && until <= crate::now() + 5);
        assert!(binding.get("key_id").is_none());
        binding
    };
    let writer = f
        .store
        .try_acquire_authority(
            &[AuthorityScope::identity(&f.ids[1], AuthorityMode::Write).unwrap()],
        )
        .unwrap();
    assert!(f
        .store
        .try_initial_telephone_media_read(origin.clone())
        .is_err());
    drop(writer);
    drop(
        f.store
            .try_initial_telephone_media_read(origin.clone())
            .unwrap(),
    );
    let c = f.store.connection().unwrap();
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id
        )
        .is_ok());
}

#[test]
fn initial_sip_captured_origin_refuses_forged_shape_scope_and_missing_observation() {
    let f = Fixture::new();
    let origin = connected(&f);
    drop(
        f.store
            .try_initial_telephone_media_read(origin.clone())
            .unwrap(),
    );
    let c = f.store.connection().unwrap();
    let (_, binding, _) = f
        .store
        .initial_telephone_audience_facts(&c, &origin)
        .unwrap();
    for (key, value) in [
        ("sip_call_id", json!("foreign-call")),
        ("generation", json!(2)),
        ("key_id", json!("invented-native-key")),
        ("membership_epoch", json!(999)),
    ] {
        let mut forged = binding.clone();
        forged[key] = value;
        assert!(
            f.store
                .sip_audience_snapshot_facts(
                    &c,
                    &forged,
                    &f.request.conversation_id,
                    &f.request.session_id
                )
                .is_err(),
            "{key}"
        );
    }
    assert!(f
        .store
        .sip_audience_snapshot_facts(&c, &binding, &f.request.conversation_id, "foreign-session")
        .is_err());
    let cached = f
        .store
        .session_ready_origins
        .lock()
        .unwrap()
        .remove(origin.connection_id())
        .unwrap();
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id
        )
        .is_err());
    f.store
        .session_ready_origins
        .lock()
        .unwrap()
        .insert(origin.connection_id().into(), cached);
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id
        )
        .is_ok());
}

#[test]
fn initial_sip_cached_metadata_cannot_resurrect_withdrawn_physical_authority() {
    for fault in ["endpoint", "route", "generation", "retired"] {
        let f = Fixture::new();
        let origin = connected(&f);
        drop(
            f.store
                .try_initial_telephone_media_read(origin.clone())
                .unwrap(),
        );
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &origin)
            .unwrap();
        match fault {
            "endpoint" => {
                c.execute(
                    "UPDATE endpoints SET verification_state='revoked' WHERE id=?1",
                    [&f.policy.endpoint_id],
                )
                .unwrap();
            }
            "route" => {
                c.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&f.policy.route_credential_id],
                )
                .unwrap();
            }
            "generation" => {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [origin.connection_id()],
                )
                .unwrap();
            }
            "retired" => {
                c.execute("UPDATE initial_telephone_answers SET retired_at=?2 WHERE operation_record_id=?1", params![origin.record_id(),crate::now()]).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            f.store
                .sip_audience_snapshot_facts(
                    &c,
                    &binding,
                    &f.request.conversation_id,
                    &f.request.session_id
                )
                .is_err(),
            "{fault}"
        );
    }
}
