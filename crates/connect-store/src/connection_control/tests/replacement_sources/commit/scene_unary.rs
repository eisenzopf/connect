//! Real current Store projections for three origins, never Core IO evidence.
use super::*;
fn scene() -> Movement {
    let mut f = Fixture::new();
    for index in 0..3 {
        let identity = f.store.revalidate(&f.keys[index]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[index] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let sources = (0..3)
        .map(|index| consultation_reads::ready(&f, index))
        .collect::<Vec<_>>();
    let target = endpoint(&f, vec!["audio".into()], "replacement");
    let key = key(&f);
    let revision = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "three-source-current-scene".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target),
            candidate_connid: None,
            retained_connid: sources[1].connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let prepared = super::super::allocation::prepared_target_for_scene(
        &["stream.subscribe"],
        (f, key, command, origin, sources),
    );
    fixture_prepared(true, prepared)
}
fn fingerprint(view: &crate::PeerConnectionProjection) -> serde_json::Value {
    json!({"session":{"view":view.session.session,"actor":view.session.actor,
        "conversation_revision":view.session.conversation_revision,"present":view.session.present},
        "connection":view.connection,"runtime":view.runtime_connection_id,"device":view.device_id,
        "endpoint_address":view.endpoint_address,"endpoint_transport":view.endpoint_transport,
        "endpoint_capabilities":view.endpoint_capabilities,"accepted_capabilities":view.accepted_capabilities,
        "presence_role":view.presence_role,"offer":view.offer,"private_consultation":view.private_consultation,
        "handoff_preparation":view.handoff_preparation,"publication_suspended":view.publication_suspended})
}
fn counts(m: &Movement) -> (u64, u64, u64, u64) {
    (
        m.f.count("operations"),
        m.f.count("operation_effects"),
        m.f.count("native_replacement_steps"),
        m.f.count("journal"),
    )
}
#[test]
fn three_source_scene_reads_preserve_exact_bindings_pairs_on_repeated_same_root_capture() {
    let m = scene();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let stable = counts(&m);
    for _ in 0..2 {
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        assert!(read.current());
        assert_eq!(read.media().target().connection.generation, 2);
        assert_eq!(
            read.media().target().connection.connection_id,
            "replacement-next"
        );
        for target in &m.sources {
            let ordinary = m.f.store.try_native_media_read(target, None).unwrap();
            assert_eq!(
                fingerprint(
                    read.existing(target.runtime_connection_id(), None)
                        .unwrap()
                        .target()
                ),
                fingerprint(ordinary.target())
            );
            for source in &m.sources {
                if Arc::ptr_eq(target, source) {
                    continue;
                }
                let pair =
                    m.f.store
                        .try_native_media_read(target, Some(source))
                        .unwrap();
                let captured = read
                    .existing(
                        target.runtime_connection_id(),
                        Some(source.runtime_connection_id()),
                    )
                    .unwrap();
                assert_eq!(fingerprint(captured.target()), fingerprint(pair.target()));
                assert_eq!(
                    fingerprint(captured.source().unwrap()),
                    fingerprint(pair.source().unwrap())
                );
                assert_eq!(
                    captured.permits_subscriptions(),
                    pair.permits_subscriptions()
                );
            }
        }
        assert!(
            read.permits_receive(
                &read.media().target().runtime_connection_id,
                m.sources[2].runtime_connection_id()
            ),
            "nonretained third source must be represented under current target permission"
        );
    }
    assert_eq!(counts(&m), stable);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
#[test]
fn next_scene_call_observes_target_permission_withdrawal_and_restoration_without_old_positive_facts(
) {
    let m = scene();
    let candidate = {
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let id = read.media().target().runtime_connection_id.clone();
        assert!(read.permits_receive(&id, m.sources[2].runtime_connection_id()));
        id
    };
    let credential = &m.target_request.credentials[0];
    let original: String =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT operations FROM credentials WHERE id=?1",
                [credential],
                |row| row.get(0),
            )
            .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&original).unwrap();
    operations.retain(|operation| operation != "stream.subscribe");
    m.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![credential, serde_json::to_string(&operations).unwrap()],
        )
        .unwrap();
    let before = counts(&m);
    {
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        assert!(!read.permits_receive(&candidate, m.sources[2].runtime_connection_id()));
    }
    assert_eq!(counts(&m), before);
    m.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations=?2 WHERE id=?1",
            params![credential, original],
        )
        .unwrap();
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    assert!(read.permits_receive(&candidate, m.sources[2].runtime_connection_id()));
    assert_eq!(counts(&m), before);
}
#[test]
fn three_source_scene_rechecks_nonretained_generation_revocation_and_original_cancel() {
    for change in [
        "third-generation",
        "third-credential",
        "third-expiry",
        "original-cancel",
    ] {
        let m = scene();
        drop(
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap(),
        );
        let c = m.f.store.connection().unwrap();
        match change {
            "third-generation" => {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [m.sources[2].connection_id()],
                )
                .unwrap();
            }
            "third-credential" => {
                c.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&m.f.keys[2]],
                )
                .unwrap();
            }
            "third-expiry" => {
                c.execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![m.f.keys[2], crate::now() - 1],
                )
                .unwrap();
            }
            "original-cancel" => {
                c.execute("UPDATE operations SET cancel_requested=1 WHERE record_id=(SELECT operation_record_id FROM native_replacements WHERE candidate_connection_id=?1)",["replacement-next"]).unwrap();
            }
            _ => unreachable!(),
        }
        drop(c);
        let before = counts(&m);
        assert!(
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .is_err(),
            "{change}"
        );
        assert_eq!(counts(&m), before);
    }
}
#[test]
fn scene_consolidation_does_not_bypass_original_receiver_epoch_commit_phase_guard() {
    let m = scene();
    drop(
        m.f.store
            .try_native_replacement_receiver_epoch_read(&m.origin)
            .unwrap(),
    );
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    drop(read);
    let before = counts(&m);
    assert_eq!(
        m.f.store
            .try_native_replacement_receiver_epoch_read(&m.origin)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    assert_eq!(counts(&m), before);
    drop(attempt);
}

#[test]
fn consolidated_scene_refuses_duplicate_missing_and_foreign_originals_without_effects() {
    let m = scene();
    let foreign = scene();
    let before = counts(&m);
    let mut duplicate = m.sources.clone();
    duplicate.push(m.sources[2].clone());
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &duplicate)
        .is_err());
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &m.sources[1..])
        .is_err());
    let mut other = m.sources.clone();
    other[2] = foreign.sources[2].clone();
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &other)
        .is_err());
    assert_eq!(counts(&m), before);
}

#[test]
fn packet_capture_source_context_matches_independent_generic_media_read_without_publication() {
    let m = scene();
    let before = counts(&m);
    let snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    for _ in 0..2 {
        let scene =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let generic =
            m.f.store
                .try_native_replacement_media_read(&m.origin)
                .unwrap();
        assert!(scene.current() && generic.current());
        let current = scene.media().source();
        assert_eq!(current.context(), generic.source().context());
        assert_eq!(
            fingerprint(current.retained().target()),
            fingerprint(generic.source().retained().target())
        );
        assert_eq!(
            fingerprint(current.source().native().unwrap().target()),
            fingerprint(generic.source().source().native().unwrap().target())
        );
        assert_eq!(
            current.retained().permits_subscriptions(),
            generic.source().retained().permits_subscriptions()
        );
        assert_eq!(
            fingerprint(scene.media().target()),
            fingerprint(generic.target())
        );
        assert_eq!(m.f.count("native_replacement_lifetimes"), 0);
    }
    assert_eq!(counts(&m), before);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), snapshot);
}

#[test]
fn completed_packet_capture_does_not_reuse_operator_endpoint_key_or_cancel_authority() {
    // Expired/revoked credentials fail current authentication; a missing
    // permission or invalid signed key/endpoint/operation remains forbidden.
    // Keep a strict code for every original mutation, not any-error success.
    for (mutation, expected_code) in [
        ("operator-scope", "forbidden"),
        ("operator-expiry", "unauthorized"),
        ("retained-credential", "unauthorized"),
        ("target-key", "forbidden"),
        ("target-endpoint", "forbidden"),
        ("original-cancel", "forbidden"),
    ] {
        let m = scene();
        drop(
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap(),
        );
        if mutation == "original-cancel" {
            cancel(&m.f, &m.key, &m.command);
        } else {
            let c = m.f.store.connection().unwrap();
            match mutation {
                "operator-scope" => {
                    let raw: String = c
                        .query_row(
                            "SELECT operations FROM credentials WHERE id=?1",
                            [&m.key],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut operations: Vec<String> = serde_json::from_str(&raw).unwrap();
                    assert!(operations.iter().any(|op| op == "connection.replace"));
                    operations.retain(|op| op != "connection.replace");
                    c.execute(
                        "UPDATE credentials SET operations=?2 WHERE id=?1",
                        params![m.key, serde_json::to_string(&operations).unwrap()],
                    )
                    .unwrap();
                }
                "operator-expiry" => {
                    c.execute(
                        "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                        params![m.key, crate::now() - 1],
                    )
                    .unwrap();
                }
                "retained-credential" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&m.f.keys[1]],
                    )
                    .unwrap();
                }
                "target-key" => {
                    c.execute(
                        "UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1",
                        [&m.target_request.key_id],
                    )
                    .unwrap();
                }
                "target-endpoint" => {
                    let TransitionIntent::Replace {
                        endpoint_id: Some(endpoint),
                        ..
                    } = &m.command.intent
                    else {
                        panic!("fixture must retain the original provisioned target");
                    };
                    c.execute(
                        "UPDATE endpoints SET verification_state='unverified' WHERE id=?1",
                        [endpoint],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let before = counts(&m);
        let snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert_eq!(
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .err()
                .unwrap()
                .code,
            expected_code,
            "{mutation}"
        );
        assert_eq!(counts(&m), before, "{mutation}");
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            snapshot,
            "{mutation}"
        );
    }
}

#[test]
fn packet_capture_retained_receiver_permission_changes_are_fresh_without_replaying_effects() {
    let m = scene();
    let before = counts(&m);
    let retained = m.sources[1].runtime_connection_id();
    let third = m.sources[2].runtime_connection_id();
    let original: String =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT operations FROM credentials WHERE id=?1",
                [&m.f.keys[1]],
                |row| row.get(0),
            )
            .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&original).unwrap();
    assert!(operations.iter().any(|op| op == "stream.subscribe"));
    operations.retain(|op| op != "stream.subscribe");
    for (encoded, expected) in [
        (original.clone(), true),
        (serde_json::to_string(&operations).unwrap(), false),
        (original, true),
    ] {
        m.f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![m.f.keys[1], encoded],
            )
            .unwrap();
        let scene =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        assert!(scene.current());
        assert_eq!(
            scene.media().source().retained().permits_subscriptions(),
            expected
        );
        assert_eq!(scene.permits_receive(retained, third), expected);
        let generic =
            m.f.store
                .try_native_replacement_media_read(&m.origin)
                .unwrap();
        assert_eq!(
            generic.source().retained().permits_subscriptions(),
            expected
        );
        assert!(
            scene.permits_receive(&scene.media().target().runtime_connection_id, third),
            "retained receiver scope cannot replace the independent candidate receiver permission"
        );
        assert_eq!(counts(&m), before);
    }
}

mod custody;
