//! Wire admission must survive restart and roll back on durable-write failure.
use super::media_fixture::*;
use crate::{Limits, Store};

#[test]
fn committed_start_admissions_survive_another_store_handle_and_restart() {
    let (f, _) = call_setup();
    f.store
        .execute(&f.key.id, &call_start(&f, "start"))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    assert!(!reopened.voice_dispatch_allowed(&job).unwrap());
    reopened.recover_voice_calls().unwrap();
    assert!(!reopened.voice_dispatch_allowed(&job).unwrap());
    assert!(reopened.claim_voice_job().unwrap().is_none());

    let (f, _, s) = sip_setup();
    f.store
        .execute(&f.key.id, &sip_start(&f, "start", &s))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    assert!(!reopened.sip_dispatch_allowed(&job).unwrap());
    reopened.recover_sip_legs().unwrap();
    assert!(!reopened.sip_dispatch_allowed(&job).unwrap());
    assert!(reopened.claim_sip_leg_job().unwrap().is_none());
}

#[test]
fn failed_start_admission_write_does_not_consume_the_dispatch_fence() {
    let (f, _) = call_setup();
    f.store
        .execute(&f.key.id, &call_start(&f, "start"))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    f.store.connection().unwrap().execute_batch(
        "CREATE TRIGGER fail_admission BEFORE INSERT ON media_start_admissions BEGIN SELECT RAISE(ABORT,'fixture'); END;",
    ).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.voice_dispatch_allowed(&job).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_admission")
        .unwrap();
    assert!(f.store.voice_dispatch_allowed(&job).unwrap());
    assert!(!f.store.voice_dispatch_allowed(&job).unwrap());
}

#[test]
fn failed_end_admission_write_leaves_the_exact_generation_claimed() {
    let (f, _, s) = sip_setup();
    let leg = active_sip(&f, &s);
    f.store
        .execute(&f.key.id, &sip_end(&f, "end", &leg))
        .unwrap();
    let job = f.store.claim_sip_leg_job().unwrap().unwrap();
    f.store.connection().unwrap().execute_batch(
        "CREATE TRIGGER fail_end BEFORE UPDATE OF state ON media_end_intents WHEN NEW.state='dispatching' BEGIN SELECT RAISE(ABORT,'fixture'); END;",
    ).unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_eq!(
        f.store.sip_dispatch_allowed(&job).unwrap_err().code,
        "storage-unavailable"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_end")
        .unwrap();
    assert!(f.store.sip_dispatch_allowed(&job).unwrap());
    let reopened = Store::open(f.store.path(), Limits::default()).unwrap();
    assert!(!reopened.sip_dispatch_allowed(&job).unwrap());
    reopened.recover_sip_legs().unwrap();
    assert!(reopened.claim_sip_leg_job().unwrap().is_none());
}

#[test]
fn competing_store_handles_admit_only_one_physical_voice_start() {
    let (f, _) = call_setup();
    f.store
        .execute(&f.key.id, &call_start(&f, "start"))
        .unwrap();
    let job = f.store.claim_voice_job().unwrap().unwrap();
    let other = Store::open(f.store.path(), Limits::default()).unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            f.store.voice_dispatch_allowed(&job).unwrap()
        });
        let second = scope.spawn(|| {
            barrier.wait();
            other.voice_dispatch_allowed(&job).unwrap()
        });
        assert_ne!(first.join().unwrap(), second.join().unwrap());
    });
}
