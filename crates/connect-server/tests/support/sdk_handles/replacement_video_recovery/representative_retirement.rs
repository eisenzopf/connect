//! Phase all original Session cleanup calls under one unchanged absolute cutoff.
//! Fixture scheduling only: no policy grant, new client, effect retry or budget.
use super::*;
use futures_util::{future::join_all, FutureExt};
use std::{future::Future, pin::Pin};

type Attempt<'a, T> = Pin<Box<dyn Future<Output = std::result::Result<T, String>> + 'a>>;
struct Report<T> {
    completed: Vec<(usize, T)>,
    already_terminal: Vec<usize>,
    failures: Vec<String>,
}

async fn observe<T>(
    index: usize,
    phase: &'static str,
    until: Instant,
    began: Instant,
    attempt: Attempt<'_, T>,
) -> std::result::Result<T, String> {
    let now = Instant::now();
    eprintln!("representative Session retirement index={index} phase={phase} stage=entry programme_us={} remaining_us={}",
        now.saturating_duration_since(began).as_micros(), until.saturating_duration_since(now).as_micros());
    // An unpolled callback owns no submitted request. Never start a later phase
    // with a renewed timeout after the original cohort cutoff.
    if now >= until {
        return Err(format!("original Session cleanup index={index} phase={phase}: original cutoff elapsed before request"));
    }
    let result = match AssertUnwindSafe(attempt).catch_unwind().await {
        Ok(result) => result,
        Err(panic) => Err(panic_text(panic)),
    };
    eprintln!("representative Session retirement index={index} phase={phase} stage=returned programme_us={} ok={}",
        Instant::now().saturating_duration_since(began).as_micros(), result.is_ok());
    result.map_err(|error| format!("original Session cleanup index={index} phase={phase}: {error}"))
}

async fn phases<'a, A: 'a, B: 'a, C: 'a>(
    count: usize,
    until: Instant,
    began: Instant,
    read: impl Fn(usize) -> Attempt<'a, A>,
    submit: impl Fn(usize, A) -> Attempt<'a, Option<B>>,
    wait: impl Fn(usize, B) -> Attempt<'a, C>,
) -> Report<C> {
    let mut report = Report {
        completed: Vec::new(),
        already_terminal: Vec::new(),
        failures: Vec::new(),
    };
    let reads =
        join_all((0..count).map(|index| observe(index, "refresh", until, began, read(index))))
            .await;
    let mut submissions = Vec::new();
    let submit = &submit;
    for (index, read) in reads.into_iter().enumerate() {
        match read {
            Ok(value) => submissions.push(async move {
                (
                    index,
                    observe(index, "end", until, began, submit(index, value)).await,
                )
            }),
            Err(error) => report.failures.push(error),
        }
    }
    let ended = join_all(submissions).await;
    let mut waits = Vec::new();
    let wait = &wait;
    for (index, ended) in ended {
        match ended {
            Ok(Some(value)) => waits.push(async move {
                (
                    index,
                    observe(index, "wait", until, began, wait(index, value)).await,
                )
            }),
            Ok(None) => report.already_terminal.push(index),
            Err(error) => report.failures.push(error),
        }
    }
    for (index, waited) in join_all(waits).await {
        match waited {
            Ok(value) => report.completed.push((index, value)),
            Err(error) => report.failures.push(error),
        }
    }
    report
}

pub(super) struct Retirement {
    pub completed: Vec<(usize, sdk::Completion<sdk::Session>)>,
    pub failures: Vec<String>,
}
pub(super) async fn retire(
    sessions: &[sdk::Session],
    until: Instant,
    began: Instant,
) -> Retirement {
    let report = phases(
        sessions.len(),
        until,
        began,
        |index| {
            let session = sessions[index].clone();
            Box::pin(async move {
                session
                    .refresh(programme::request(until))
                    .await
                    .map_err(|error| format!("{error:?}"))
            })
        },
        |index, before: sdk::SessionSnapshot| {
            let session = sessions[index].clone();
            Box::pin(async move {
                if before.state == "ended" {
                    return Ok(None);
                }
                session
                    .end(
                        0,
                        "representative original cleanup".into(),
                        before.topology_revision,
                        programme::mutation_at(
                            &format!("representative-session-end-{index}"),
                            until,
                        ),
                    )
                    .await
                    .map(Some)
                    .map_err(|error| format!("{error:?}"))
            })
        },
        |_, operation: sdk::Operation<sdk::Session>| {
            Box::pin(async move {
                operation
                    .wait(programme::request(until))
                    .await
                    .map_err(|error| format!("{error:?}"))
            })
        },
    )
    .await;
    eprintln!("representative Session retirement completed={} already_ended={} failures={} original_cutoff_ms={}",
        report.completed.len(), report.already_terminal.len(), report.failures.len(),
        until.saturating_duration_since(began).as_millis());
    Retirement {
        completed: report.completed,
        failures: report.failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn session_retirement_batches_all_current_reads_and_submissions_before_waits() {
        let reads = Arc::new(AtomicUsize::new(0));
        let submitted = Arc::new(AtomicUsize::new(0));
        let read = reads.clone();
        let end_read = reads.clone();
        let end_submitted = submitted.clone();
        let wait_submitted = submitted.clone();
        let began = Instant::now();
        let until = began + Duration::from_secs(1);
        let report = phases(
            2,
            until,
            began,
            move |index| {
                let read = read.clone();
                Box::pin(async move {
                    tokio::task::yield_now().await;
                    read.fetch_or(1 << index, Ordering::SeqCst);
                    Ok(index)
                })
            },
            move |index, value| {
                let reads = end_read.clone();
                let submitted = end_submitted.clone();
                Box::pin(async move {
                    assert_eq!(
                        reads.load(Ordering::SeqCst),
                        3,
                        "a Session End began before both original current reads joined"
                    );
                    tokio::task::yield_now().await;
                    submitted.fetch_or(1 << index, Ordering::SeqCst);
                    Ok(Some(value))
                })
            },
            move |index, value| {
                let submitted = wait_submitted.clone();
                Box::pin(async move {
                    assert_eq!(
                        submitted.load(Ordering::SeqCst),
                        3,
                        "one wait occupied the helper before both original End submissions joined"
                    );
                    assert_eq!(index, value);
                    Ok(value)
                })
            },
        )
        .await;
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.completed, vec![(0, 0), (1, 1)]);
        assert!(report.already_terminal.is_empty());
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        assert_eq!(submitted.load(Ordering::SeqCst), 3);
        assert!(Instant::now() < until);
    }

    #[tokio::test]
    async fn session_retirement_joins_original_phase_siblings_and_preserves_errors_and_cutoff() {
        let waited = Arc::new(AtomicUsize::new(0));
        let observed = waited.clone();
        let began = Instant::now();
        let until = began + Duration::from_secs(1);
        let report = phases(
            2,
            until,
            began,
            |index| Box::pin(async move { Ok(index) }),
            |_, value| Box::pin(async move { Ok(Some(value)) }),
            move |index, value| {
                let waited = waited.clone();
                Box::pin(async move {
                    tokio::task::yield_now().await;
                    waited.fetch_add(1, Ordering::SeqCst);
                    if index == 0 {
                        panic!("original wait failure");
                    }
                    Ok(value)
                })
            },
        )
        .await;
        assert_eq!(
            observed.load(Ordering::SeqCst),
            2,
            "first panic dropped another original wait"
        );
        assert_eq!(report.completed, vec![(1, 1)]);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].contains("index=0 phase=wait: original wait failure"));

        let report: Report<usize> = phases(
            2,
            until,
            began,
            |index| {
                Box::pin(async move {
                    if index == 0 {
                        Err("original read failure".into())
                    } else {
                        Ok(index)
                    }
                })
            },
            |_, _| Box::pin(async { Err("original End failure".into()) }),
            |_, _value: usize| Box::pin(async { panic!("failed End cannot start a wait") }),
        )
        .await;
        assert_eq!(report.failures.len(), 2);
        assert!(report.failures[0].contains("phase=refresh: original read failure"));
        assert!(report.failures[1].contains("phase=end: original End failure"));
        let calls = AtomicUsize::new(0);
        let expired = Instant::now();
        let report: Report<usize> = phases(
            2,
            expired,
            began,
            |_| {
                Box::pin(async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
            |_, ()| Box::pin(async { panic!("expired read cannot submit End") }),
            |_, _value: usize| Box::pin(async { panic!("expired End cannot wait") }),
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(report.failures.len(), 2);
        assert!(report
            .failures
            .iter()
            .all(|e| e.contains("original cutoff elapsed before request")));
    }
}
