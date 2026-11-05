//! Domain admission of the exact in-flight attempt; no physical route is granted.
use super::*;

#[test]
fn exact_started_commit_can_read_current_public_pair_before_journal_projection() {
    let m = fixture(true);
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
