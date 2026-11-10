use super::*;
use crate::{
    AuthorityMode, AuthorityScope, AuthorityWait, TelephoneMediaSnapshot,
    TelephoneMediaSnapshotLease,
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
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
fn admit(snapshot: &TelephoneMediaSnapshot) -> TelephoneMediaSnapshotLease {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Ok(read) => return read,
            Err(e) if e.code == "authority-busy" => {}
            Err(e) => panic!("snapshot refused: {}", e.code),
        }
        assert!(Instant::now() < until, "snapshot did not publish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture(
    run: impl FnOnce(&Arc<Store>, Arc<crate::TelephoneMediaOrigin>, Arc<crate::NativeMediaOrigin>),
) {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (_, capture) = attempt(&f, &origin, &sources, &key);
    f.store
        .record_telephone_replacement_committed(&capture)
        .unwrap();
    let phone = f
        .store
        .project_telephone_replacement_committed(&capture)
        .unwrap();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&store, phone, sources[1].clone())
    }));
    watcher.shutdown().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
#[test]
fn established_sip_unary_and_pair_delivery_do_not_reenter_sql() {
    fixture(|store, phone, native| {
        let setup_until = Instant::now() + Duration::from_secs(2);
        let unary = enroll(setup_until, || {
            store.try_register_telephone_media_snapshot(phone.clone(), None)
        });
        let pair = enroll(setup_until, || {
            store.try_register_telephone_media_snapshot(phone.clone(), Some(native.clone()))
        });
        let original = admit(&unary);
        let original_pair = admit(&pair);
        let _sql = store.connection().unwrap();
        let observation = store.packet_connections.observe().unwrap();
        let _observers = (0..crate::packet_observation::OBSERVERS)
            .map(|_| store.try_packet_connection(&observation).unwrap())
            .collect::<Vec<_>>();
        for _ in 0..32 {
            let read = unary.admit().unwrap();
            assert!(read.current() && read.native().is_none());
            assert!(read.stamp() == original.stamp());
            assert_eq!(
                read.telephone().snapshot().generation,
                phone.snapshot().generation
            );
            assert_eq!(
                read.telephone().expires_at(),
                original.telephone().expires_at()
            );
            let read = pair.admit().unwrap();
            assert!(read.current() && read.stamp() == original_pair.stamp());
            assert_eq!(
                read.native().unwrap().target().runtime_connection_id,
                native.runtime_connection_id()
            );
            assert_eq!(
                read.permits_native_receive(),
                original_pair.permits_native_receive()
            );
            assert_eq!(
                read.permits_telephone_receive(),
                original_pair.permits_telephone_receive()
            );
        }
    });
}
#[test]
fn established_sip_snapshot_io_drains_but_queued_epoch_never_reauthorizes() {
    fixture(|store, phone, native| {
        let setup_until = Instant::now() + Duration::from_secs(2);
        let snapshot = enroll(setup_until, || {
            store.try_register_telephone_media_snapshot(phone.clone(), Some(native.clone()))
        });
        let read = admit(&snapshot);
        let epoch = read.packet_epoch();
        let cid = phone.snapshot().conversation_id.clone();
        let other = store.clone();
        let (waited_tx, waited_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let held = crate::authority_fences::with_waited_lock_probe(
                move || {
                    let _ = waited_tx.send(());
                },
                || {
                    other.acquire_authority(
                        &[AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap()],
                        &AuthorityWait::new(Duration::from_secs(2)),
                    )
                },
            );
            let _ = acquired_tx.send(held.is_ok());
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
            drop(held);
        });
        let actually_waited = waited_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        let old_revoked = !epoch.current() && !read.current() && snapshot.admit().is_err();
        let held_io_excludes = acquired_rx.try_recv().is_err();
        drop(read);
        let acquired = acquired_rx.recv_timeout(Duration::from_secs(1)) == Ok(true);
        let _ = release_tx.send(());
        writer.join().unwrap();
        assert!(actually_waited && old_revoked && held_io_excludes && acquired);
        let fresh = admit(&snapshot);
        assert!(
            fresh.current() && !epoch.current(),
            "same logical pair must not renew old queued epoch"
        );
    });
}
#[test]
fn established_sip_snapshot_route_revocation_refuses_without_reborrowing_retired_source() {
    fixture(|store, phone, _| {
        let setup_until = Instant::now() + Duration::from_secs(2);
        let snapshot = enroll(setup_until, || {
            store.try_register_telephone_media_snapshot(phone.clone(), None)
        });
        let read = admit(&snapshot);
        let epoch = read.packet_epoch();
        let credential = read.telephone().route_actor().credential_id.clone();
        let identity = read.telephone().route_actor().identity_id.clone();
        drop(read);
        let held = store
            .acquire_authority(
                &[AuthorityScope::identity(&identity, AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        let c = store.connection_with_authority(&held).unwrap();
        assert_eq!(
            c.execute(
                "UPDATE credentials SET revoked=1 WHERE id=?1",
                [&credential]
            )
            .unwrap(),
            1
        );
        drop(c);
        drop(held);
        assert!(!epoch.current());
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let e = snapshot
                .admit()
                .err()
                .expect("revoked route must not authorize old phone");
            if e.code == "unauthorized" {
                break;
            }
            assert_eq!(e.code, "authority-busy");
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    });
}
