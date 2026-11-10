//! Real file-reader overlap and the production historical retirement call.
//! Fixture absence is controlled Store evidence, not actual Core/media teardown.
use super::*;
use std::time::{Duration, Instant};

#[test]
fn replacement_cleanup_fair_admission_drains_continuously_overlapping_short_readers() {
    let m = fixture(true);
    let (_, origin) = installed(&m);
    let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
    let read_scopes = {
        let sql = m.f.store.connection().unwrap();
        crate::authority_admission::conversation_scopes(&sql, &m.f.cid, crate::AuthorityMode::Read)
            .unwrap()
    };
    // Each successor read is acquired BEFORE its predecessor is released. An
    // immediate writer therefore has no gap, including at its original200ms retry.
    // These are real exclusion guards; they do not supply a packet/IO permission.
    let mut reader = Some(m.f.store.try_acquire_authority(&read_scopes).unwrap());
    let (waiting, observed_wait) = std::sync::mpsc::sync_channel(1);
    let (result, attempts, handoffs, waited) = std::thread::scope(|workers| {
        let movement = &m;
        let original = &origin;
        let absence = &absent;
        let cleanup = workers.spawn(move || {
            crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = waiting.send(());
                },
                || {
                    for attempt in 1..=3 {
                        let result = movement
                            .f
                            .store
                            .settle_native_replacement_cleanup(original, absence);
                        if result.is_ok() || attempt == 3 {
                            return (result, attempt);
                        }
                        if result
                            .as_ref()
                            .err()
                            .is_none_or(|e| e.code != "authority-busy")
                        {
                            return (result, attempt);
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    unreachable!()
                },
            )
        });
        let until = Instant::now() + Duration::from_secs(2);
        let mut handoffs = 0;
        let mut waited = false;
        while !cleanup.is_finished() && Instant::now() < until {
            if observed_wait.try_recv().is_ok() {
                // Release the last old reader ONLY after a real WouldBlock in
                // the waited-lock path. An immediate attempt briefly owns an
                // admission lock too; observing that alone would let the old
                // code win a race and would not be a causal counterexample.
                waited = true;
                drop(reader.take());
                break;
            }
            match m.f.store.try_acquire_authority(&read_scopes) {
                Ok(next) => {
                    let previous = reader.replace(next);
                    drop(previous);
                    handoffs += 1;
                }
                Err(error) if error.code == "authority-busy" => {
                    // Keep the existing reader alive through any transient
                    // reservation; it is released only by the notification.
                }
                Err(error) => panic!("actual reader admission failed: {error:?}"),
            }
            std::thread::yield_now();
        }
        let (result, attempts) = cleanup.join().unwrap();
        (result, attempts, handoffs, waited)
    });
    // Original cleanup worker is joined before causal assertions; release every
    // remaining original reader even when the old implementation refused.
    drop(reader);
    eprintln!(
        "retirement overlap: attempts={attempts} handoffs={handoffs} waited={waited} result={:?}",
        result.as_ref().err().map(|e| &e.code)
    );
    assert!(
        waited,
        "retirement never entered the actual bounded file-wait path"
    );
    result.expect("continuously overlapping short readers starved original retirement");
    assert_eq!(
        state(&m, &origin),
        ("retired".into(), origin.generation() + 1, true)
    );
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], None)
        .unwrap()
        .current());
    let settled = Store::inspect_snapshot(m.f.store.path()).unwrap();
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), settled);
}

#[test]
fn replacement_cleanup_long_reader_releases_short_reservation_without_changing_facts() {
    let m = fixture(true);
    let (_, origin) = installed(&m);
    let absent = Gone(origin.runtime_connection_id().into(), origin.generation());
    let held =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let began = Instant::now();
    let result =
        m.f.store
            .settle_native_replacement_cleanup(&origin, &absent);
    let elapsed = began.elapsed();
    // The old read is STILL alive after refusal. No five-millisecond writer
    // reservation or partial Identity scope may survive into another packet read.
    let packet = m.f.store.try_native_media_read(&m.sources[1], None);
    let packet_current = packet.as_ref().is_ok_and(|p| p.current());
    drop(packet);
    drop(held);
    assert_eq!(result.unwrap_err().code, "authority-busy");
    assert!(
        elapsed < Duration::from_millis(100),
        "short reservation outlived bounded admission: {elapsed:?}"
    );
    assert!(
        packet_current,
        "refused cleanup retained a writer behind its old reader"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    m.f.store
        .settle_native_replacement_cleanup(&origin, &absent)
        .unwrap();
    assert_eq!(
        state(&m, &origin),
        ("retired".into(), origin.generation() + 1, true)
    );
}
