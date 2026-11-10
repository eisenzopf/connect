//! Actual effect-free Start read admission; no replay of a Start mutation.
use super::*;
use crate::AuthorityWait;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn replacement_initial_read_waits_for_primary_sql_then_reads_same_original_without_effect() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let sql = f.store.connection().unwrap();
    let wait = AuthorityWait::new(Duration::from_secs(2));
    let (sent, received) = mpsc::channel();
    let (entered, entering) = mpsc::channel();
    let read = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            entered.send(()).unwrap();
            let result = f.store.native_replacement_start_read_with_peer_wait(
                &allocated,
                &request.credentials[0],
                &proof,
                &wait,
            );
            sent.send(result.is_ok()).unwrap();
            result
        });
        entering.recv_timeout(Duration::from_secs(1)).unwrap();
        let blocked = received.recv_timeout(Duration::from_millis(25)).is_err();
        drop(sql);
        let result = worker.join().unwrap();
        assert!(blocked, "the actual primary SQL owner excludes this read");
        result.unwrap()
    });
    assert!(read.current());
    assert!(read.expires_at() <= proof.expires_at());
    assert_eq!(read.target().connection.generation, 2);
    drop(read);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn replacement_initial_read_cancellation_and_expiry_join_without_start_effect() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    for cancel in [false, true] {
        let sql = f.store.connection().unwrap();
        let wait = AuthorityWait::new(if cancel {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(30)
        });
        let (sent, received) = mpsc::channel();
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                sent.send(()).unwrap();
                f.store.native_replacement_start_read_with_peer_wait(
                    &allocated,
                    &request.credentials[0],
                    &proof,
                    &wait,
                )
            });
            received.recv_timeout(Duration::from_secs(1)).unwrap();
            if cancel {
                wait.cancel();
            }
            // Join while the original SQL owner STILL excludes admission.
            let result = worker.join().unwrap();
            drop(sql);
            result
        });
        assert_eq!(
            result.unwrap_err().code,
            if cancel {
                "cancelled"
            } else {
                "authority-busy"
            }
        );
    }
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
}

#[test]
fn replacement_initial_read_rechecks_source_withdrawal_after_sql_wait_without_start() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let effects_before = f.count("operation_effects");
    // The real supplied writer authorizes this mutation; no raw SQL exemption.
    let sql = f
        .store
        .connection_with_authority(Store::peer_scopes(&proof))
        .unwrap();
    let wait = AuthorityWait::new(Duration::from_secs(2));
    let (sent, received) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            sent.send(()).unwrap();
            let result = f.store.native_replacement_start_read_with_peer_wait(
                &allocated,
                &request.credentials[0],
                &proof,
                &wait,
            );
            done.send(()).unwrap();
            result
        });
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        let blocked = completed.recv_timeout(Duration::from_millis(25)).is_err();
        let removed = sql
            .execute(
                "DELETE FROM session_presence_capabilities WHERE participant_id=?1",
                [&f.pids[0]],
            )
            .unwrap();
        drop(sql);
        let result = worker.join().unwrap();
        assert!(blocked);
        assert_eq!(removed, 1);
        result
    });
    assert_eq!(result.unwrap_err().code, "forbidden");
    assert_eq!(f.count("native_replacement_steps"), 2);
    assert_eq!(f.count("operation_effects"), effects_before);
}

#[test]
fn replacement_initial_read_sql_and_policy_errors_are_terminal_without_effect() {
    let (f, _, _, allocated, request) = fixture();
    let proof = f.admit(&request);
    let wait = AuthorityWait::new(Duration::from_secs(2));
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let refused = f
        .store
        .native_replacement_start_read_with_peer_wait(&allocated, &f.keys[0], &proof, &wait);
    assert_eq!(refused.unwrap_err().code, "forbidden");
    // Interrupt the first actual SQLite query. A statement error must not be
    // mistaken for mutex admission and retried after its one-shot interruption.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    f.store.connection().unwrap().progress_handler(
        1,
        Some(move || observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0),
    );
    let failed = f.store.native_replacement_start_read_with_peer_wait(
        &allocated,
        &request.credentials[0],
        &proof,
        &wait,
    );
    f.store
        .connection()
        .unwrap()
        .progress_handler(0, None::<fn() -> bool>);
    assert_eq!(failed.unwrap_err().code, "storage-unavailable");
    let read = f
        .store
        .native_replacement_start_read_with_peer_wait(
            &allocated,
            &request.credentials[0],
            &proof,
            &wait,
        )
        .unwrap();
    drop(read);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    assert_eq!(f.count("native_replacement_steps"), 2);
}
