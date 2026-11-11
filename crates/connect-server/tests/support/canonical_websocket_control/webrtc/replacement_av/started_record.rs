//! Real post-activation metadata contention, never a fabricated media result.
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
                "same Start receipt rendezvous expected={expected} actual={actual}"
            )
            .into());
        }
        tokio::time::sleep_until(until.min(tokio::time::Instant::now() + Duration::from_millis(1)))
            .await;
    }
}

pub(super) async fn settle_under_real_sql_exclusion(
    f: &Fixture,
    record: &str,
    probe: Arc<AtomicUsize>,
    until: tokio::time::Instant,
) -> TestResult<()> {
    let mut release = None;
    let mut worker = None;
    let observed=AssertUnwindSafe(async {
        reached(&probe,1,until).await?;
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let before:(String,String,u64,u64,String,bool)=sql.query_row(
            "SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation,e.state,s.observation_json IS NULL FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.started=1",
            [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        assert_eq!((before.2,before.3,before.4.as_str(),before.5),(1,1,"submitting",true));
        let media_phases:u64=sql.query_row("SELECT COUNT(*) FROM native_replacement_media_steps WHERE operation_record_id=?1",[record],|r|r.get(0))?;
        assert_eq!(media_phases,0,"no private preparation before recorded Start");
        drop(sql);
        let store=f.server.store.clone();
        let(entered_tx,entered_rx)=oneshot::channel();
        let(release_tx,release_rx)=oneshot::channel();
        release=Some(release_tx);
        worker=Some(tokio::task::spawn_blocking(move ||store.with_primary_sql_held_for_test(|| {
            let _=entered_tx.send(());
            let _=release_rx.blocking_recv();
        })));
        tokio::time::timeout_at(until,entered_rx).await??;
        probe.store(2,Ordering::Release);
        // This transition is emitted only by the ACTUAL failed Store capture
        // after its original blocking owner returned and released all guards.
        reached(&probe,3,until).await?;
        release.take().unwrap().send(()).map_err(|_|"original SQL owner already ended")?;
        worker.take().unwrap().await??;
        reached(&probe,4,until).await?;
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let after:(String,String,u64,u64,String,bool)=sql.query_row(
            "SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation,e.state,s.observation_json IS NOT NULL FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.started=1",
            [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        assert_eq!((&before.0,&before.1,before.2,before.3),(&after.0,&after.1,after.2,after.3));
        assert_eq!((after.4.as_str(),after.5),("confirmed",true));
        assert!(tokio::time::Instant::now()<until);
        Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
    }).catch_unwind().await;
    // No failure can leave primary SQL held or detach its original worker.
    drop(release.take());
    if let Some(worker) = worker.take() {
        worker.await??;
    }
    observed.map_err(|_| "actual captured Start assertion panicked")?
}

/// Both hooks run AFTER fresh policy admission and before the exact chosen SQL
/// boundary. The only retained blocker is the REAL original primary SQL owner.
pub(super) async fn settle_before_start_under_real_sql_exclusion(
    f: &Fixture,
    _cid: &str,
    record: &str,
    probe: Arc<AtomicUsize>,
    until: tokio::time::Instant,
) -> TestResult<()> {
    let mut release = None;
    let mut worker = None;
    let observed=AssertUnwindSafe(async {
        reached(&probe,1,until).await?;
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let starts:u64=sql.query_row("SELECT COUNT(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start'",[record],|r|r.get(0))?;
        assert_eq!(starts,0);drop(sql);
        let store=f.server.store.clone();
        let(entered_tx,entered_rx)=oneshot::channel();let(release_tx,release_rx)=oneshot::channel();
        release=Some(release_tx);
        worker=Some(tokio::task::spawn_blocking(move ||store.with_primary_sql_held_for_test(|| {
            let _=entered_tx.send(());let _=release_rx.blocking_recv();
        })));
        tokio::time::timeout_at(until,entered_rx).await??;
        probe.store(2,Ordering::Release);reached(&probe,3,until).await?;
        // Production's test-only phase3 is emitted only after its exact
        // original/reply AND fresh command scope-set Weak probes observed live
        // custody and then observed zero surviving aliases after joined SQL.
        // A healthy old/retained media publication has an independent scope
        // set and stays admitted; global exclusive-file availability cannot
        // distinguish that valid authority from a leaked command alias.
        assert_eq!(probe.load(Ordering::Acquire),3,"exact invocation custody released without revoking healthy media");
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let starts:u64=sql.query_row("SELECT COUNT(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start'",[record],|r|r.get(0))?;
        assert_eq!(starts,0,"no original Start transaction/effect before successful SQL admission");drop(sql);
        release.take().unwrap().send(()).map_err(|_|"original SQL owner already ended")?;
        worker.take().unwrap().await??;reached(&probe,4,until).await?;
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let counts:(u64,u64,u64)=sql.query_row("SELECT COUNT(*),MAX(e.attempt),MAX(e.submission_generation) FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.started=1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        assert_eq!(counts,(1,1,1));assert!(tokio::time::Instant::now()<until);
        Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
    }).catch_unwind().await;
    drop(release.take());
    if let Some(worker) = worker.take() {
        worker.await??;
    }
    observed.map_err(|_| "actual original RTC preStart assertion panicked")?
}
