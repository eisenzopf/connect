//! Local ordering/cancellation controls. These do not fabricate a Core/native
//! reservation, a Store-issued observation, or an integrated NNF receipt.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait, Store};
use std::sync::{atomic::AtomicUsize, Mutex};

#[tokio::test]
async fn original_resource_is_owned_before_native_poll_and_actual_readers_are_released_before_observation(
) {
    let directory = tempfile::tempdir().unwrap();
    let store =
        Store::initialize(&directory.path().join("state.sqlite3"), Default::default()).unwrap();
    let roles = tokio::sync::Mutex::new(());
    let scope = AuthorityScope::identity("same-original-control", AuthorityMode::Read).unwrap();
    let reader = store
        .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_millis(50)))
        .unwrap();
    let coordinator = roles.lock().await;
    let original = Arc::new(());
    let root = Mutex::new(None::<Arc<()>>);
    let native_polls = AtomicUsize::new(0);
    let recorded = AtomicUsize::new(0);
    let outcome = own_then_observe(
        (coordinator, reader),
        |_| {
            assert!(roles.try_lock().is_err());
            assert_eq!(native_polls.load(Ordering::Acquire), 0);
            *root.lock().unwrap() = Some(original.clone());
            Ok(())
        },
        async {
            native_polls.fetch_add(1, Ordering::AcqRel);
            assert!(Arc::ptr_eq(
                root.lock().unwrap().as_ref().unwrap(),
                &original
            ));
            let coordinator = roles
                .try_lock()
                .expect("original coordinator must be gone before native observation");
            let writer = store
                .acquire_authority(
                    &[
                        AuthorityScope::identity("same-original-control", AuthorityMode::Write)
                            .unwrap(),
                    ],
                    &AuthorityWait::new(Duration::from_millis(50)),
                )
                .expect("original Reader must be gone before native observation");
            drop(writer);
            drop(coordinator);
            Ok(17u64)
        },
        |binding| {
            assert_eq!(binding, 17);
            assert_eq!(recorded.load(Ordering::Acquire), 0);
            Ok(())
        },
        async {
            let coordinator = roles.try_lock().unwrap();
            let writer = store
                .acquire_authority(
                    &[
                        AuthorityScope::identity("same-original-control", AuthorityMode::Write)
                            .unwrap(),
                    ],
                    &AuthorityWait::new(Duration::from_millis(50)),
                )
                .unwrap();
            recorded.fetch_add(1, Ordering::AcqRel);
            drop(writer);
            drop(coordinator);
            Ok(())
        },
    )
    .await;
    let same_owned = root
        .lock()
        .unwrap()
        .take()
        .is_some_and(|held| Arc::ptr_eq(&held, &original));
    outcome.unwrap();
    assert!(same_owned);
    assert_eq!(native_polls.load(Ordering::Acquire), 1);
    assert_eq!(recorded.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn cancelled_native_observation_preserves_same_root_resource_and_never_binds_or_records() {
    let roles = tokio::sync::Mutex::new(());
    let coordinator = roles.lock().await;
    let original = Arc::new(());
    let root = Mutex::new(None::<Arc<()>>);
    let polls = AtomicUsize::new(0);
    let bound = AtomicUsize::new(0);
    let recorded = AtomicUsize::new(0);
    let mut observation = Box::pin(own_then_observe(
        coordinator,
        |_| {
            *root.lock().unwrap() = Some(original.clone());
            Ok(())
        },
        async {
            assert!(roles.try_lock().is_ok());
            assert!(Arc::ptr_eq(
                root.lock().unwrap().as_ref().unwrap(),
                &original
            ));
            polls.fetch_add(1, Ordering::AcqRel);
            std::future::pending::<Result<()>>().await
        },
        |_| {
            bound.fetch_add(1, Ordering::AcqRel);
            Ok(())
        },
        async {
            recorded.fetch_add(1, Ordering::AcqRel);
            Ok(())
        },
    ));
    std::future::poll_fn(|cx| {
        assert!(observation.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(observation);
    let same_owned = root
        .lock()
        .unwrap()
        .take()
        .is_some_and(|held| Arc::ptr_eq(&held, &original));
    assert!(same_owned);
    assert!(roles.try_lock().is_ok());
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_eq!(bound.load(Ordering::Acquire), 0);
    assert_eq!(recorded.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn original_ownership_or_binding_refusal_never_records_a_phase_or_retains_admission() {
    for refuse_ownership in [true, false] {
        let roles = tokio::sync::Mutex::new(());
        let coordinator = roles.lock().await;
        let polls = AtomicUsize::new(0);
        let bound = AtomicUsize::new(0);
        let recorded = AtomicUsize::new(0);
        let outcome = own_then_observe(
            coordinator,
            |_| {
                if refuse_ownership {
                    Err(Error::forbidden())
                } else {
                    Ok(())
                }
            },
            async {
                polls.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
            |_| {
                bound.fetch_add(1, Ordering::AcqRel);
                Err(Error::forbidden())
            },
            async {
                recorded.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
        )
        .await;
        assert!(roles.try_lock().is_ok());
        assert_eq!(outcome.unwrap_err().code, "forbidden");
        assert_eq!(
            polls.load(Ordering::Acquire),
            usize::from(!refuse_ownership)
        );
        assert_eq!(
            bound.load(Ordering::Acquire),
            usize::from(!refuse_ownership)
        );
        assert_eq!(recorded.load(Ordering::Acquire), 0);
    }
}
