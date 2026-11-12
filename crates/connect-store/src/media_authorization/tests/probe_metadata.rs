//! Actual Manager metadata exclusion versus actual kernel Writer intent.
//! Store fence/epoch tests only; no PCM, RTP or decoded quality is claimed.
use super::*;

fn completed_turns(registry: &Registry, before: usize) -> bool {
    let until = Instant::now() + Duration::from_secs(1);
    while registry.original_idle_waits.load(Ordering::Relaxed) < before + 3
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    registry.original_idle_waits.load(Ordering::Relaxed) >= before + 3
}

#[test]
fn local_metadata_exclusion_preserves_same_original_epoch_and_stamp_without_writer_intent() {
    let mut observations = Vec::new();
    for kind in 0..3 {
        let f = Fixture::new();
        let handle = f.register(Instant::now() + Duration::from_secs(10));
        let mut watcher = f.start();
        let original = admit(&handle);
        let stamp = original.stamp();
        let epoch = original.packet_epoch();
        let initial = original.current() && epoch.current();
        let (turns, current, queued, same_stamp, no_writer) = f
            .manager
            .with_probe_metadata_exclusion_for_test(&f.scope, kind, || {
                // Count AFTER exclusion, so three original watcher waits must
                // have followed actual failing probes while this lock is held.
                let before = f.registry.original_idle_waits.load(Ordering::Relaxed);
                let turns = completed_turns(&f.registry, before);
                let same = handle.admit_original(stamp);
                let same_stamp = same.as_ref().is_ok_and(Lease::current);
                drop(same);
                (
                    turns,
                    original.current(),
                    epoch.current(),
                    same_stamp,
                    f.registry.control.lock().unwrap().writers.is_empty(),
                )
            });
        let same = handle.admit_original(stamp);
        let after = same.as_ref().is_ok_and(Lease::current) && epoch.current();
        drop(same);
        drop(original);
        let shutdown = watcher.shutdown();
        observations.push((
            kind, initial, turns, current, queued, same_stamp, no_writer, after, shutdown,
        ));
    }
    // All three original watchers are joined before even the intended OLD
    // failure. A cached old epoch is never restored by this control.
    for (kind, initial, turns, current, queued, stamp, no_writer, after, shutdown) in observations {
        shutdown.unwrap();
        assert!(
            initial && turns && no_writer,
            "metadata counter missing live original/actual watcher turns/no Writer, kind={kind}"
        );
        assert!(
            current && queued && stamp && after,
            "metadata-only exclusion revoked SAME original authority/epoch/stamp, kind={kind}"
        );
    }
}

#[test]
fn true_external_kernel_writer_still_revokes_only_affected_original_and_never_restores_it() {
    let f = Fixture::new();
    let other_scope =
        AuthorityScope::identity("unaffected-probe-owner", AuthorityMode::Read).unwrap();
    let target = f.register(Instant::now() + Duration::from_secs(10));
    let other = f.register_for(&other_scope, Instant::now() + Duration::from_secs(10));
    let mut watcher = f.start();
    let original = admit(&target);
    let healthy = admit(&other);
    let stamp = original.stamp();
    let epoch = original.packet_epoch();
    let initial = original.current() && epoch.current() && healthy.current();
    let (observed, no_local_notice, healthy_during, same_refused, knowledge_preserved) =
        f.manager.with_external_probe_writer_for_test(&f.scope, || {
            let until = Instant::now() + Duration::from_secs(1);
            while epoch.current() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(1));
            }
            let observed = !epoch.current() && !original.current();
            let same_refused = target.admit_original(stamp).is_err();
            let no_local_notice = f.registry.control.lock().unwrap().writers.is_empty();
            // Once known blocked, a metadata miss must not clear that knowledge
            // or refresh a replacement through the same ongoing kernel Writer.
            let preserved = f
                .manager
                .with_probe_metadata_exclusion_for_test(&f.scope, 0, || {
                    let before = f.registry.original_idle_waits.load(Ordering::Relaxed);
                    let turns = completed_turns(&f.registry, before);
                    turns
                        && f.registry
                            .control
                            .lock()
                            .unwrap()
                            .external_scopes
                            .contains(&f.scope)
                        && target.admit().is_err()
                });
            (
                observed,
                no_local_notice,
                healthy.current(),
                same_refused,
                preserved,
            )
        });
    let old_still_refused = !epoch.current() && target.admit_original(stamp).is_err();
    let healthy_after = healthy.current();
    drop(original);
    drop(healthy);
    let fresh = admit(&target);
    let fresh_current = fresh.current();
    drop(fresh);
    watcher.shutdown().unwrap();
    assert!(initial && observed && no_local_notice && healthy_during && healthy_after);
    assert!(same_refused && old_still_refused && knowledge_preserved && fresh_current);
}

#[test]
fn namespace_storage_and_original_expiry_stay_fail_closed_without_epoch_renewal() {
    for failure in 0..3 {
        let f = Fixture::new();
        let until = Instant::now() + Duration::from_secs(if failure == 2 { 2 } else { 10 });
        let handle = f.register(until);
        let mut watcher = f.start();
        let original = admit(&handle);
        let stamp = original.stamp();
        let epoch = original.packet_epoch();
        let initial = original.current() && epoch.current() && Instant::now() < until;
        let control = match failure {
            0 => {
                f.manager.replace_probe_namespace_for_test(&f.scope);
                true
            }
            1 => f.manager.poison_probe_metadata_for_test(),
            _ => f
                .manager
                .with_probe_metadata_exclusion_for_test(&f.scope, 0, || {
                    let watchdog = Instant::now() + Duration::from_secs(3);
                    while Instant::now() < until && Instant::now() < watchdog {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Instant::now() >= until
                }),
        };
        let watchdog = Instant::now() + Duration::from_secs(1);
        while epoch.current() && Instant::now() < watchdog {
            std::thread::sleep(Duration::from_millis(1));
        }
        let refused =
            !original.current() && !epoch.current() && handle.admit_original(stamp).is_err();
        drop(original);
        watcher.shutdown().unwrap();
        assert!(
            initial && control && refused,
            "namespace/storage/expiry failed open or renewed original epoch, failure={failure}"
        );
    }
}
