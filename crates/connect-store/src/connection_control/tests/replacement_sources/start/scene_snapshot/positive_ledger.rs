//! Real media epochs plus exact SQL Unit/collision/lifetime refusal controls.
use super::*;
use crate::authority_writer_boundary::phase_ledger::{Key, Stage};
use crate::{NativeReplacementMediaCapture, NativeReplacementMediaPhase as Phase};
use rusqlite::{params, TransactionBehavior};
fn root_key(store: &Store, record: &str, phase: &'static str, stage: Stage) -> Key {
    let c = store.connection().unwrap();
    let (runtime,reservation,digest,candidate,generation):(String,String,String,String,u64)=c.query_row("SELECT runtime_epoch,reservation_id,plan_digest,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    let expected=crate::provisioning::canonical_json(&serde_json::json!({"candidate":candidate,"generation":generation,"phase":phase,"attached":true,"private":true,"activated_muted":phase!="attach","media_ready":phase=="readiness"})).unwrap();
    Key {
        record: record.into(),
        phase,
        attempt: "ledger-control-attempt".into(),
        effect: "ledger-control-effect".into(),
        expected,
        runtime,
        reservation,
        digest,
        stage,
    }
}
fn copy_key(key: &Key) -> Key {
    Key {
        record: key.record.clone(),
        phase: key.phase,
        attempt: key.attempt.clone(),
        effect: key.effect.clone(),
        expected: key.expected.clone(),
        runtime: key.runtime.clone(),
        reservation: key.reservation.clone(),
        digest: key.digest.clone(),
        stage: key.stage,
    }
}
fn record_id(store: &Store, command: &TransitionCommand) -> String {
    store.connection().unwrap().query_row("SELECT record_id FROM operations WHERE operation_id=?1 AND command_type='connection.replace'",[&command.operation_id],|r|r.get(0)).unwrap()
}
fn counts(store: &Store, record: &str) -> (u64, u64) {
    store.connection().unwrap().query_row("SELECT count(*),COALESCE(sum(e.attempt),0) FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
}
fn complete(
    store: &Store,
    origin: &Arc<crate::NativeReplacementMediaOrigin>,
    sources: &[Arc<crate::NativeMediaOrigin>],
) -> crate::NativeReplacementSceneRead {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match store.try_native_replacement_scene_read(origin, sources) {
            Ok(read) => return read,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::yield_now()
            }
            Err(error) => panic!("complete original scene setup refused: {error}"),
        }
    }
}
fn record(
    store: &Store,
    capture: &NativeReplacementMediaCapture,
) -> connect_protocol::transitions::OperationRecord {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match store.record_native_replacement_media_phase(capture) {
            Ok(record) => return record,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::yield_now()
            }
            Err(error) => panic!("same original captured metadata refused: {error}"),
        }
    }
}
fn applied(
    store: &Store,
    origin: &Arc<crate::NativeReplacementMediaOrigin>,
    sources: &[Arc<crate::NativeMediaOrigin>],
    phase: Phase,
) -> NativeReplacementMediaCapture {
    let read = complete(store, origin, sources);
    let attempt = store
        .begin_native_replacement_scene_media_phase(origin.clone(), phase, &read)
        .unwrap();
    let expected = attempt.expected().unwrap();
    drop(read);
    attempt.applied(expected).unwrap()
}
fn effect_birth(c: &rusqlite::Connection, key: &Key) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,?3,?1,'none','prepared',?4)",params![key.effect,key.record,format!("native-replacement-{}",key.phase),key.expected])
}
fn writer(store: &Store, record: &str) -> Arc<crate::AuthorityGuardSet> {
    let (owner, cid): (String, String) = store
        .connection()
        .unwrap()
        .query_row(
            "SELECT owner_id,conversation_id FROM operations WHERE record_id=?1",
            [record],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    store
        .acquire_authority(
            &[
                AuthorityScope::identity(&owner, AuthorityMode::Write).unwrap(),
                AuthorityScope::conversation(&cid, AuthorityMode::Write).unwrap(),
            ],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap()
}
#[test]
fn scene_positive_attach_preserves_actual_io_and_original_queued_epoch_once() {
    // Receiver-epoch eligibility first checks the original target grant.
    // Grant subscription authority during fixture creation so this case
    // reaches the separate confirmed activation check.
    let (f, _, command, _, origin, sources) = resource_fixture_scene_scoped(&["stream.subscribe"]);
    let store = Arc::new(f.store);
    let id = record_id(&store, &command);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let io = source_admitted(&source);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let cutoff = io.expires_at();
    let read = complete(&store, &origin, &sources);
    let attempt = store
        .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Attach, &read)
        .unwrap();
    let expected = attempt.expected().unwrap();
    drop(read);
    assert!(io.current() && epoch.current());
    assert!(source.admit_original(stamp).unwrap().current());
    let captured = attempt.applied(expected).unwrap();
    let first = record(&store, &captured);
    assert_eq!(counts(&store, &id), (1, 1));
    assert!(io.current() && epoch.current());
    assert_eq!(io.expires_at(), cutoff);
    assert_eq!(record(&store, &captured), first);
    assert!(source.admit_original(stamp).unwrap().current());
    let packet = store.try_native_replacement_media_read(&origin).unwrap();
    assert_eq!(
        store
            .begin_native_replacement_media_phase(origin.clone(), Phase::Attach, &packet)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(packet);
    let read = complete(&store, &origin, &sources);
    assert_eq!(
        store
            .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Attach, &read)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    drop(read);
    assert_eq!(counts(&store, &id), (1, 1));
    let error = store
        .try_native_replacement_receiver_epoch_read(&origin)
        .unwrap_err();
    assert_eq!(error.code, "not-ready");
    assert_eq!(
        error.message,
        "Original private resources are not confirmed active"
    );
    drop(io);
    drop(source);
    watcher.shutdown().unwrap();
}
#[test]
fn scene_positive_readiness_preserves_io_but_activate_and_current_writer_still_exclude() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    let id = record_id(&store, &command);
    let attach = applied(&store, &origin, &sources, Phase::Attach);
    record(&store, &attach);
    let read = complete(&store, &origin, &sources);
    assert_eq!(
        store
            .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Activate, &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(read);
    let write = store
        .try_native_replacement_scene_write(&origin, &sources)
        .unwrap();
    let activate = store
        .begin_native_replacement_media_phase(origin.clone(), Phase::Activate, write.media())
        .unwrap();
    let expected = activate.expected().unwrap();
    drop(write);
    record(&store, &activate.applied(expected).unwrap());
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let io = source_admitted(&source);
    let epoch = io.packet_epoch().unwrap();
    let stamp = io.snapshot_stamp().unwrap();
    let ready = applied(&store, &origin, &sources, Phase::Readiness);
    record(&store, &ready);
    assert_eq!(counts(&store, &id), (3, 3));
    assert!(io.current() && epoch.current());
    assert!(source.admit_original(stamp).unwrap().current());
    let scope =
        AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap();
    assert_eq!(
        store
            .try_acquire_authority(&[scope.clone()])
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(!epoch.current());
    drop(io);
    drop(source);
    drop(
        store
            .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_secs(1)))
            .unwrap(),
    );
    watcher.shutdown().unwrap();
}
fn blocked_capture(
    store: &Store,
    captured: &NativeReplacementMediaCapture,
    revoked: impl Fn() -> bool,
) {
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        let error = store
            .record_native_replacement_media_phase(captured)
            .unwrap_err();
        assert_eq!(error.code, "authority-busy");
        if revoked() {
            break;
        }
        assert!(
            Instant::now() < until,
            "real writer did not invalidate the original epoch"
        );
        std::thread::yield_now();
    }
}
#[test]
fn uncertain_capture_retains_writer_and_drains_actual_io_before_unknown() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    let id = record_id(&store, &command);
    let read = complete(&store, &origin, &sources);
    let attempt = store
        .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Attach, &read)
        .unwrap();
    drop(read);
    let capture = attempt.uncertain();
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let io = source_admitted(&source);
    let epoch = io.packet_epoch().unwrap();
    blocked_capture(&store, &capture, || !epoch.current());
    assert!(!io.current());
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [&id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "preparing"
    );
    drop(io);
    drop(source);
    let result = record(&store, &capture);
    assert_eq!(result.state, OperationState::Unknown);
    assert_eq!(counts(&store, &id), (1, 1));
    assert!(!epoch.current());
    assert!(store.try_native_replacement_media_read(&origin).is_err());
    watcher.shutdown().unwrap();
}
#[test]
fn late_positive_unknown_or_cancelled_capture_cannot_use_read_or_revive_io() {
    for cancelled in [false, true] {
        let (f, key, command, _, origin, sources) = resource_fixture_scene();
        let id = record_id(&f.store, &command);
        let captured = applied(&f.store, &origin, &sources, Phase::Attach);
        if cancelled {
            cancel(&f, &key, &command);
        } else {
            // A real bound writer models recovery marking the exact original
            // effect Unknown while its immutable positive result is retained.
            let held = writer(&f.store, &id);
            let mut c = f.store.connection_with_authority(&held).unwrap();
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            tx.execute("UPDATE operation_effects SET state='unknown' WHERE id IN (SELECT effect_id FROM native_replacement_media_steps WHERE operation_record_id=?1)",[&id]).unwrap();
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1",
                [&id],
            )
            .unwrap();
            tx.commit().unwrap();
            drop(c);
            drop(held);
        }
        let before: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let store = Arc::new(f.store);
        let mut watcher = store.start_media_authorization_watcher().unwrap();
        let source = source_snapshot(&store, &sources[0]);
        let io = source_admitted(&source);
        let epoch = io.packet_epoch().unwrap();
        blocked_capture(&store, &captured, || !epoch.current());
        drop(io);
        drop(source);
        let historical = record(&store, &captured);
        assert_eq!(historical.state.as_str(), before);
        assert_eq!(counts(&store, &id), (1, 1));
        assert!(!epoch.current());
        assert!(store.try_native_replacement_media_read(&origin).is_err());
        watcher.shutdown().unwrap();
    }
}
#[test]
fn raw_read_wrong_phase_and_foreign_manager_cannot_borrow_ledger_permit() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    let key = root_key(&f.store, &id, "attach", Stage::Begin);
    let read = complete(&f.store, &origin, &sources);
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(effect_birth(&tx, &key).is_err());
    assert!(tx
        .execute(
            "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
            [&id]
        )
        .is_err());
    assert!(f
        .store
        .authority_writer_boundary
        .native_phase_ledger(
            &f.store,
            &tx,
            read.snapshot_interval(),
            Key {
                phase: "activate",
                ..copy_key(&key)
            }
        )
        .is_err());
    assert!(f
        .store
        .authority_writer_boundary
        .native_phase_ledger(
            &f.store,
            &tx,
            read.snapshot_interval(),
            Key {
                runtime: "foreign-runtime".into(),
                ..copy_key(&key)
            }
        )
        .is_err());
    let mut other = rusqlite::Connection::open_in_memory().unwrap();
    let other_tx = other
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(
        f.store
            .authority_writer_boundary
            .native_phase_ledger(
                &f.store,
                &other_tx,
                read.snapshot_interval(),
                copy_key(&key)
            )
            .is_err(),
        "same proof cannot lend the permit to a foreign connection"
    );
    other_tx.rollback().unwrap();
    tx.rollback().unwrap();
    drop(c);
    drop(read);
    let foreign_dir = tempfile::tempdir().unwrap();
    let foreign = Store::initialize(
        &foreign_dir.path().join("foreign.sqlite3"),
        Default::default(),
    )
    .unwrap();
    let scopes = complete(&f.store, &origin, &sources)
        .snapshot_interval()
        .snapshot_read_scopes()
        .unwrap();
    let foreign_proof = foreign
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .native_phase_ledger(&f.store, &tx, &foreign_proof, copy_key(&key))
        .is_err());
    tx.rollback().unwrap();
    drop(c);
    drop(foreign_proof);
    let raw = rusqlite::Connection::open(f.store.path()).unwrap();
    let error = effect_birth(&raw, &key).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(crate::authority_writer_boundary::FUNCTION)
            || error
                .to_string()
                .contains(crate::authority_writer_boundary::phase_ledger::FUNCTION)
    );
    assert_eq!(counts(&f.store, &id), (0, 0));
}
#[test]
fn ledger_nonce_is_transaction_bound_and_cannot_replace_unique_victim() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    let key = root_key(&f.store, &id, "attach", Stage::Begin);
    let held = writer(&f.store, &id);
    let c = f.store.connection_with_authority(&held).unwrap();
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES('ledger-collision-victim',?1,'native-replacement-attach',?2,'none','prepared',?3)",params![id,key.effect,key.expected]).unwrap();
    drop(c);
    drop(held);
    let read = complete(&f.store, &origin, &sources);
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let permit = f
        .store
        .authority_writer_boundary
        .native_phase_ledger(&f.store, &tx, read.snapshot_interval(), copy_key(&key))
        .unwrap();
    assert!(tx.execute("INSERT OR REPLACE INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-attach',?1,'none','prepared',?3)",params![key.effect,id,key.expected]).is_err());
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM operation_effects WHERE id='ledger-collision-victim'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    for sql in [
        "UPDATE operations SET state='unknown' WHERE record_id=?1",
        "UPDATE operations SET lease_owner='foreign' WHERE record_id=?1",
        "UPDATE operations SET expires_at=expires_at+1 WHERE record_id=?1",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
    ] {
        assert!(
            tx.execute(sql, [&id]).is_err(),
            "permit rewrote non-ledger field"
        );
    }
    assert!(tx.execute_batch("COMMIT").is_err());
    assert!(tx.is_autocommit());
    tx.execute_batch("BEGIN IMMEDIATE").unwrap();
    let second = f
        .store
        .authority_writer_boundary
        .native_phase_ledger(&f.store, &tx, read.snapshot_interval(), copy_key(&key))
        .unwrap();
    drop(permit);
    assert_eq!(tx.query_row("SELECT connect_store_native_phase_ledger_v1('effect-birth',?1,?2,'attach',?3,NULL,NULL)",params![id,key.effect,key.expected],|r|r.get::<_,i64>(0)).unwrap(),1,"stale Drop cleared later nonce");
    tx.execute_batch("ROLLBACK").unwrap();
    assert_eq!(tx.query_row("SELECT connect_store_native_phase_ledger_v1('effect-birth',?1,?2,'attach',?3,NULL,NULL)",params![id,key.effect,key.expected],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(second);
    drop(tx);
    drop(c);
    drop(read);
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT connect_store_native_phase_ledger_v1('effect-birth',?1,?2,'attach',?3,NULL,NULL)",params![id,key.effect,key.expected],|r|r.get::<_,i64>(0)).unwrap(),0,"permit leaked into next SQL Unit");
    drop(c);
    assert_eq!(counts(&f.store, &id), (0, 0));
}
#[test]
fn expired_complete_scene_refuses_ledger_begin_without_renewing_original_cutoff() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    let read = complete(&f.store, &origin, &sources);
    let cutoff = read.expires_at();
    let until = Instant::now() + Duration::from_secs(6);
    while crate::now() < cutoff && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(crate::now() >= cutoff);
    assert!(!read.current());
    assert_eq!(store_error(&f.store, &origin, &read), "forbidden");
    assert_eq!(read.expires_at(), cutoff);
    assert_eq!(counts(&f.store, &id), (0, 0));
}
fn store_error(
    store: &Store,
    origin: &Arc<crate::NativeReplacementMediaOrigin>,
    read: &crate::NativeReplacementSceneRead,
) -> String {
    store
        .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Attach, read)
        .unwrap_err()
        .code
}

#[test]
fn phase_step_collisions_and_wrong_attempt_do_not_consume_original_binding() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    let key = root_key(&f.store, &id, "attach", Stage::Begin);
    let read = complete(&f.store, &origin, &sources);
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let permit = f
        .store
        .authority_writer_boundary
        .native_phase_ledger(&f.store, &tx, read.snapshot_interval(), copy_key(&key))
        .unwrap();
    effect_birth(&tx, &key).unwrap();
    tx.execute("INSERT INTO native_replacement_media_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'attach',?2,?3)",params![id,key.attempt,key.effect]).unwrap();
    let collision=tx.execute("INSERT OR REPLACE INTO native_replacement_media_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'attach','wrong-attempt',?2)",params![id,key.effect]).unwrap_err();
    assert!(collision
        .to_string()
        .contains("native replacement media phase binding conflicts"));
    let actual:String=tx.query_row("SELECT attempt_id FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='attach'",[&id],|r|r.get(0)).unwrap();
    assert_eq!(actual, key.attempt);
    tx.execute("UPDATE native_replacement_media_steps SET started=1 WHERE operation_record_id=?1 AND phase='attach'",[&id]).unwrap();
    tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&key.effect]).unwrap();
    drop(permit);
    assert!(
        f.store
            .authority_writer_boundary
            .native_phase_ledger(
                &f.store,
                &tx,
                read.snapshot_interval(),
                Key {
                    stage: Stage::Confirm,
                    attempt: "wrong-attempt".into(),
                    ..copy_key(&key)
                }
            )
            .is_err(),
        "a mismatched attempt cannot confirm the exact reserved effect"
    );
    tx.rollback().unwrap();
    drop(c);
    drop(read);
    assert_eq!(counts(&f.store, &id), (0, 0));
}

mod candidate_activation;
