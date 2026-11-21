//! Actual signed Service replacement under actual Orchestrator participant-model exclusion.
use super::*;
#[path = "final_control/observer.rs"]
mod observer;
pub(super) struct Outcome {
    pub negative: bool,
    evidence: observer::Evidence,
}
impl Outcome {
    pub(super) fn validate_completed(&self, done: &OperationRecord) {
        self.evidence.validate_completed(done);
    }
    pub(super) fn validate_public(&self, done: &OperationRecord, packets: &[NativeProbeSequence]) {
        self.evidence.validate_public(done, packets);
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Case {
    Resume,
    Revoke,
    Expire,
    PostPanic,
    BeginFault,
}
struct Holder {
    release: Option<std::sync::mpsc::Sender<()>>,
    owner: Option<std::thread::JoinHandle<connect_protocol::Result<bool>>>,
}
impl Holder {
    fn close(&mut self) -> bool {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        self.owner
            .take()
            .is_some_and(|owner| matches!(owner.join(), Ok(Ok(true))))
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
pub(super) async fn verify(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
    candidate_runtime: &str,
    peers: &mut [Peer],
    custody: &connect_service::canonical_media_projection_fixture::PreparationCustodyProbe,
    case: Case,
) -> Outcome {
    tokio::time::timeout(Duration::from_secs(5), custody.entered())
        .await
        .expect("reader-free cold preparation did not enter");
    let original_until = custody.original_cutoff().unwrap();
    let original = status(f, key, &command.operation_id).await;
    let before:usize=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[&original.record_id],|r|r.get(0)).unwrap();
    assert_eq!(before, 0, "cold preparation started a final attempt");
    // Preserve the once-chosen five-second publication cutoff. These short
    // control observations preserve >10 fresh frames and the same measured
    // 300ms minimum/200ms quiet tail, while reserving decision time explicitly.
    let media_until = original_until
        .checked_sub(Duration::from_millis(500))
        .unwrap();
    let mut evidence = observer::Evidence::begin(
        f,
        &original.record_id,
        &command.operation_id,
        candidate_runtime,
        custody,
        peers,
        media_until,
    )
    .await;
    if matches!(case, Case::BeginFault) {
        evidence.live_duplex(f, peers, media_until).await;
        assert!(
            tokio::time::Instant::now() + Duration::from_millis(250) < original_until,
            "actual SQL fault must be submitted before the original cutoff"
        );
        // Actual SQLite transaction has begun before this INSERT trigger
        // refuses. Public storage errors cannot prove pre-DML retry eligibility.
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        db.execute_batch("CREATE TRIGGER fixture_final_begin_failure BEFORE INSERT ON operation_effects WHEN NEW.provider='native-replacement-commit' BEGIN SELECT RAISE(ABORT,'controlled real post-BEGIN final begin failure'); END;").unwrap();
        drop(db);
        // Force one genuine try_connection WouldBlock/Ok(None), not a result stub.
        // The scoped raw holder drops before the existing async retry pause.
        let binding = custody.native_begin_binding_for_test();
        assert_eq!(binding.0, original.record_id);
        custody.exclude_next_native_begin_entry_for_test();
        custody.release();
        custody.left().await;
        tokio::time::timeout_at(original_until, async {
            while !custody.actual_begin_unconfirmed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("real failed begin did not preserve original uncertain attribution");
        let observed = custody.native_begin_snapshot_for_test();
        assert!(
            observed.none >= 1,
            "real pre-entry Busy path was not exercised"
        );
        assert_eq!(observed.pre_entry_busy, observed.none);
        assert_eq!(observed.exclusion_holds, 1);
        assert_eq!(observed.scope_refusals, 0);
        assert_eq!(observed.some, 0);
        assert_eq!(observed.errors, 1);
        assert_eq!(observed.calls, observed.none + 1);
        assert_eq!(custody.actual_begin_submissions(), observed.calls);
        assert_eq!(
            observed.sql_begins, 1,
            "exactly one actual post-BEGIN attempt"
        );
        assert_eq!(observed.before_dml, 1);
        assert_eq!(observed.insert_errors, 1);
        assert_eq!(
            observed.controlled_trigger_errors, 1,
            "actual INSERT must report SQLite 1811 and the controlled trigger text"
        );
        assert_eq!(observed.binding_mismatches, 0);
        assert_eq!(observed.unconfirmed, 1);
        assert_eq!(
            custody.actual_unpublished_returns(),
            0,
            "Core was never entered"
        );
        assert_eq!(
            custody.post_lifetime_observation().0,
            0,
            "no Core post may be fabricated"
        );
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        let count:usize=db.query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[&original.record_id],|r|r.get(0)).unwrap();
        assert_eq!(count, 0, "actual failed transaction must roll back");
        let effects: usize = db
            .query_row(
                "SELECT count(*) FROM operation_effects WHERE id=?1",
                [&binding.2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(effects, 0, "original effect INSERT must roll back");
        drop(db);
        // Removing the fault must not cause resubmission during the SAME
        // original publication interval: the owned outcome stays uncertain.
        rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .execute_batch("DROP TRIGGER fixture_final_begin_failure")
            .unwrap();
        evidence.cancelled_original(f, peers, original_until).await;
        tokio::time::sleep_until(original_until).await;
        assert_eq!(
            custody.native_begin_snapshot_for_test(),
            observed,
            "no call/SQL entry/outcome after uncertainty, even after removing the fault"
        );
        assert_eq!(custody.actual_begin_submissions(), observed.calls);
        assert_eq!(custody.native_begin_binding_for_test(), binding);
        assert_eq!(custody.actual_unpublished_returns(), 0);
        let count:usize=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[&original.record_id],|r|r.get(0)).unwrap();
        assert_eq!(
            count, 0,
            "uncertainty cannot become a fresh persisted Started/NotSubmitted"
        );
        let effects: usize = rusqlite::Connection::open(&f.config.database)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM operation_effects WHERE id=?1",
                [&binding.2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            effects, 0,
            "uncertainty must not recreate the original effect"
        );
        eprintln!("SERVICE_FINAL_BEGIN real_sql_trigger_after_begin=true typed={observed:?} actual_core_calls=0 original_ids_retained=true");
        return Outcome {
            negative: true,
            evidence,
        };
    }
    let service = f.server.service.clone();
    let session = command.session_id.clone();
    let (entered, held) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let fuse = original_until.saturating_duration_since(tokio::time::Instant::now())
        + Duration::from_millis(250);
    let owner = std::thread::spawn(move || {
        service.with_canonical_control_model_metadata_for_test(&session, || {
            let _ = entered.send(());
            released.recv_timeout(fuse).is_ok()
        })
    });
    let mut holder = Holder {
        release: Some(release),
        owner: Some(owner),
    };
    held.await
        .expect("original actual participant model was not held");
    custody.release();
    custody.left().await;
    tokio::time::timeout_at(
        original_until.min(tokio::time::Instant::now() + Duration::from_secs(1)),
        async {
            while custody.actual_unpublished_returns() == 0 {
                tokio::task::yield_now().await;
            }
        },
    )
    .await
    .expect("held actual model did not produce a real opaque StillUnpublished");
    let started=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation,e.state FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.started=1",[&original.record_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?,r.get::<_,u64>(3)?,r.get::<_,String>(4)?))).unwrap();
    assert_eq!(
        (&started.2, &started.3, started.4.as_str()),
        (&1, &1, "submitting")
    );
    // Actual Store writer acquisition while the SAME model is still excluded.
    // Old Writer-across-Core-wait code cannot satisfy this control.
    let writer_until = original_until.min(tokio::time::Instant::now() + Duration::from_secs(1));
    let writer = loop {
        match f
            .server
            .store
            .try_acquire_authority(&[connect_store::AuthorityScope::identity(
                &f.client.identity_id,
                connect_store::AuthorityMode::Write,
            )
            .unwrap()])
        {
            Ok(writer) => break writer,
            Err(_) => {
                assert!(
                    tokio::time::Instant::now() < writer_until,
                    "final retry retained an idle Store writer"
                );
                tokio::task::yield_now().await;
            }
        }
    };
    drop(writer);
    let held_writer_before = custody.counters_for_test().snapshot();
    evidence.live_duplex(f, peers, media_until).await;
    let held_writer_after = custody.counters_for_test().snapshot();
    assert_eq!(
        (
            held_writer_after.0,
            held_writer_after.1,
            held_writer_after.2
        ),
        (
            held_writer_before.0,
            held_writer_before.1,
            held_writer_before.2
        ),
        "same held model must not churn Core attempts/purpose Writers after genuine Started+Still"
    );
    assert!(
        held_writer_after.3 > held_writer_before.3,
        "actual held model must defer before reacquiring Writer"
    );
    assert!(
        holder
            .owner
            .as_ref()
            .is_some_and(|owner| !owner.is_finished()),
        "actual model exclusion ended during pre-cut observations"
    );
    assert!(
        tokio::time::Instant::now() + Duration::from_millis(250) < original_until,
        "Resume/Revoke decision must retain margin before original expiry"
    );
    let negative = !matches!(case, Case::Resume);
    if matches!(case, Case::Revoke) {
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let key = key.to_owned();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &key))
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::Instant::now() < original_until,
            "revocation must actually finish before expiry, not substitute an expiry control"
        );
    } else if matches!(case, Case::Expire) {
        tokio::time::sleep_until(original_until + Duration::from_millis(1)).await;
    }
    let joined = holder.close();
    assert!(
        joined,
        "original metadata holder did not release and join naturally"
    );
    if matches!(case, Case::PostPanic) {
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                let(starts,aliases_gone,resources_alive,observer_wait,resources_gone,accepted)=custody.post_lifetime_observation();
                if starts==1&&resources_gone {
                    assert!(accepted,"actual original retained worker did not accept cleanup");
                    assert!(aliases_gone&&resources_alive,"actual final Writer aliases must drop before send, then real outer bank fields drop");
                    assert!(!observer_wait,"unwind must not await or admit another cleanup task");
                    break;
                }
                assert!(starts<=1,"original cleanup submitted more than once");tokio::task::yield_now().await;
            }
        }).await.expect("actual publication unwind lost its post/resource custody");
        let actual:(String,u64,u64,Option<String>)=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT e.state,e.attempt,e.submission_generation,s.observation_json FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit'",[&original.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            actual,
            ("submitting".into(), 1, 1, None),
            "panic cannot manufacture Known capture or replay a possible effect"
        );
    } else if negative {
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let db = rusqlite::Connection::open(&f.config.database).unwrap();
            let actual:(String,String,u64,u64,String,Option<String>)=db.query_row("SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation,e.state,s.observation_json FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit'",[&original.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
            assert_eq!(
                (&actual.0, &actual.1, actual.2, actual.3),
                (&started.0, &started.1, 1, 1)
            );
            assert!(
                actual.5.is_none(),
                "invalid final publication acquired a false observation"
            );
            assert_ne!(actual.4, "confirmed");
            if actual.4 == "unknown" {
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "same invoked attempt did not retain uncertainty"
            );
            drop(db);
            tokio::task::yield_now().await;
        }
        evidence.cancelled_original(f, peers, until).await;
    }
    if matches!(case, Case::Resume) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let (starts, aliases_gone, resources_alive, before_wait, resources_gone, accepted) =
                    custody.post_lifetime_observation();
                if starts == 1 && before_wait {
                    assert!(accepted && aliases_gone && resources_alive && resources_gone);
                    break;
                }
                assert!(starts <= 1);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("normal final return did not send/drop owners before cleanup wait");
    }
    assert!(
        custody.final_read_released(),
        "actual latest Writer still has an alias after final turn"
    );
    eprintln!("SERVICE_FINAL_CONTROL case={case:?} actual_unpublished={} writer_free=true original_attempt={} original_effect={} holder_joined={joined} negative={negative}",custody.actual_unpublished_returns(),started.0,started.1);
    Outcome { negative, evidence }
}
