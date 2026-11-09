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
        "source-generation",
        "target-generation",
        "source-expiry",
        "target-expiry",
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
        continuation_actual_hit(&activation, || read());
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
            "source-generation" => {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [m.sources[1].connection_id()],
                )
                .unwrap();
            }
            "target-generation" => {
                // The reservation is immutable; corrupt this disposable fixture
                // only after proving its trigger refuses the ordinary update.
                let sql = "UPDATE native_replacements SET generation=generation+1 WHERE operation_record_id=?1";
                immutable(
                    c.execute(sql, [record]).unwrap_err(),
                    "native replacement reservation is immutable",
                );
                c.execute_batch("DROP TRIGGER native_replacement_immutable")
                    .unwrap();
                c.execute(sql, [record]).unwrap();
            }
            "source-expiry" => {
                let credential: String = c.query_row("SELECT n.credential_id FROM native_admission_sources n JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id WHERE l.connection_id=?1 AND l.retired=0", [m.sources[1].connection_id()], |row| row.get(0)).unwrap();
                c.execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![credential, crate::now() - 1],
                )
                .unwrap();
            }
            "target-expiry" => {
                c.execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![m.target_request.credentials[0], crate::now() - 1],
                )
                .unwrap();
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

fn continuation_actual_hit(
    activation: &crate::NativeReplacementActivation,
    mut read: impl FnMut() -> Result<crate::NativeMediaRead>,
) {
    for _ in 0..8 {
        let before = activation.continuation_memo_counts_for_test();
        drop(read().unwrap());
        let after = activation.continuation_memo_counts_for_test();
        if after[1] > before[1] {
            return;
        }
    }
    panic!("no actual unchanged-version continuation memo hit");
}

#[test]
fn pending_continuation_memo_reuses_facts_but_reacquires_current_authority_on_every_observer() {
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
    let snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let writer = crate::AuthorityScope::conversation(
        activation.origin().conversation_id(),
        crate::AuthorityMode::Write,
    )
    .unwrap();
    for selected in 0..crate::packet_observation::OBSERVERS {
        let occupied: Vec<_> =
            m.f.store
                .packet_connections
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != selected)
                .map(|(_, slot)| slot.lock().unwrap())
                .collect();
        let read = || {
            m.f.store.try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                Some((&m.sources[1], None)),
                None,
            )
        };
        let cold = read().unwrap();
        let expected_target = serde_json::to_value(cold.target().connection.clone()).unwrap();
        let expected_source =
            serde_json::to_value(cold.source().unwrap().connection.clone()).unwrap();
        drop(cold);
        continuation_actual_hit(&activation, || read());
        let before = m.f.store.authority.test_admission_attempts();
        let warm = read().unwrap();
        assert_eq!(
            m.f.store.authority.test_admission_attempts() - before,
            1,
            "an unchanged SQL memo must acquire one NEW file authority interval"
        );
        assert_eq!(
            serde_json::to_value(warm.target().connection.clone()).unwrap(),
            expected_target
        );
        assert_eq!(
            serde_json::to_value(warm.source().unwrap().connection.clone()).unwrap(),
            expected_source
        );
        assert!(warm.current() && warm.permits_subscriptions());
        assert_eq!(
            m.f.store
                .try_acquire_authority(&[writer.clone()])
                .unwrap_err()
                .code,
            "authority-busy"
        );
        drop(warm);
        drop(m.f.store.try_acquire_authority(&[writer.clone()]).unwrap());
        // An acquired writer also excludes a warmed positive; no old reader is
        // retained by the memo and no cached grant can bypass current admission.
        let held_writer = m.f.store.try_acquire_authority(&[writer.clone()]).unwrap();
        assert_eq!(read().err().unwrap().code, "authority-busy");
        drop(held_writer);
        assert!(read().unwrap().current());
        drop(occupied);
    }
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), snapshot);
}

#[test]
fn pending_continuation_memo_separates_phase_source_and_observes_external_commits() {
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
    let occupied: Vec<_> =
        m.f.store
            .packet_connections
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 0)
            .map(|(_, slot)| slot.lock().unwrap())
            .collect();
    let read = || {
        m.f.store.try_native_replacement_continuation_media_read(
            activation.origin(),
            Some(&activation),
            Some((&m.sources[1], None)),
            None,
        )
    };
    continuation_actual_hit(&activation, || read());
    // A warmed pending phase never authorizes the ordinary origin before the
    // durable commit; removing or changing the actual activation cannot borrow it.
    assert!(m
        .f
        .store
        .try_native_media_read(activation.origin(), None)
        .is_err());
    assert!(m
        .f
        .store
        .try_native_replacement_continuation_media_read(
            activation.origin(),
            None,
            Some((&m.sources[1], None)),
            None
        )
        .is_err());
    assert!(m
        .f
        .store
        .try_native_replacement_continuation_media_read(
            &m.sources[1],
            Some(&activation),
            Some((&m.sources[0], None)),
            None
        )
        .is_err());
    let wrong_source =
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .unwrap();
    assert!(m
        .f
        .store
        .try_native_replacement_continuation_media_read(
            activation.origin(),
            Some(&activation),
            Some((&m.sources[1], None)),
            Some(&wrong_source)
        )
        .is_err());
    drop(wrong_source);
    // The unary namespace must not inherit a paired source. Both are bounded
    // entries of the existing memo, with the original None/Weak source key.
    let unary =
        m.f.store
            .try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                None,
                None,
            )
            .unwrap();
    assert!(unary.source().is_none());
    drop(unary);
    assert!(read().unwrap().source().is_some());
    continuation_actual_hit(&activation, || read());
    // Time itself can withdraw a credential without a SQL commit. A positive
    // memo must never turn its captured expiry into an authorization TTL.
    let external = rusqlite::Connection::open(m.f.store.path()).unwrap();
    let original_expiry: i64 = external
        .query_row(
            "SELECT expires_at FROM credentials WHERE id=?1",
            [&m.target_request.credentials[0]],
            |row| row.get(0),
        )
        .unwrap();
    let short_expiry = crate::now() + 2;
    assert!(short_expiry < original_expiry);
    external
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![m.target_request.credentials[0], short_expiry],
        )
        .unwrap();
    continuation_actual_hit(&activation, || read());
    let short = read().unwrap();
    assert!(short.expires_at() <= short_expiry);
    drop(short);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while crate::now() < short_expiry {
        assert!(
            std::time::Instant::now() < until,
            "original expiry clock did not advance"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(read().err().unwrap().code, "unauthorized");
    external
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![m.target_request.credentials[0], original_expiry],
        )
        .unwrap();
    continuation_actual_hit(&activation, || read());
    // A second handle in this process is also a distinct writer. It cannot be
    // hidden behind a Manager-local epoch or this observer's total_changes.
    // Exercise its reversible expiry changes before the final revocation.
    let admin = Store::open_administration(m.f.store.path(), crate::Limits::default()).unwrap();
    let c = admin.connection().unwrap();
    c.execute(
        "UPDATE credentials SET expires_at=?2 WHERE id=?1",
        params![m.target_request.credentials[0], crate::now() - 1],
    )
    .unwrap();
    drop(c);
    assert_eq!(read().err().unwrap().code, "unauthorized");
    let c = admin.connection().unwrap();
    c.execute(
        "UPDATE credentials SET expires_at=?2 WHERE id=?1",
        params![m.target_request.credentials[0], original_expiry],
    )
    .unwrap();
    drop(c);
    drop(admin);
    continuation_actual_hit(&activation, || read());
    let before = activation.continuation_memo_counts_for_test();
    // This independent SQLite connection has its own total_changes value:
    // PRAGMA data_version, not an in-process notification, must invalidate us.
    // Revocation is intentionally final; the schema forbids unrevocation.
    external
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&m.target_request.credentials[0]],
        )
        .unwrap();
    assert_eq!(read().err().unwrap().code, "unauthorized");
    let after = activation.continuation_memo_counts_for_test();
    assert!(
        after[0] > before[0],
        "external commit must force fresh current derivation"
    );
    drop(external);
    drop(occupied);
}

#[test]
fn pending_continuation_positive_memo_keeps_phase_source_and_expiry_across_clock_steps() {
    let base = crate::now();
    let clock = crate::test_clock::Clock::at(base);
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
    let external = rusqlite::Connection::open(m.f.store.path()).unwrap();
    external
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![m.target_request.credentials[0], base + 4],
        )
        .unwrap();
    for selected in 0..crate::packet_observation::OBSERVERS {
        clock.set(base);
        let _occupied: Vec<_> =
            m.f.store
                .packet_connections
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != selected)
                .map(|(_, slot)| slot.lock().unwrap())
                .collect();
        let read = || {
            m.f.store.try_native_replacement_continuation_media_read(
                activation.origin(),
                Some(&activation),
                Some((&m.sources[1], None)),
                None,
            )
        };
        let cold = read().unwrap();
        let expires = cold.expires_at();
        assert!(expires > base + 2 && expires <= base + 4);
        let expected = serde_json::to_value(cold.target().connection.clone()).unwrap();
        drop(cold);
        let before = activation.continuation_memo_counts_for_test();
        for second in [base + 2, base + 1] {
            clock.set(second);
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
                    .unwrap();
            assert!(current.current() && current.permits_subscriptions());
            assert_eq!(current.expires_at(), expires);
            assert_eq!(
                serde_json::to_value(current.target().connection.clone()).unwrap(),
                expected
            );
            assert!(std::sync::Arc::ptr_eq(current.interval(), held.interval()));
        }
        let after = activation.continuation_memo_counts_for_test();
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], before[1] + 2);
        assert!(m
            .f
            .store
            .try_native_media_read(activation.origin(), None)
            .is_err());
        assert!(m
            .f
            .store
            .try_native_replacement_continuation_media_read(
                &m.sources[1],
                Some(&activation),
                Some((&m.sources[0], None)),
                None
            )
            .is_err());
    }
    clock.set(base + 4);
    assert!(m
        .f
        .store
        .try_native_replacement_continuation_media_read(
            activation.origin(),
            Some(&activation),
            Some((&m.sources[1], None)),
            None
        )
        .is_err());
}
