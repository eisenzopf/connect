//! Exact owned started-attempt projection, with ordinary pending refusal unchanged.
use super::*;
#[test]
fn handoff_same_started_attempt_purpose_writer_preserves_modes_currentness_and_original_cutoff() {
    // An actually cancelled original waiter never reserves Started or an
    // effect submission, and owns no SQL/file fence after refusal.
    {
        let (f, sources, origin) = fixture(OutgoingDisposition::Observe);
        let selected = prepare(&f, &sources, &origin, OutgoingDisposition::Observe);
        let read = f.store.native_handoff_attempt_read(&selected).unwrap();
        let cutoff = read.expires_at();
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(100));
        wait.cancel();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        assert!(f
            .store
            .try_begin_native_handoff_with_wait(&selected, read, cutoff, &wait)
            .is_err());
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    for disposition in [OutgoingDisposition::Observe, OutgoingDisposition::Leave] {
        for denial in ["none", "cancel", "observed", "provider"] {
            let (f, sources, origin) = fixture(disposition.clone());
            let selected = prepare(&f, &sources, &origin, disposition);
            let planning = f.store.native_handoff_attempt_read(&selected).unwrap();
            let original_until = planning.expires_at();
            let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(100));
            let (attempt, cut) = f
                .store
                .try_begin_native_handoff_with_wait(&selected, planning, original_until, &wait)
                .unwrap()
                .unwrap();
            drop(cut);
            assert!(attempt.expires_at() <= original_until);
            assert!(
                f.store.try_native_media_read(&sources[0], None).is_err(),
                "ordinary packet cannot borrow pending handoff authority"
            );
            match denial {
                "cancel" => cancel(&f, &origin),
                "observed" | "provider" => {
                    let c = f.change_fixture();
                    let sql = if denial == "observed" {
                        "UPDATE operation_effects SET observation_json='{}' WHERE id=?1"
                    } else {
                        "UPDATE operation_effects SET provider_ref='observed-provider-reference' WHERE id=?1"
                    };
                    assert_eq!(
                        c.execute(sql, [attempt.attempt_binding().2]).unwrap(),
                        1,
                        "legal first attribution needs no immutable-trigger bypass"
                    );
                }
                _ => {}
            }
            let (foreign, foreign_sources, foreign_origin) = fixture(OutgoingDisposition::Observe);
            let foreign_lease = prepare(
                &foreign,
                &foreign_sources,
                &foreign_origin,
                OutgoingDisposition::Observe,
            );
            let foreign_attempt = begin(&foreign, foreign_lease);
            assert_ne!(
                attempt.attempt_binding().1,
                foreign_attempt.attempt_binding().1
            );
            let before = Store::inspect_snapshot(f.store.path()).unwrap();
            assert!(
                f.store
                    .try_native_handoff_started_scene_write(
                        &foreign_attempt.admission(),
                        &crate::AuthorityWait::new(std::time::Duration::from_millis(100))
                    )
                    .is_err(),
                "foreign opaque handle cannot adopt this Store's authority"
            );
            let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(100));
            let read = f
                .store
                .try_native_handoff_started_scene_write(&attempt.admission(), &wait);
            if denial != "none" {
                assert!(read.is_err(), "{denial}");
            } else {
                let read = read.unwrap();
                assert!(read.current() && read.expires_at() <= original_until);
                drop(read);
            }
            assert_eq!(
                Store::inspect_snapshot(f.store.path()).unwrap(),
                before,
                "no new attempt/effect/lease mutation"
            );
        }
    }
}
