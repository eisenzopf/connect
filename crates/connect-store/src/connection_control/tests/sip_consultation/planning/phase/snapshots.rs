use super::*;
use crate::{
    InitialTelephoneMediaSnapshot, InitialTelephoneMediaSnapshotLease, InitialTelephonePacketPhase,
};
use std::time::{Duration, Instant};
// Cold enrollment may contend with the shared watcher before inserting any slot.
// Retry only that effect-free refusal, retaining the exact original origins and
// a fixed setup cutoff. Packet admission assertions never use this helper.
fn enroll<T>(until: Instant, mut register: impl FnMut() -> connect_protocol::Result<T>) -> T {
    loop {
        assert!(Instant::now() < until, "snapshot enrollment deadline");
        match register() {
            Ok(snapshot) => return snapshot,
            Err(error) if error.code == "authority-busy" => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("snapshot enrollment refused: {}", error.code),
        }
    }
}
fn current(snapshot: &InitialTelephoneMediaSnapshot) -> InitialTelephoneMediaSnapshotLease {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Ok(read) => return read,
            Err(e) if e.code == "authority-busy" => {}
            Err(e) => panic!("initial snapshot refused: {}", e.code),
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn refused(snapshot: &InitialTelephoneMediaSnapshot) {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Err(e) if e.code == "forbidden" => return,
            Err(e) if e.code == "authority-busy" => {}
            _ => panic!("unapplied initial phase must not publish packet authority"),
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn initial_private_snapshot_refuses_quarantined_and_prepared_packet_phases() {
    for prepared in [false, true] {
        let s = Scene::with_controller(Some("snapshot-private-unapplied"));
        let _capture = prepared.then(|| s.applied());
        assert_eq!(
            s.phase().unwrap(),
            if prepared { Prepared } else { Quarantined }
        );
        let phone = s.phone.clone();
        let store = Arc::new(s.f.store);
        let mut watcher = store.start_media_authorization_watcher().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let setup_until = Instant::now() + Duration::from_secs(2);
            let snapshot = enroll(setup_until, || {
                store.try_register_initial_telephone_media_snapshot(phone.clone(), None)
            });
            refused(&snapshot);
        }));
        watcher.shutdown().unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
#[test]
fn initial_private_snapshot_preserves_applied_to_established_epoch_and_sql_free_delivery() {
    let s = Scene::with_controller(Some("snapshot-private-lifecycle"));
    let receipt = s.applied();
    s.f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    let phone = s.phone.clone();
    let native = s.native[0].clone();
    let foreign = s.native[1].clone();
    let store = Arc::new(s.f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let setup_until = Instant::now() + Duration::from_secs(2);
        let snapshot = enroll(setup_until, || {
            store.try_register_initial_telephone_media_snapshot(phone.clone(), None)
        });
        let pair = enroll(setup_until, || {
            store.try_register_initial_telephone_media_snapshot(phone.clone(), Some(native.clone()))
        });
        let unrelated = enroll(setup_until, || {
            store
                .try_register_initial_telephone_media_snapshot(phone.clone(), Some(foreign.clone()))
        });
        refused(&unrelated);
        drop(unrelated);
        let original = current(&snapshot);
        let original_pair = current(&pair);
        assert_eq!(
            original.phase(),
            InitialTelephonePacketPhase::PrivateApplied
        );
        assert_eq!(
            original_pair.phase(),
            InitialTelephonePacketPhase::PrivateApplied
        );
        let old_epoch = original.packet_epoch();
        let old_pair = original_pair.packet_epoch();
        {
            let _sql = store.connection().unwrap();
            let observation = store.packet_connections.observe().unwrap();
            let _observers = (0..crate::packet_observation::OBSERVERS)
                .map(|_| store.try_packet_connection(&observation).unwrap())
                .collect::<Vec<_>>();
            for _ in 0..32 {
                let read = snapshot.admit().unwrap();
                assert!(read.current() && read.stamp() == original.stamp());
                assert_eq!(read.phone().expires_at(), original.phone().expires_at());
                let read = pair.admit().unwrap();
                assert!(read.current() && read.stamp() == original_pair.stamp());
                assert_eq!(
                    read.native().unwrap().target().runtime_connection_id,
                    native.runtime_connection_id()
                );
            }
        }
        drop(original);
        drop(original_pair);
        store.record_native_consultation_decoded(&receipt).unwrap();
        // The watcher can own primary SQL immediately after decoded-state
        // invalidation. Match the Service's bounded same-receipt completion
        // retry, only for this definite pre-DML SQL-mutex refusal.
        let completion_until = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < completion_until,
                "original completion deadline"
            );
            match store.complete_native_consultation(&receipt) {
                Ok(record) => {
                    record.unwrap();
                    break;
                }
                Err(error)
                    if error.code == "authority-busy"
                        && error.message == "Current state is busy"
                        && Instant::now() < completion_until =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("original consultation completion refused: {error:?}"),
            }
        }
        let current = current(&snapshot);
        assert_eq!(
            current.phase(),
            InitialTelephonePacketPhase::PrivateEstablished
        );
        assert!(current.phone().is_private_audio() && !current.phone().is_private_admission());
        assert!(!old_epoch.current() && !old_pair.current());
        drop(current);
        drop(pair);
        drop(snapshot);
    }));
    watcher.shutdown().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
