use super::*;
#[tokio::test]
async fn actual_final_owner_pin_retains_same_admitted_entry_when_watcher_finishes_before_outcome_flush(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let runtime = Runtime::new(store).unwrap();
    let record = "same-admitted-record";
    assert!(runtime
        .consultation_jobs
        .lock()
        .unwrap()
        .insert(record.into()));
    let mut final_owner =
        PostCustody::new(&runtime, (record, "one-attempt", "one-effect")).unwrap();
    let original = runtime
        .consultation_jobs
        .lock()
        .unwrap()
        .get(record)
        .unwrap();
    // EXACT operation Job::drop path, forced before the final owner flush.
    runtime.consultation_jobs.lock().unwrap().remove(record);
    assert!(Arc::ptr_eq(
        &original,
        &runtime
            .consultation_jobs
            .lock()
            .unwrap()
            .get(record)
            .unwrap()
    ));
    // Exercise the actual Drop-owned typed begin-uncertainty flush. These IDs
    // are metadata only, not a fake packet/model effect or permission grant.
    final_owner.begin_issued();
    drop(final_owner);
    let current = runtime
        .consultation_jobs
        .lock()
        .unwrap()
        .get(record)
        .unwrap();
    assert!(Arc::ptr_eq(&original, &current));
    assert_eq!(current.final_owners.load(Ordering::Acquire), 0);
    let outcomes = {
        let mut jobs = runtime.consultation_jobs.lock().unwrap();
        let outcomes = jobs.take_unconfirmed_begin();
        jobs.prune(record);
        outcomes
    };
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        (
            &outcomes[0].record,
            &outcomes[0].attempt,
            &outcomes[0].effect
        ),
        (
            &record.to_owned(),
            &"one-attempt".to_owned(),
            &"one-effect".to_owned()
        )
    );
    assert_eq!(runtime.consultation_jobs.lock().unwrap().len(), 0);
    // Payloads move out before report/drop; never a re-admission or Core replay.
    for outcome in outcomes {
        outcome.report_and_release();
    }
    runtime.stop.cancel();
}

#[tokio::test]
async fn queued_store_begin_refuses_after_original_final_owner_closes() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let runtime = Runtime::new(store).unwrap();
    let record = "closed-before-store-begin";
    assert!(runtime
        .consultation_jobs
        .lock()
        .unwrap()
        .insert(record.into()));
    let final_owner = PostCustody::new(&runtime, (record, "attempt", "effect")).unwrap();
    let dispatched = final_owner.begin_dispatch();
    drop(final_owner);
    assert!(dispatched.issue().is_err());
    assert!(runtime
        .consultation_jobs
        .lock()
        .unwrap()
        .take_unconfirmed_begin()
        .is_empty());
}
