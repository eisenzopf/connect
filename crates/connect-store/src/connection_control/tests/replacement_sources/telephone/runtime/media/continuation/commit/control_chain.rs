//! Original resource/phase/commit controls under actual reader and SQL owners.
use super::*;
use std::{sync::mpsc, time::Duration};

#[test]
fn resources_and_phase_control_admissions_preserve_original_custody_on_reader_contention() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let (submitted, media) = known(&f, &origin, &sources, &key);
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let mut until = crate::now() + 3;
    let (_, read) = f
        .store
        .telephone_replacement_media_origin_with_wait(submitted, &mut until, &wait)
        .unwrap();
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Read).unwrap();
    let held = f.store.try_acquire_authority(&[scope.clone()]).unwrap();
    let before = f.count("operation_effects");
    assert!(f
        .store
        .try_begin_telephone_replacement_resources_with_wait(read, until, &wait)
        .unwrap()
        .is_none());
    assert!(f.store.try_connection().is_ok());
    assert_eq!(f.count("operation_effects"), before);
    drop(held);
    let read = f
        .store
        .telephone_replacement_media_read_with_wait(media, &mut until, &wait)
        .unwrap();
    let attempt = f
        .store
        .try_begin_telephone_replacement_resources_with_wait(read, until, &wait)
        .unwrap()
        .unwrap();
    let read = f
        .store
        .telephone_replacement_resources_attempt_read_with_wait(attempt.clone(), &wait)
        .unwrap();
    assert!(read.current() && read.expires_at() <= until);
    let capture = Arc::new(attempt.prepared(resources(&read)).unwrap());
    drop(read);
    f.store
        .record_telephone_replacement_resources(&capture)
        .unwrap();
    let read = f
        .store
        .telephone_replacement_scene_read_with_wait(capture.clone(), &sources, &mut until, &wait)
        .unwrap();
    let held = f.store.try_acquire_authority(&[scope]).unwrap();
    let before = f.count("operation_effects");
    assert!(f
        .store
        .try_begin_telephone_replacement_phase_with_wait(read, Step::Attach, until, &wait)
        .unwrap()
        .is_none());
    assert!(f.store.try_connection().is_ok());
    assert_eq!(f.count("operation_effects"), before);
    drop(held);
    let read = f
        .store
        .telephone_replacement_scene_read_with_wait(capture.clone(), &sources, &mut until, &wait)
        .unwrap();
    let attempt = f
        .store
        .try_begin_telephone_replacement_phase_with_wait(read, Step::Attach, until, &wait)
        .unwrap()
        .unwrap();
    let read = f
        .store
        .telephone_replacement_phase_attempt_read_with_wait(attempt.clone(), &wait)
        .unwrap();
    assert!(read.current() && read.expires_at() <= until);
    let receipt = attempt.applied(observation(&read, "attach")).unwrap();
    drop(read);
    f.store
        .record_telephone_replacement_phase(&receipt)
        .unwrap();
    let read = f
        .store
        .telephone_replacement_scene_read_with_wait(capture, &sources, &mut until, &wait)
        .unwrap();
    assert_eq!(
        f.store
            .try_begin_telephone_replacement_phase_with_wait(read, Step::Attach, until, &wait)
            .err()
            .unwrap()
            .code,
        "reconciliation-required"
    );
    assert_eq!(f.count("operation_effects"), before + 1);
}

#[test]
fn commit_control_wait_releases_old_read_before_sql_and_returns_one_fenced_cut() {
    let (f, origin, sources, _, key) = setup_runtime(true);
    let capture = prepared(&f, &origin, &sources, &key);
    let wait = crate::AuthorityWait::new(Duration::from_secs(2));
    let mut until = crate::now() + 3;
    let read = f
        .store
        .telephone_replacement_scene_read_with_wait(capture.clone(), &sources, &mut until, &wait)
        .unwrap();
    let before = f.count("operation_effects");
    let sql = f.store.connection().unwrap();
    let (started, entered) = mpsc::channel();
    let (done, completion) = mpsc::channel();
    let scope = crate::AuthorityScope::conversation(&f.cid, crate::AuthorityMode::Write).unwrap();
    let (observed, released_old, result) = std::thread::scope(|threads| {
        let store = &f.store;
        let wait = wait.clone();
        let worker = threads.spawn(move || {
            let _ = started.send(());
            let value = store.try_begin_telephone_replacement_commit_with_wait(read, &wait);
            let _ = done.send(());
            value
        });
        let observed = entered.recv_timeout(Duration::from_secs(1)).is_ok()
            && completion.recv_timeout(Duration::from_millis(30)).is_err();
        let other = f.store.try_acquire_authority(&[scope.clone()]);
        let released_old = other.is_ok();
        // Original pending begin sees a real other Writer only AFTER SQL drains.
        // It must return None immediately and release its SQL permit.
        drop(sql);
        let result = worker.join();
        drop(other);
        (observed, released_old, result)
    });
    assert!(
        observed && released_old,
        "waiting for SQL must hold no old Read or new Writer"
    );
    assert!(result.unwrap().unwrap().is_none());
    assert!(f.store.try_connection().is_ok());
    assert_eq!(f.count("operation_effects"), before);
    let read = f
        .store
        .telephone_replacement_scene_read_with_wait(capture, &sources, &mut until, &wait)
        .unwrap();
    let (attempt, cut) = f
        .store
        .try_begin_telephone_replacement_commit_with_wait(read, &wait)
        .unwrap()
        .unwrap();
    assert!(cut.current() && cut.expires_at() <= until);
    assert!(
        f.store.try_acquire_authority(&[scope]).is_err(),
        "original returned cut must retain its actual Writer"
    );
    drop(cut);
    f.store
        .record_telephone_replacement_commit_uncertain(&attempt.uncertain())
        .unwrap();
    assert_eq!(f.count("operation_effects"), before + 1);
}
