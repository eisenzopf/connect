//! Actual four-source Store boundaries for one exact private receive packet.
use super::*;

fn selected(m: &Movement) -> Vec<Arc<NativeMediaOrigin>> {
    assert_eq!(m.sources.len(), 4);
    vec![
        m.sources[0].clone(),
        m.sources[1].clone(),
        m.sources[2].clone(),
    ]
}

#[test]
fn one_private_packet_keeps_original_owners_and_actual_third_source_under_queued_writer() {
    let (m, _) = four_plus_two();
    let subset = selected(&m);
    let full =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let target = full.media().target().runtime_connection_id.clone();
    assert!(full.permits_receive(&target, m.sources[2].runtime_connection_id()));
    let held = held_scene_source(&full, &m.sources[2]);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let stable = counts(&m);
    let scope = AuthorityScope::conversation(&m.f.cid, AuthorityMode::Write).unwrap();
    let peer = Store::open_administration(m.f.store.path(), m.f.store.limits.clone()).unwrap();
    let write = scope.clone();
    let writer = Writer(Some(std::thread::spawn(move || {
        peer.acquire_authority(&[write], &crate::AuthorityWait::new(Duration::from_secs(3)))
    })));
    crate::authority_fences::test_wait_for_writer(&m.f.store, &scope);
    let admissions = m.f.store.authority.test_admission_attempts();
    let packet =
        m.f.store
            .try_native_replacement_scene_read_under(&m.origin, &subset, &m.sources[2], &held)
            .unwrap();
    assert!(packet.current());
    assert!(packet.permits_receive(&target, m.sources[2].runtime_connection_id()));
    assert_eq!(
        fingerprint(packet.media().target()),
        fingerprint(full.media().target())
    );
    for origin in &subset {
        let actual = packet
            .existing(origin.runtime_connection_id(), None)
            .unwrap();
        let expected = full.existing(origin.runtime_connection_id(), None).unwrap();
        assert_eq!(fingerprint(actual.target()), fingerprint(expected.target()));
        assert!(Arc::ptr_eq(actual.interval(), held.interval()));
        assert!(actual.expires_at() <= held.expires_at());
    }
    assert!(packet
        .existing(m.sources[3].runtime_connection_id(), None)
        .is_none());
    assert!(!packet.permits_receive(&target, m.sources[3].runtime_connection_id()));
    assert_eq!(m.f.store.authority.test_admission_attempts(), admissions);
    // The returned full/current packet objects retain file custody only. All
    // four original SQL observer claims remain available even while they live.
    let observation = m.f.store.packet_connections.observe().unwrap();
    let mut reservations = Vec::new();
    for _ in 0..crate::packet_observation::OBSERVERS {
        reservations.push(m.f.store.packet_connections.reserve(&observation).unwrap());
    }
    assert_eq!(reservations.len(), 4);
    assert_eq!(
        m.f.store
            .packet_connections
            .reserve(&observation)
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(reservations);
    drop(observation);
    assert_eq!(counts(&m), stable);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    drop(full);
    drop(held);
    assert!(!writer.0.as_ref().unwrap().is_finished());
    drop(packet);
    writer.join();
}

#[test]
fn one_private_packet_ignores_only_unrelated_origin_and_rechecks_selected_operator_and_root() {
    for mutation in [
        "unrelated-credential",
        "selected-credential",
        "operator-credential",
        "root-cancel",
        "target-subscribe",
    ] {
        let (m, _) = four_plus_two();
        let subset = selected(&m);
        let captured =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &subset)
                .unwrap();
        let held = held_scene_source(&captured, &m.sources[2]);
        drop(captured);
        let initial =
            m.f.store
                .try_native_replacement_scene_read_under(&m.origin, &subset, &m.sources[2], &held)
                .unwrap();
        let target = initial.media().target().runtime_connection_id.clone();
        assert!(initial.permits_receive(&target, m.sources[2].runtime_connection_id()));
        drop(initial);
        {
            // Explicitly adversarial file-owner changes expose fresh-read
            // validation under the SAME still-owned original scene interval.
            // A supported coordinated Writer must remain excluded by it.
            let c = uncooperative_file_owner_at(&m.f, m.f.store.path());
            match mutation {
                "unrelated-credential" | "selected-credential" | "operator-credential" => {
                    let credential = match mutation {
                        "unrelated-credential" => &m.f.keys[3],
                        "selected-credential" => &m.f.keys[2],
                        _ => &m.key,
                    };
                    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [credential])
                        .unwrap();
                }
                "root-cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE operation_id=?1",
                        [&m.command.operation_id],
                    )
                    .unwrap();
                }
                "target-subscribe" => {
                    let credential = &m.target_request.credentials[0];
                    let original: String = c
                        .query_row(
                            "SELECT operations FROM credentials WHERE id=?1",
                            [credential],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut operations: Vec<String> = serde_json::from_str(&original).unwrap();
                    operations.retain(|name| name != "stream.subscribe");
                    c.execute(
                        "UPDATE credentials SET operations=?2 WHERE id=?1",
                        params![credential, serde_json::to_string(&operations).unwrap()],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        let stable = counts(&m);
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let read_scene = |sources: &[Arc<NativeMediaOrigin>]| {
            m.f.store.try_native_replacement_scene_read_under(
                &m.origin,
                sources,
                &m.sources[2],
                &held,
            )
        };
        let assert_refused = |sources: &[Arc<NativeMediaOrigin>], expected: &str| {
            // A refused projection can cross a SQL/policy-clock snapshot
            // boundary. Keep the original packet custody and require a
            // stable, definitive refusal within the existing fixture bound.
            let until = std::time::Instant::now() + Duration::from_secs(1);
            let expiry = held.expires_at();
            let admissions = m.f.store.authority.test_admission_attempts();
            let error = loop {
                let error = read_scene(sources).err().unwrap_or_else(|| {
                    panic!("{mutation}: withdrawn authority admitted the scene")
                });
                assert_eq!(held.expires_at(), expiry);
                assert_eq!(m.f.store.authority.test_admission_attempts(), admissions);
                if error.code != "authority-busy" || std::time::Instant::now() >= until {
                    break error;
                }
                assert_eq!(counts(&m), stable);
                assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
                std::thread::yield_now();
            };
            assert_eq!(
                error.code, expected,
                "{mutation}: exact refused observation: {error:?}"
            );
        };
        match mutation {
            "unrelated-credential" => {
                let packet = read_scene(&subset).unwrap();
                assert!(packet.current());
                assert!(packet.permits_receive(&target, m.sources[2].runtime_connection_id()));
                // The generic full scene deliberately retains its old strict
                // semantics; preparation/commit may not adopt this packet subset.
                assert_refused(&m.sources, "unauthorized");
            }
            "target-subscribe" => {
                let packet = read_scene(&subset).unwrap();
                assert!(!packet.permits_receive(&target, m.sources[2].runtime_connection_id()));
            }
            "selected-credential" | "operator-credential" => {
                assert_refused(&subset, "unauthorized");
            }
            "root-cancel" => assert_refused(&subset, "forbidden"),
            _ => unreachable!(),
        }
        assert_eq!(counts(&m), stable);
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    }
}
