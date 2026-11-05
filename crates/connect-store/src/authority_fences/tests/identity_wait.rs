//! Both inode metadata acquisitions share the original admission wait owner.
use super::*;

#[derive(Clone, Copy)]
enum Outcome {
    Immediate,
    Released,
    Cancelled,
    Deadline,
    AlreadyExpired,
}

fn exercise(stage: u8, outcome: Outcome) {
    let f = Fixture::new();
    let a = AuthorityScope::identity("metadata_wait_a", AuthorityMode::Write).unwrap();
    let b = AuthorityScope::identity("metadata_wait_b", AuthorityMode::Write).unwrap();
    let a_name = f.store.authority.name(&(a.kind, a.id.clone())).unwrap();
    let b_name = f.store.authority.name(&(b.kind, b.id.clone())).unwrap();
    let target = format!("{b_name}.resource");
    let (entered, entry) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let (waiting, blocked) = mpsc::channel();
    let resumed = std::sync::Mutex::new(resumed);
    let entered_once = AtomicBool::new(false);
    let blocked_once = AtomicBool::new(false);
    *f.store.authority.file_identity_probe.lock().unwrap() = Some(Arc::new(move |name, at| {
        if name != target {
            return;
        }
        if at == stage && !entered_once.swap(true, Ordering::AcqRel) {
            entered.send(()).unwrap();
            resumed
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
        if at == stage + 2 && !blocked_once.swap(true, Ordering::AcqRel) {
            waiting.send(()).unwrap();
        }
    }));
    let wait = AuthorityWait::new(
        if matches!(outcome, Outcome::Deadline | Outcome::AlreadyExpired) {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(2)
        },
    );
    let original_deadline = wait.deadline;
    let cancellation = wait.clone();
    let store = f.store.clone();
    let scopes = vec![a.clone(), b.clone()];
    let (completed, completion) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = if matches!(outcome, Outcome::Immediate) {
            store.try_acquire_authority(&scopes)
        } else {
            store.acquire_authority(&scopes, &wait)
        };
        completed.send((result, Instant::now())).unwrap();
    });
    entry.recv_timeout(Duration::from_secs(1)).unwrap();
    let metadata = f.store.authority.file_identities.lock().unwrap();
    // By this stage the first write scope and the second admission turnstile
    // are actually held. Raw independent opens avoid the intentionally blocked
    // metadata helper while observing their real kernel exclusion.
    let a_resource = OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            f.store
                .authority
                .directory
                .join(format!("{a_name}.resource")),
        )
        .unwrap();
    let b_admission = OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            f.store
                .authority
                .directory
                .join(format!("{b_name}.admission")),
        )
        .unwrap();
    assert!(FileExt::try_lock_exclusive(&a_resource).is_err());
    assert!(FileExt::try_lock_exclusive(&b_admission).is_err());
    if matches!(outcome, Outcome::AlreadyExpired) {
        std::thread::sleep(original_deadline.saturating_duration_since(Instant::now()));
    }
    resume.send(()).unwrap();
    if !matches!(outcome, Outcome::AlreadyExpired) {
        blocked.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    let result = match outcome {
        Outcome::Released => {
            assert!(
                completion.try_recv().is_err(),
                "waited metadata lookup aborted instead of waiting"
            );
            assert!(FileExt::try_lock_exclusive(&a_resource).is_err());
            assert!(FileExt::try_lock_exclusive(&b_admission).is_err());
            drop(metadata);
            let (result, _) = completion.recv_timeout(Duration::from_secs(1)).unwrap();
            let guard = result.expect("original waited admission did not recover");
            assert!(guard.covers(&[a, b]));
            drop(guard);
            None
        }
        _ => {
            if matches!(outcome, Outcome::Cancelled) {
                cancellation.cancel();
            }
            let (result, finished) = completion.recv_timeout(Duration::from_millis(500)).unwrap();
            let expected = if matches!(outcome, Outcome::Cancelled) {
                "cancelled"
            } else {
                "authority-busy"
            };
            assert_eq!(result.unwrap_err().code, expected);
            if matches!(outcome, Outcome::Deadline | Outcome::AlreadyExpired) {
                assert!(finished >= original_deadline);
            }
            // Failure must drop partial scopes even while the unrelated map
            // remains unavailable; cleanup must not reacquire that map.
            FileExt::try_lock_exclusive(&a_resource).unwrap();
            FileExt::try_lock_exclusive(&b_admission).unwrap();
            FileExt::unlock(&a_resource).unwrap();
            FileExt::unlock(&b_admission).unwrap();
            drop(metadata);
            Some(())
        }
    };
    let _ = result;
    worker.join().unwrap();
    *f.store.authority.file_identity_probe.lock().unwrap() = None;
    FileExt::try_lock_exclusive(&a_resource).unwrap();
    FileExt::try_lock_exclusive(&b_admission).unwrap();
}
#[test]
fn waited_lookup_and_publication_resume_under_the_original_admission() {
    for stage in [0, 1] {
        exercise(stage, Outcome::Released);
    }
}
#[test]
fn waited_lookup_and_publication_cancel_without_retaining_partial_turnstiles() {
    for stage in [0, 1] {
        exercise(stage, Outcome::Cancelled);
    }
}
#[test]
fn waited_lookup_and_publication_keep_the_original_deadline() {
    for stage in [0, 1] {
        exercise(stage, Outcome::Deadline);
    }
}
#[test]
fn elapsed_original_deadline_cannot_be_renewed_at_either_metadata_stage() {
    for stage in [0, 1] {
        exercise(stage, Outcome::AlreadyExpired);
    }
}
#[test]
fn immediate_lookup_and_publication_refuse_without_waiting_or_partial_locks() {
    for stage in [0, 1] {
        exercise(stage, Outcome::Immediate);
    }
}
