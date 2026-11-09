use super::*;

#[test]
fn media_facts_clock_match_keeps_original_interval_and_exact_database_source() {
    let f = Fixture::new();
    let observation = f.store.packet_connections.observe().unwrap();
    let c = f.store.try_packet_connection(&observation).unwrap();
    let captured = Version {
        second: 100,
        ..version(&c, c.id()).unwrap()
    };
    for second in [100, 120, 110, 129] {
        assert!(media_clock_matches(
            captured,
            Version { second, ..captured },
            130
        ));
    }
    for second in [99, 130, 131] {
        assert!(!media_clock_matches(
            captured,
            Version { second, ..captured },
            130
        ));
    }
    for changed in [
        Version {
            external: captured.external + 1,
            ..captured
        },
        Version {
            schema: captured.schema + 1,
            ..captured
        },
        Version {
            changes: captured.changes + 1,
            ..captured
        },
    ] {
        assert!(!media_clock_matches(captured, changed, 130));
    }
    let another = f.store.try_packet_connection(&observation).unwrap();
    assert_ne!(c.id(), another.id());
    assert!(!media_clock_matches(
        captured,
        Version {
            connection: another.id(),
            ..captured
        },
        130
    ));
    // The generic matcher still rejects a different second and exact-source
    // identity still distinguishes same-shaped but separately owned tokens.
    let source: MetadataToken = Arc::new(7u8);
    let other: MetadataToken = Arc::new(7u8);
    let generic = Entry {
        version: captured,
        source: Some(Arc::downgrade(&source)),
        facts: Arc::new(1i64),
        scopes: vec![],
    };
    assert!(generic.matches(captured, Some(&source)));
    assert!(!generic.matches(
        Version {
            second: 101,
            ..captured
        },
        Some(&source)
    ));
    assert!(!generic.same_source(Some(&other)));
    assert!(!generic.same_source(None));
    let real = crate::now();
    let clock = crate::test_clock::Clock::at(100);
    assert_eq!(crate::now(), 100);
    let panic = std::panic::catch_unwind(|| {
        let _nested = crate::test_clock::Clock::at(200);
        assert_eq!(crate::now(), 200);
        panic!("controlled scope unwinding");
    });
    assert!(panic.is_err());
    assert_eq!(crate::now(), 100);
    std::thread::scope(|scope| {
        scope
            .spawn(|| assert!(crate::now() >= real))
            .join()
            .unwrap()
    });
    drop(clock);
    assert!(crate::now() >= real);
}
