//! Join one fresh, effect-free admission before deciding whether to wait.
use connect_protocol::{Error, Result};
use std::{future::Future, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) async fn capture<T, F, Fut>(
    deadline: Instant,
    cancellation: [&CancellationToken; 3],
    mut attempt: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    loop {
        if Instant::now() >= deadline || cancellation.iter().any(|c| c.is_cancelled()) {
            return Err(Error::forbidden());
        }
        // Never race/drop an admission owner: even cancellation joins this
        // attempt. Its failed future and every partial read drop before sleep.
        let result = attempt().await;
        if Instant::now() >= deadline || cancellation.iter().any(|c| c.is_cancelled()) {
            drop(result);
            return Err(Error::forbidden());
        }
        match result {
            Ok(value) => return Ok(value),
            Err(error) if error.code == "authority-busy" => {}
            Err(error) => return Err(error),
        }
        tokio::select! { biased;
            _ = cancellation[0].cancelled() => return Err(Error::forbidden()),
            _ = cancellation[1].cancelled() => return Err(Error::forbidden()),
            _ = cancellation[2].cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(deadline.min(Instant::now()+Duration::from_millis(20))) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[derive(Debug)]
    struct Held(Arc<AtomicUsize>);
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn busy_scene_drops_previous_authority_before_fresh_capture_and_one_effect() {
        let stop = CancellationToken::new();
        let physical = CancellationToken::new();
        let caller = CancellationToken::new();
        let held = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let effects = AtomicUsize::new(0);
        let original = Arc::new(());
        let expected = Arc::as_ptr(&original);
        let value = capture(
            Instant::now() + Duration::from_secs(1),
            [&stop, &physical, &caller],
            || async {
                assert_eq!(
                    held.load(Ordering::SeqCst),
                    0,
                    "a previous capture retained current authority across the wait"
                );
                held.fetch_add(1, Ordering::SeqCst);
                let _guard = Held(held.clone());
                assert_eq!(Arc::as_ptr(&original), expected);
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::new("authority-busy", "actual capture excluded"))
                } else {
                    Ok(17)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(value, 17);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(held.load(Ordering::SeqCst), 0);
        effects.fetch_add(1, Ordering::SeqCst);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn permanent_scene_refusal_and_storage_error_never_retry_or_activate() {
        for code in [
            "forbidden",
            "authority-unavailable",
            "storage-unavailable",
            "not-found",
            "conflict",
        ] {
            let stop = CancellationToken::new();
            let physical = CancellationToken::new();
            let caller = CancellationToken::new();
            let calls = AtomicUsize::new(0);
            let result = capture(
                Instant::now() + Duration::from_secs(1),
                [&stop, &physical, &caller],
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(Error::new(code, "permanent original refusal"))
                },
            )
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn cancellation_or_current_revocation_after_busy_cannot_reuse_previous_capture() {
        for mode in ["stop", "physical", "caller", "revoke"] {
            let stop = CancellationToken::new();
            let physical = CancellationToken::new();
            let caller = CancellationToken::new();
            let calls = AtomicUsize::new(0);
            let result = capture(
                Instant::now() + Duration::from_secs(1),
                [&stop, &physical, &caller],
                || async {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        match mode {
                            "stop" => stop.cancel(),
                            "physical" => physical.cancel(),
                            "caller" => caller.cancel(),
                            _ => {}
                        }
                        Err::<(), _>(Error::new("authority-busy", "original capture excluded"))
                    } else {
                        Err(Error::forbidden())
                    }
                },
            )
            .await;
            assert_eq!(result.unwrap_err().code, "forbidden");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                if mode == "revoke" { 2 } else { 1 }
            );
        }
    }

    #[tokio::test]
    async fn original_expiry_joins_inflight_capture_and_refuses_its_late_positive_result() {
        let stop = CancellationToken::new();
        let physical = CancellationToken::new();
        let caller = CancellationToken::new();
        let held = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let entered = Instant::now();
        let deadline = entered + Duration::from_millis(10);
        let result = capture(deadline, [&stop, &physical, &caller], || async {
            calls.fetch_add(1, Ordering::SeqCst);
            held.fetch_add(1, Ordering::SeqCst);
            let guard = Held(held.clone());
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(guard)
        })
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(held.load(Ordering::SeqCst), 0);
        assert!(entered.elapsed() >= Duration::from_millis(30));
        assert_eq!(
            deadline,
            entered + Duration::from_millis(10),
            "no renewed capture lifetime"
        );
    }
}
