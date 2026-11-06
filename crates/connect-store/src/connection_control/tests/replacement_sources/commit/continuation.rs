//! Domain admission of the exact in-flight attempt; no physical route is granted.
use super::*;

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
                c.execute("UPDATE native_replacements SET runtime_epoch='another-installation' WHERE operation_record_id=?1", [record]).unwrap();
            }
            "foreign-attempt" => {
                c.execute("UPDATE native_replacement_steps SET attempt_id='another-attempt' WHERE operation_record_id=?1 AND phase='commit'", [record]).unwrap();
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
