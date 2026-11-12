//! One original signed Ready traverses both real primary SQL exclusions.
use super::*;
use futures_util::FutureExt;
use std::{
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::oneshot;

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn reached(
    probe: &AtomicUsize,
    expected: usize,
    until: tokio::time::Instant,
) -> TestResult<()> {
    loop {
        let actual = probe.load(Ordering::Acquire);
        if actual == expected {
            return Ok(());
        }
        if actual > expected || tokio::time::Instant::now() >= until {
            return Err(format!(
                "same original Ready rendezvous expected={expected} actual={actual}"
            )
            .into());
        }
        tokio::time::sleep_until(until.min(tokio::time::Instant::now() + Duration::from_millis(1)))
            .await;
    }
}

// Resume markers deliberately are not numerically monotonic. Accept only the
// exact original resume and its known intervening phase, never an arbitrary
// larger value (including the custody-failure marker99).
async fn reached_after_resume(
    probe: &AtomicUsize,
    resume: usize,
    expected: usize,
    until: tokio::time::Instant,
) -> TestResult<()> {
    loop {
        let actual = probe.load(Ordering::Acquire);
        let reached = matches!((resume, expected, actual), (9, 5, 5) | (10, 8, 8));
        let pending = matches!((resume, expected, actual), (9, 5, 9 | 4) | (10, 8, 10));
        if reached && tokio::time::Instant::now() < until {
            return Ok(());
        }
        if !pending || tokio::time::Instant::now() >= until {
            return Err(format!(
                "same original Ready resume={resume} expected={expected} actual={actual}"
            )
            .into());
        }
        tokio::time::sleep_until(until.min(tokio::time::Instant::now() + Duration::from_millis(1)))
            .await;
    }
}

fn original_resources(
    f: &Fixture,
    record: &str,
) -> TestResult<Vec<(String, String, u64, u64, String)>> {
    let sql = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut statement = sql.prepare("SELECT provider,id,attempt,submission_generation,state FROM operation_effects WHERE operation_record_id=?1 AND provider IN ('native-replacement-prepare','native-replacement-allocate','native-replacement-start','native-replacement-attach','native-replacement-activate') ORDER BY provider")?;
    let rows: Vec<(String, String, u64, u64, String)> = statement
        .query_map([record], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        rows.len(),
        5,
        "all exact original resources exist before Ready"
    );
    for row in &rows {
        assert_eq!((row.2, row.3, row.4.as_str()), (1, 1, "confirmed"));
    }
    Ok(rows)
}

pub(super) async fn settle_both_under_real_sql_exclusion(
    f: &Fixture,
    record: &str,
    probe: Arc<AtomicUsize>,
    until: tokio::time::Instant,
) -> TestResult<()> {
    let mut release = None;
    let mut worker = None;
    let original = original_resources(f, record)?;
    let observed = AssertUnwindSafe(async {
        for (before, held, dropped, after) in [(1,2,3,5),(5,6,7,8)] {
            reached(&probe,before,until).await?;
            assert_eq!(original_resources(f,record)?,original);
            if before==1 {
                let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let committed:u64=sql.query_row("SELECT COUNT(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1",[record],|r|r.get(0))?;
                assert_eq!(committed,0,"no Commit before the original valid Ready capture");
            }
            let store=f.server.store.clone();
            let(entered_tx,entered_rx)=oneshot::channel();
            let(release_tx,release_rx)=oneshot::channel();
            release=Some(release_tx);
            worker=Some(tokio::task::spawn_blocking(move || store.with_primary_sql_held_for_test(|| {
                let _=entered_tx.send(());
                let _=release_rx.blocking_recv();
            })));
            tokio::time::timeout_at(until,entered_rx).await??;
            probe.store(held,Ordering::Release);
            // The actual failed factory emits this ONLY after all original,
            // reply and fresh exact custody aliases have been dropped.
            reached(&probe,dropped,until).await?;
            assert_eq!(original_resources(f,record)?,original);
            release.take().unwrap().send(()).map_err(|_|"original Ready SQL owner already ended")?;
            worker.take().unwrap().await??;
            let resume=if dropped==3 {9} else {10};
            probe.store(resume,Ordering::Release);
            reached_after_resume(&probe,resume,after,until).await?;
            assert_eq!(original_resources(f,record)?,original);
        }
        assert_eq!(probe.load(Ordering::Acquire),8);
        assert!(tokio::time::Instant::now()<until,"the original Ready cutoff never renews");
        Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
    }).catch_unwind().await;
    // No SQL owner or original blocking task can outlive a failed assertion.
    drop(release.take());
    let joined = if let Some(worker) = worker.take() {
        Some(worker.await)
    } else {
        None
    };
    match observed {
        Err(panic) => std::panic::resume_unwind(panic),
        Ok(Err(error)) => {
            if let Some(Err(join)) = joined {
                eprintln!("secondary Ready SQL join failure: {join}");
            }
            Err(error)
        }
        Ok(Ok(())) => {
            if let Some(joined) = joined {
                joined??;
            }
            Ok(())
        }
    }
}

/// A genuine Writer mutation happens only after the actual failed SQL factory
/// dropped the exact original and fresh authority; no fake guard or SQL edit.
pub(super) async fn revoke_after_pre_capture_busy(
    f: &Fixture,
    record: &str,
    credential: &str,
    probe: Arc<AtomicUsize>,
    until: tokio::time::Instant,
) -> TestResult<()> {
    let admin = f.server.store.authenticate(&f.admin)?;
    let original = original_resources(f, record)?;
    let mut release = None;
    let mut worker = None;
    let observed=AssertUnwindSafe(async {
        reached(&probe,1,until).await?;
        let store=f.server.store.clone();
        let(entered_tx,entered_rx)=oneshot::channel();
        let(release_tx,release_rx)=oneshot::channel();
        release=Some(release_tx);
        worker=Some(tokio::task::spawn_blocking(move ||store.with_primary_sql_held_for_test(|| {
            let _=entered_tx.send(()); let _=release_rx.blocking_recv();
        })));
        tokio::time::timeout_at(until,entered_rx).await??;
        probe.store(2,Ordering::Release);
        reached(&probe,3,until).await?;
        assert_eq!(original_resources(f,record)?,original);
        release.take().unwrap().send(()).map_err(|_|"original Ready SQL owner already ended")?;
        worker.take().unwrap().await??;
        let store=f.server.store.clone();let credential=credential.to_owned();
        // Keep and join the original real Writer task even on failure. The
        // same original Ready stays at its test rendezvous until this finishes.
        let revoked=tokio::task::spawn_blocking(move ||store.revoke(&admin,&credential)).await??;
        let _=revoked;
        assert_eq!(probe.load(Ordering::Acquire),3,"no capture while original Ready waits grant-free");
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let accepted:u64=sql.query_row("SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1 AND provider IN ('native-replacement-readiness','native-replacement-commit') AND state='confirmed'",[record],|r|r.get(0))?;
        assert_eq!(accepted,0,"real revocation precedes Ready acceptance and Commit");
        assert!(tokio::time::Instant::now()<until);
        probe.store(9,Ordering::Release);
        Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
    }).catch_unwind().await;
    drop(release.take());
    let joined = if let Some(worker) = worker.take() {
        Some(worker.await)
    } else {
        None
    };
    match observed {
        Err(panic) => std::panic::resume_unwind(panic),
        Ok(Err(error)) => {
            if let Some(Err(join)) = joined {
                eprintln!("secondary revoked Ready SQL join failure: {join}");
            }
            Err(error)
        }
        Ok(Ok(())) => {
            if let Some(joined) = joined {
                joined??;
            }
            Ok(())
        }
    }
}

/// Read and verify the same signed request, or observe its real original
/// websocket terminal Close. Timeout, malformed text, foreign replies and IO
/// uncertainty are never substituted for a refusal or successful cleanup.
pub(super) async fn read_original_refusal_or_close(
    peer: &mut Peer,
    ready: Value,
) -> ReadyNegativeReceipt {
    for field in ["cid", "sid", "connid", "operation_id"] {
        assert!(
            ready[field].as_str().is_some_and(|value| !value.is_empty()),
            "original Ready must freeze every exact resource scope"
        );
    }
    let original = ready.clone();
    let request = peer.send(ready, true).await;
    for _ in 0..128 {
        match peer.socket.next().await {
            Some(Ok(Message::Close(_))) | None => return ReadyNegativeReceipt::TerminalClose,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Text(text))) => {
                let event = Envelope::parse(text.as_bytes()).unwrap();
                peer.verifier
                    .verify_with_context(event.original(), &peer.context)
                    .await
                    .unwrap();
                assert_eq!(event.original()["in_reply_to"], request);
                assert!(
                    original_ready_scope_matches(&original, event.original()),
                    "same-request foreign resource reply cannot qualify Ready refusal"
                );
                match event.kind() {
                    "error" => {
                        let problem = public_problem(&event.original()["payload"]);
                        assert!(is_public_refusal_shape(&problem));
                        return ReadyNegativeReceipt::Problem(problem);
                    }
                    "operation.updated" => {
                        assert!(
                            original_ready_update_matches(&original, event.original()),
                            "Ready refusal progress must name exact original replacement record"
                        );
                        assert!(is_refusal_progress(
                            event.original()["payload"]["state"].as_str()
                        ));
                    }
                    _ => panic!(
                        "revoked original Ready acquired unexpected receipt kind={}",
                        event.kind()
                    ),
                }
            }
            other => panic!(
                "original Ready refusal must be signed problem or actual terminal Close: {other:?}"
            ),
        }
    }
    panic!("revoked original Ready response exceeded bounded event count");
}

fn original_ready_scope_matches(original: &Value, reply: &Value) -> bool {
    ["cid", "sid", "connid", "operation_id"]
        .iter()
        .all(|field| {
            original[*field]
                .as_str()
                .is_some_and(|value| !value.is_empty())
                && original[*field].as_str() == reply[*field].as_str()
        })
}
fn original_ready_update_matches(original: &Value, reply: &Value) -> bool {
    let record = original["payload"]["replacement_preparation"]["operation_record_id"].as_str();
    record.is_some_and(|value| !value.is_empty())
        && original["operation_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        && reply["payload"]["record_id"].as_str() == record
        && reply["payload"]["operation_id"].as_str() == original["operation_id"].as_str()
        && reply["payload"]["command_type"] == "connection.replace"
}

#[tokio::test]
async fn ready_refusal_scope_requires_all_four_original_resources_and_exact_update_record() {
    let original = json!({"cid":"c","sid":"s","connid":"n","operation_id":"op",
        "payload":{"replacement_preparation":{"operation_record_id":"record"}}});
    let reply = json!({"cid":"c","sid":"s","connid":"n","operation_id":"op",
        "payload":{"record_id":"record","operation_id":"op","command_type":"connection.replace","state":"failed"}});
    assert!(original_ready_scope_matches(&original, &reply));
    assert!(original_ready_update_matches(&original, &reply));
    for field in ["cid", "sid", "connid", "operation_id"] {
        let mut foreign = reply.clone();
        foreign[field] = json!("foreign");
        assert!(!original_ready_scope_matches(&original, &foreign));
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(!original_ready_scope_matches(&missing, &reply));
    }
    for field in ["record_id", "operation_id", "command_type"] {
        let mut foreign = reply.clone();
        foreign["payload"][field] = json!("foreign");
        assert!(!original_ready_update_matches(&original, &foreign));
        let mut missing = reply.clone();
        missing["payload"].as_object_mut().unwrap().remove(field);
        assert!(!original_ready_update_matches(&original, &missing));
    }
    let mut missing = original;
    missing["payload"]["replacement_preparation"]
        .as_object_mut()
        .unwrap()
        .remove("operation_record_id");
    assert!(!original_ready_update_matches(&missing, &reply));

    use std::{future::Future, task::Poll};
    // Poll the ACTUAL waiter while the original resume is still visible. The
    // former numeric-order waiter returned an error on this very first poll.
    // A separate initial4 case proves that intervening phase is also pending.
    for (initial, resume, expected, intermediate) in
        [(9, 9, 5, Some(4)), (4, 9, 5, None), (10, 10, 8, None)]
    {
        let probe = AtomicUsize::new(initial);
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(1);
        let mut waiter = Box::pin(reached_after_resume(&probe, resume, expected, cutoff));
        let first_poll = std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx))).await;
        assert!(first_poll.is_pending(), "valid original resume was refused");
        let advance = async {
            if let Some(phase) = intermediate {
                probe.store(phase, Ordering::Release);
                tokio::task::yield_now().await;
            }
            probe.store(expected, Ordering::Release);
        };
        // Both original futures remain owned and joined by this unit; no task
        // or timer can survive an assertion or acquire a renewed cutoff.
        let (result, ()) = tokio::join!(waiter, advance);
        result.unwrap();
        assert_eq!(probe.load(Ordering::Acquire), expected);
        assert!(tokio::time::Instant::now() < cutoff);
    }
    for (resume, expected, invalid) in [
        (9, 5, vec![0, 1, 2, 3, 6, 7, 8, 10, 99]),
        (10, 8, vec![0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 99]),
    ] {
        for phase in invalid {
            let probe = AtomicUsize::new(phase);
            let cutoff = tokio::time::Instant::now() + Duration::from_secs(1);
            assert!(reached_after_resume(&probe, resume, expected, cutoff)
                .await
                .is_err());
            assert_eq!(probe.load(Ordering::Acquire), phase);
        }
        let probe = AtomicUsize::new(expected);
        let expired = tokio::time::Instant::now();
        assert!(reached_after_resume(&probe, resume, expected, expired)
            .await
            .is_err());
        assert_eq!(probe.load(Ordering::Acquire), expected);
    }
}
