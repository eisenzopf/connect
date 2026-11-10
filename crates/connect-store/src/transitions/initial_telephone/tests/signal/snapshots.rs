use super::*;
use crate::{AuthorityMode, AuthorityScope, AuthorityWait, InitialTelephonePacketPhase};
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
#[test]
fn ordinary_initial_sip_snapshot_keeps_connected_owner_without_packet_sql_or_idle_fence() {
    let f = Fixture::new();
    let phone = connected(&f);
    let expected = f
        .store
        .try_initial_telephone_media_read(phone.clone())
        .unwrap();
    let may_receive = expected.may_receive();
    let expected_generation = expected.member().membership_epoch;
    drop(expected);
    let cid = phone.source().conversation_id.clone();
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let setup_until = Instant::now() + Duration::from_secs(2);
        let snapshot = enroll(setup_until, || {
            store.try_register_initial_telephone_media_snapshot(phone.clone(), None)
        });
        let until = Instant::now() + Duration::from_secs(2);
        let read = loop {
            match snapshot.admit() {
                Ok(read) => break read,
                Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("ordinary snapshot: {}", e.code),
            }
        };
        assert_eq!(read.phase(), InitialTelephonePacketPhase::Ordinary);
        assert!(read.phone().is_connected() && !read.phone().is_private_audio());
        assert_eq!(read.phone().member().membership_epoch, expected_generation);
        assert_eq!(read.phone().may_receive(), may_receive);
        let original = read.packet_epoch();
        {
            let _sql = store.connection().unwrap();
            let observation = store.packet_connections.observe().unwrap();
            let _observers = (0..crate::packet_observation::OBSERVERS)
                .map(|_| store.try_packet_connection(&observation).unwrap())
                .collect::<Vec<_>>();
            for _ in 0..32 {
                assert!(snapshot.admit().unwrap().current());
            }
        }
        drop(read);
        let writer = store
            .acquire_authority(
                &[AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap()],
                &AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
        assert!(!original.current() && snapshot.admit().is_err());
        drop(writer);
        drop(snapshot);
    }));
    watcher.shutdown().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
