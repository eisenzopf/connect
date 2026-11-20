//! Same real telephone resources and SQL authority; no SIP IO/readiness is fabricated here.
use super::*;
#[test]
fn telephone_commit_preparation_and_same_attempt_readmission_keep_no_idle_writer_and_never_renew() {
    for denied in [false, true] {
        let (f, origin, sources, _, key) = setup_runtime(true);
        let resources = prepared(&f, &origin, &sources, &key);
        let read = f
            .store
            .try_telephone_replacement_scene_read(resources.clone(), &sources)
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let preparation = f.store.prepare_telephone_replacement_commit(&read).unwrap();
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
        drop(read);
        let scope =
            crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
        drop(
            f.store
                .try_acquire_authority(&[scope])
                .expect("no idle planning Reader hidden in preparation"),
        );
        let read = f
            .store
            .try_telephone_replacement_scene_read(resources, &sources)
            .unwrap();
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(100));
        let (attempt, read) = f
            .store
            .try_begin_prepared_telephone_replacement_commit_with_wait(&preparation, read, &wait)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(attempt.snapshot()).unwrap(),
            serde_json::to_value(preparation.snapshot()).unwrap()
        );
        drop(read);
        if denied {
            f.store
                .execute_operation_action(
                    &f.admin,
                    connect_protocol::transitions::OperationAction::Cancel {
                        operation_id: "cancel-prepared-phone-final".into(),
                        target_operation_id: origin.request().operation_id.clone(),
                        reason: None,
                    },
                )
                .unwrap();
        }
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let actual = f
            .store
            .try_telephone_replacement_commit_attempt_scene_write(&attempt, &wait);
        if denied {
            assert!(actual.is_err());
        } else {
            let actual = actual.unwrap();
            assert!(actual.current() && actual.expires_at() <= preparation.expires_at());
            drop(actual);
        }
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
}
