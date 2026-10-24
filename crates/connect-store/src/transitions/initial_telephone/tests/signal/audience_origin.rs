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
        assert!(Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
        for (key, value) in [
            ("sip_call_id", json!("foreign-call")),
            ("runtime_connection_id", json!("foreign-runtime-call")),
            ("membership_epoch", json!(999)),
            ("generation", json!(2)),
            ("endpoint_id", json!("foreign-endpoint")),
        ] {
            let mut forged = binding.clone();
            forged[key] = value;
            assert!(!Store::recorded_sip_live(&c, &forged, f.store.runtime_epoch()).unwrap());
        }
        assert!(!Store::recorded_sip_live(&c, &binding, "foreign-runtime").unwrap());
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
    assert!(!Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
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

#[test]
fn initial_sip_return_completion_requires_full_current_interval_and_named_receive_permission() {
    let f = Fixture::new();
    let origin = connected(&f);
    drop(
        f.store
            .try_initial_telephone_media_read(origin.clone())
            .unwrap(),
    );
    let (scopes, binding, _) = f
        .store
        .initial_telephone_audience_facts(&f.store.connection().unwrap(), &origin)
        .unwrap();
    let c = f.store.connection().unwrap();
    let narrow = f
        .store
        .try_acquire_authority(&[AuthorityScope::conversation(
            &f.request.conversation_id,
            AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    assert!(f
        .store
        .sip_completion_actors(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id,
            &narrow
        )
        .unwrap()
        .is_none());
    drop(narrow);
    let held = f.store.try_acquire_authority(&scopes).unwrap();
    assert_eq!(
        f.store
            .sip_completion_actors(
                &c,
                &binding,
                &f.request.conversation_id,
                &f.request.session_id,
                &held
            )
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    assert!(f
        .store
        .try_acquire_authority(&[AuthorityScope::identity(
            binding["route_owner_id"].as_str().unwrap(),
            AuthorityMode::Write
        )
        .unwrap()])
        .is_err());
    drop(held);
    let credential = binding["endpoint_credential_id"].as_str().unwrap();
    let mut actor = crate::identity::actor_from_connection(&c, credential).unwrap();
    // Preserve current media authority while removing only receive permission.
    actor.scope.operations = vec![
        "connection.offer".into(),
        "dtmf.send".into(),
        "dtmf.receive".into(),
    ];
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![
            credential,
            serde_json::to_string(&actor.scope.operations).unwrap()
        ],
    )
    .unwrap();
    let held = f.store.try_acquire_authority(&scopes).unwrap();
    assert!(Store::recorded_sip_live(&c, &binding, f.store.runtime_epoch()).unwrap());
    assert!(f
        .store
        .sip_audience_snapshot_facts(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id
        )
        .is_ok());
    assert!(f
        .store
        .sip_completion_actors(
            &c,
            &binding,
            &f.request.conversation_id,
            &f.request.session_id,
            &held
        )
        .unwrap()
        .is_none());
}

#[test]
fn initial_sip_private_slot_facts_keep_endpoint_receive_and_route_identity_separate() {
    let f = Fixture::new();
    let origin = connected(&f);
    drop(
        f.store
            .try_initial_telephone_media_read(origin.clone())
            .unwrap(),
    );
    let binding = {
        let c = f.store.connection().unwrap();
        let (_, binding, _) = f
            .store
            .initial_telephone_audience_facts(&c, &origin)
            .unwrap();
        let (scopes, actors, expiry) = f
            .store
            .sip_private_snapshot_facts(
                &c,
                &binding,
                &f.request.conversation_id,
                &f.request.session_id,
            )
            .unwrap();
        assert!(!scopes.is_empty() && expiry <= crate::now() + 5);
        assert_eq!(actors.len(), 2);
        assert_eq!(
            actors[0].identity_id,
            binding["route_owner_id"].as_str().unwrap()
        );
        assert_eq!(
            actors[1].identity_id,
            binding["identity_id"].as_str().unwrap()
        );
        assert_ne!(actors[0].identity_id, actors[1].identity_id);
        for (key, value) in [
            ("sip_call_id", json!("other-call")),
            ("membership_epoch", json!(999)),
            ("key_id", json!("fabricated-native-key")),
        ] {
            let mut forged = binding.clone();
            forged[key] = value;
            assert!(f
                .store
                .sip_private_snapshot_facts(
                    &c,
                    &forged,
                    &f.request.conversation_id,
                    &f.request.session_id
                )
                .is_err());
        }
        binding
    };
    // Returned actors are metadata; no authority reader survives the call.
    drop(
        f.store
            .try_acquire_authority(&[AuthorityScope::identity(
                binding["route_owner_id"].as_str().unwrap(),
                AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap(),
    );
    let c = f.store.connection().unwrap();
    c.execute("UPDATE credentials SET operations='[\"connection.offer\",\"dtmf.send\",\"dtmf.receive\"]' WHERE id=?1",[binding["endpoint_credential_id"].as_str().unwrap()]).unwrap();
    assert!(f
        .store
        .initial_telephone_audience_facts(&c, &origin)
        .is_ok());
    assert_eq!(
        f.store
            .sip_private_snapshot_facts(
                &c,
                &binding,
                &f.request.conversation_id,
                &f.request.session_id
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
