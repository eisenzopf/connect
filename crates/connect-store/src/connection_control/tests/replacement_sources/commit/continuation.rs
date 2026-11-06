//! Domain admission of the exact in-flight attempt; no physical route is granted.
use super::*;

fn immutable(error: rusqlite::Error, expected: &str) {
    assert!(matches!(error,
        rusqlite::Error::SqliteFailure(code, Some(message))
            if code.code == rusqlite::ErrorCode::ConstraintViolation && message == expected));
}

#[test]
fn exact_started_commit_can_read_current_public_pair_before_journal_projection() {
    let m = fixture_scoped(true, &["stream.subscribe"]);
    assert_ne!(m.f.store.runtime_epoch(), m.f.store.transition_runtime);
    let scene =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &scene)
            .unwrap();
    let activation = attempt.activation().clone();
    let expected = attempt.expected().unwrap();
    let observed = ready(&scene, &expected);
    drop(scene);
    // An ordinary entry cannot become usable just because commit has started.
    assert!(m
        .f
        .store
        .try_native_media_read(activation.origin(), None)
        .is_err());
    assert!(
        m.f.store
            .try_native_replacement_continuation_media_read(
                &m.sources[1],
                Some(&activation),
                Some((&m.sources[0], None)),
                None,
            )
            .is_err(),
        "another original cannot borrow this sealed activation"
    );
    let wrong_source =
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .unwrap();
    assert!(
        m.f.store
            .try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                Some((&m.sources[1], None)),
                Some(&wrong_source),
            )
            .is_err(),
        "another source interval cannot authorize the retained source"
    );
    drop(wrong_source);
    let foreign = Fixture::new();
    assert!(
        foreign
            .store
            .try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                Some((&m.sources[1], None)),
                None,
            )
            .is_err(),
        "another installation cannot borrow this original activation"
    );
    drop(foreign);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut attempt = Some(attempt);
    for captured in [false, true] {
        let held =
            m.f.store
                .try_native_media_read(&m.sources[1], None)
                .unwrap();
        let current =
            m.f.store
                .try_native_replacement_continuation_media_read(
                    activation.origin(),
                    Some(&activation),
                    Some((&m.sources[1], None)),
                    Some(&held),
                )
                .unwrap_or_else(|e| panic!("pending pair refused (captured={captured}): {e:?}"));
        assert!(current.current() && current.permits_subscriptions());
        assert_eq!(
            current.target().runtime_connection_id,
            activation.target().runtime_connection_id
        );
        assert_eq!(
            current.source().unwrap().runtime_connection_id,
            m.sources[1].runtime_connection_id()
        );
        drop(current);
        drop(held);
        if !captured {
            // The trusted runtime capture adds history, never a public lifetime.
            let capture = attempt
                .take()
                .unwrap()
                .committed(expected.clone(), &observed)
                .unwrap();
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap();
        }
    }
    assert_ne!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    assert!(m
        .f
        .store
        .try_native_media_read(activation.origin(), None)
        .is_err());
    assert_eq!(m.f.count("native_replacement_lifetimes"), 0);
}

#[test]
fn pending_pair_refuses_foreign_worker_attempt_and_current_authority_loss() {
    for mutation in [
        "installation-as-worker",
        "foreign-worker",
        "foreign-installation",
        "foreign-attempt",
        "cancelled",
        "expired-lease",
        "target-credential",
        "target-key",
        "original-credential",
        "retained-endpoint",
        "accepted-media",
    ] {
        let m = fixture_scoped(true, &["stream.subscribe"]);
        let scene =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &scene)
                .unwrap();
        let activation = attempt.activation().clone();
        drop(scene);
        // This is current Store eligibility only, not evidence of Core movement.
        let read = || {
            m.f.store.try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                Some((&m.sources[1], None)),
                None,
            )
        };
        assert!(read().unwrap().current(), "positive {mutation}");
        let c = m.f.store.connection().unwrap();
        let record = activation.origin().offer_record_id();
        match mutation {
            "installation-as-worker" => {
                c.execute(
                    "UPDATE operations SET worker_runtime=?2 WHERE record_id=?1",
                    params![record, m.f.store.runtime_epoch()],
                )
                .unwrap();
            }
            "foreign-worker" => {
                c.execute(
                    "UPDATE operations SET worker_runtime='another-process' WHERE record_id=?1",
                    [record],
                )
                .unwrap();
            }
            "foreign-installation" => {
                let sql = "UPDATE native_replacements SET runtime_epoch='another-installation' WHERE operation_record_id=?1";
                immutable(
                    c.execute(sql, [record]).unwrap_err(),
                    "native replacement reservation is immutable",
                );
                // Deliberate corruption of this disposable fixture only: the
                // independent domain check must refuse even without the guard.
                c.execute_batch("DROP TRIGGER native_replacement_immutable")
                    .unwrap();
                c.execute(sql, [record]).unwrap();
            }
            "foreign-attempt" => {
                let sql = "UPDATE native_replacement_steps SET attempt_id='another-attempt' WHERE operation_record_id=?1 AND phase='commit'";
                immutable(
                    c.execute(sql, [record]).unwrap_err(),
                    "native replacement step is immutable",
                );
                c.execute_batch("DROP TRIGGER native_replacement_step_immutable")
                    .unwrap();
                c.execute(sql, [record]).unwrap();
            }
            "cancelled" => {
                c.execute(
                    "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                    [record],
                )
                .unwrap();
            }
            "expired-lease" => {
                let until = crate::now() - 1;
                immutable(
                    c.execute(
                        "UPDATE native_replacements SET expires_at=?2 WHERE operation_record_id=?1",
                        params![record, until],
                    )
                    .unwrap_err(),
                    "native replacement reservation is immutable",
                );
                // Preserve equal lease/reservation deadlines while exercising
                // the real expired-clock refusal, never extending a lifetime.
                c.execute_batch("DROP TRIGGER native_replacement_immutable")
                    .unwrap();
                c.execute(
                    "UPDATE native_replacements SET expires_at=?2 WHERE operation_record_id=?1",
                    params![record, until],
                )
                .unwrap();
                c.execute(
                    "UPDATE operations SET lease_expires_at=?2 WHERE record_id=?1",
                    params![record, until],
                )
                .unwrap();
            }
            "target-credential" => {
                c.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&m.target_request.credentials[0]],
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
            "original-credential" => {
                c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&m.key])
                    .unwrap();
            }
            "retained-endpoint" => {
                c.execute("UPDATE endpoints SET verification_state='unverified' WHERE id=(SELECT endpoint_id FROM connections WHERE id=?1)", [m.sources[1].connection_id()]).unwrap();
            }
            "accepted-media" => {
                c.execute(
                    "DELETE FROM session_presence_capabilities WHERE participant_id=?1",
                    [&m.f.pids[0]],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        drop(c);
        assert!(read().is_err(), "refused {mutation}");
        assert_eq!(m.f.count("native_replacement_lifetimes"), 0);
    }
}

#[test]
fn pending_receiver_metadata_observes_subscribe_withdrawal_on_the_same_actor() {
    let m = fixture_scoped(true, &["stream.subscribe"]);
    let scene =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &scene)
            .unwrap();
    let activation = attempt.activation().clone();
    drop(scene);
    let read = |paired: bool| {
        m.f.store.try_native_replacement_continuation_media_read(
            activation.origin(),
            Some(&activation),
            paired.then_some((m.sources[1].as_ref(), None)),
            None,
        )
    };
    for paired in [false, true] {
        let current = read(paired).unwrap();
        assert!(current.current() && current.permits_subscriptions());
    }
    let c = m.f.store.connection().unwrap();
    let raw: String = c
        .query_row(
            "SELECT operations FROM credentials WHERE id=?1",
            [&m.target_request.credentials[0]],
            |r| r.get(0),
        )
        .unwrap();
    let mut operations: Vec<String> = serde_json::from_str(&raw).unwrap();
    assert!(operations.iter().any(|v| v == "stream.subscribe"));
    operations.retain(|v| v != "stream.subscribe");
    c.execute(
        "UPDATE credentials SET operations=?2 WHERE id=?1",
        params![
            m.target_request.credentials[0],
            serde_json::to_string(&operations).unwrap()
        ],
    )
    .unwrap();
    drop(c);
    let actor =
        m.f.store
            .revalidate(&m.target_request.credentials[0])
            .unwrap();
    assert!(actor.permits("connection.offer") && !actor.permits("stream.subscribe"));
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    for paired in [false, true] {
        let current = read(paired).unwrap();
        assert!(current.current());
        assert!(!current.permits_subscriptions(),
            "receiver metadata must check current subscription permission, not only current connection eligibility");
    }
    assert_eq!(m.f.count("native_replacement_lifetimes"), 0);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}
