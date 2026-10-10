//! Drain all owned tasks even when one fails; a panic is not successful shutdown.
use std::time::Duration;
use tokio::task::JoinHandle;

/// Observe normal errors, unexpected completion, panic and abort immediately.
/// Dropping a worker during requested shutdown is not an operational failure.
pub(crate) fn spawn<F>(
    readiness: std::sync::Arc<connect_service::readiness::OperationalReadiness>,
    name: &'static str,
    stop: tokio_util::sync::CancellationToken,
    future: F,
) -> JoinHandle<anyhow::Result<()>>
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    struct Exit {
        readiness: std::sync::Arc<connect_service::readiness::OperationalReadiness>,
        name: &'static str,
        stop: tokio_util::sync::CancellationToken,
    }
    impl Drop for Exit {
        fn drop(&mut self) {
            if self.stop.is_cancelled() {
                self.readiness.set(
                    self.name,
                    connect_protocol::ReadinessState::Unavailable,
                    "stopped",
                );
            } else {
                self.readiness.fail(self.name);
            }
        }
    }
    readiness.set(
        name,
        connect_protocol::ReadinessState::Ready,
        "worker-running",
    );
    let exit = Exit {
        readiness,
        name,
        stop,
    };
    tokio::spawn(async move {
        let exit = exit;
        let result = future.await;
        if result.is_ok() && !exit.stop.is_cancelled() {
            anyhow::bail!("Worker exited unexpectedly");
        }
        result
    })
}

pub(crate) async fn drain(
    tasks: Vec<JoinHandle<anyhow::Result<()>>>,
    timeout: Duration,
) -> anyhow::Result<()> {
    // If the caller cancels shutdown, dropping JoinHandles alone would detach
    // their tasks. Keep ownership in an abort-on-drop guard until all are joined.
    struct OwnedTasks(Vec<JoinHandle<anyhow::Result<()>>>);
    impl Drop for OwnedTasks {
        fn drop(&mut self) {
            for task in &self.0 {
                task.abort();
            }
        }
    }
    let mut owned = OwnedTasks(tasks);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut failures = Vec::new();
    for task in &mut owned.0 {
        match tokio::time::timeout_at(deadline, &mut *task).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => failures.push(error.to_string()),
            Ok(Err(_)) => failures.push("Worker task panicked or was cancelled".into()),
            Err(_) => {
                task.abort();
                let _ = (&mut *task).await;
                failures.push("Worker shutdown timed out; recover uncertain work".into());
            }
        }
    }
    anyhow::ensure!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[tokio::test]
    async fn supervised_panic_wakes_readiness_without_waiting_for_shutdown() {
        let readiness =
            std::sync::Arc::new(connect_service::readiness::OperationalReadiness::default());
        let stop = tokio_util::sync::CancellationToken::new();
        let task = spawn(readiness.clone(), "fixture", stop, async {
            panic!("fixture panic");
            #[allow(unreachable_code)]
            Ok(())
        });
        tokio::time::timeout(Duration::from_secs(1), readiness.failed())
            .await
            .expect("failure notification");
        assert_eq!(
            readiness.snapshot().state,
            connect_protocol::ReadinessState::Unavailable
        );
        assert!(task.await.is_err());
    }
    #[tokio::test]
    async fn panic_is_reported_and_other_tasks_are_still_drained() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let bad = tokio::spawn(async {
            panic!("fixture worker panic");
            #[allow(unreachable_code)]
            Ok(())
        });
        let other = tokio::spawn(async move {
            tokio::task::yield_now().await;
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(drain(vec![bad, other], Duration::from_secs(1))
            .await
            .is_err());
        assert!(finished.load(Ordering::SeqCst));
    }
    #[tokio::test]
    async fn failure_does_not_detach_a_stalled_sibling() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let bad = tokio::spawn(async { anyhow::bail!("fixture storage failure") });
        let stalled = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
            Ok(())
        });
        assert!(drain(vec![bad, stalled], Duration::from_millis(20))
            .await
            .is_err());
        assert!(dropped.load(Ordering::SeqCst));
    }
    #[tokio::test]
    async fn cancelling_shutdown_aborts_owned_tasks() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let _guard = guard;
            let _ = started.send(());
            std::future::pending::<()>().await;
            Ok(())
        });
        assert!(ready.await.is_ok());
        let (draining, owned) = tokio::sync::oneshot::channel();
        let shutdown = tokio::spawn(async move {
            let future = drain(vec![worker], Duration::from_secs(60));
            tokio::pin!(future);
            tokio::select! {
                biased;
                result = &mut future => result,
                _ = async { let _ = draining.send(()); } => future.await,
            }
        });
        assert!(owned.await.is_ok());
        shutdown.abort();
        assert!(shutdown.await.is_err());
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("owned task must be aborted");
    }
}
