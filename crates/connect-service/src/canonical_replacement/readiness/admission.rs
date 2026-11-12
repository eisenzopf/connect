//! Same-original Ready metadata admission. Every failed owner joins and drops
//! all current proof/read custody before this bounded, grant-free retry pause.
use connect_protocol::{Error, Result};
use rvoip_uctp::v2::auth_dispatch::DispatchError;
use std::{future::Future, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) type Attempt<'a, T> = std::pin::Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(super) fn authority_error(error: DispatchError) -> Error {
    match error {
        DispatchError::AuthorityBusy => {
            Error::new("authority-busy", "Ready policy admission is busy")
        }
        DispatchError::AuthorityUnavailable => {
            Error::new("authority-unavailable", "Ready authority is unavailable")
        }
        DispatchError::StorageUnavailable => Error::storage(),
        DispatchError::Capacity => {
            Error::new("capacity-exceeded", "Ready admission capacity exceeded")
        }
        _ => Error::forbidden(),
    }
}

pub(super) async fn capture<T, F, Fut>(
    until: Instant,
    cancellation: [&CancellationToken; 4],
    stage: &'static str,
    mut attempt: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut reported = false;
    loop {
        if Instant::now() >= until || cancellation.iter().any(|t| t.is_cancelled()) {
            return Err(Error::forbidden());
        }
        let result = attempt().await; // Join original policy/SQL owner even on cancellation.
        if Instant::now() >= until || cancellation.iter().any(|t| t.is_cancelled()) {
            drop(result);
            return Err(Error::forbidden());
        }
        match result {
            Ok(value) => return Ok(value),
            Err(error) if error.code == "authority-busy" => {
                if !reported {
                    crate::peer_authority::observe_unavailable(
                        "replacement-ready",
                        "connection.offer",
                        stage,
                        &error,
                    );
                    tracing::debug!(target:"connect_service::canonical_replacement", stage, code=%error.code,
                        "Original Ready awaits fresh metadata after all current guards dropped");
                    reported = true;
                }
            }
            Err(error) => {
                crate::peer_authority::observe_unavailable(
                    "replacement-ready",
                    "connection.offer",
                    stage,
                    &error,
                );
                tracing::debug!(target:"connect_service::canonical_replacement",stage,code=%error.code,
                    "Original Ready metadata admission refused after its current guards dropped");
                return Err(error);
            }
        }
        tokio::select! {biased;
            _=cancellation[0].cancelled()=>return Err(Error::forbidden()),
            _=cancellation[1].cancelled()=>return Err(Error::forbidden()),
            _=cancellation[2].cancelled()=>return Err(Error::forbidden()),
            _=cancellation[3].cancelled()=>return Err(Error::forbidden()),
            _=tokio::time::sleep_until(until.min(Instant::now()+Duration::from_millis(20)))=>{},
        }
    }
}

// Linearize the test rendezvous transition. In particular, a stale observed
// dropped phase3/7 must never overwrite its companion's resume9/10.
#[cfg(any(test, feature = "test-support"))]
fn enter_probe(value: &std::sync::atomic::AtomicUsize, from: usize, entered: usize) -> bool {
    value
        .compare_exchange(
            from,
            entered,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
}

#[cfg(feature = "test-support")]
pub(super) async fn probe(
    value: Option<&std::sync::atomic::AtomicUsize>,
    from: usize,
    entered: usize,
    released: usize,
    until: Instant,
    cancellation: [&CancellationToken; 4],
) -> Result<()> {
    use std::sync::atomic::Ordering;
    if let Some(value) = value {
        if enter_probe(value, from, entered) {
            while value.load(Ordering::Acquire) == entered {
                if Instant::now() >= until || cancellation.iter().any(|t| t.is_cancelled()) {
                    return Err(Error::forbidden());
                }
                tokio::select! {biased;
                    _=cancellation[0].cancelled()=>return Err(Error::forbidden()),
                    _=cancellation[1].cancelled()=>return Err(Error::forbidden()),
                    _=cancellation[2].cancelled()=>return Err(Error::forbidden()),
                    _=cancellation[3].cancelled()=>return Err(Error::forbidden()),
                    _=tokio::time::sleep_until(until.min(Instant::now()+Duration::from_millis(1)))=>{},
                }
            }
            if value.load(Ordering::Acquire) != released {
                return Err(Error::forbidden());
            }
        }
    }
    Ok(())
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
    fn tokens() -> [CancellationToken; 4] {
        std::array::from_fn(|_| CancellationToken::new())
    }
    #[tokio::test]
    async fn busy_ready_metadata_drops_each_proof_before_fresh_admission_and_one_capture() {
        // Reproduce the exact stale-observation ordering without sleeps:
        // the companion publishes9/10 after our load of3/7 but BEFORE entry.
        // A plain store of entered would lose the original resume here.
        for (dropped, resume) in [(3, 9), (7, 10)] {
            let probe = AtomicUsize::new(dropped);
            let observed = probe.load(Ordering::Acquire);
            probe.store(resume, Ordering::Release);
            assert!(!enter_probe(&probe, observed, observed));
            assert_eq!(probe.load(Ordering::Acquire), resume);
            probe.store(dropped, Ordering::Release);
            assert!(enter_probe(&probe, dropped, dropped));
            probe.store(resume, Ordering::Release);
            assert!(!enter_probe(&probe, dropped, dropped));
            assert_eq!(probe.load(Ordering::Acquire), resume);
        }
        // Ordinary forward transitions still enter exactly once.
        for (from, entered, released) in [(0, 1, 2), (4, 5, 6)] {
            let probe = AtomicUsize::new(from);
            assert!(enter_probe(&probe, from, entered));
            assert_eq!(probe.load(Ordering::Acquire), entered);
            probe.store(released, Ordering::Release);
            assert!(!enter_probe(&probe, from, entered));
            assert_eq!(probe.load(Ordering::Acquire), released);
        }
        let t = tokens();
        let held = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let captures = AtomicUsize::new(0);
        let original = Arc::new(());
        let original_address = Arc::as_ptr(&original);
        let proof = capture(
            Instant::now() + Duration::from_secs(1),
            [&t[0], &t[1], &t[2], &t[3]],
            "ready-current-read",
            || async {
                assert_eq!(held.load(Ordering::SeqCst), 0);
                assert_eq!(Arc::as_ptr(&original), original_address);
                held.fetch_add(1, Ordering::SeqCst);
                let guard = Held(held.clone());
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    drop(guard);
                    Err(Error::new("authority-busy", "primary SQL held"))
                } else {
                    Ok(guard)
                }
            },
        )
        .await
        .unwrap();
        captures.fetch_add(1, Ordering::SeqCst);
        drop(proof);
        let result_calls = AtomicUsize::new(0);
        capture(
            Instant::now() + Duration::from_secs(1),
            [&t[0], &t[1], &t[2], &t[3]],
            "ready-result-read",
            || async {
                assert_eq!(held.load(Ordering::SeqCst), 0);
                assert_eq!(captures.load(Ordering::SeqCst), 1);
                if result_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::new("authority-busy", "primary SQL held"))
                } else {
                    Ok(())
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (
                calls.load(Ordering::SeqCst),
                result_calls.load(Ordering::SeqCst),
                captures.load(Ordering::SeqCst)
            ),
            (2, 2, 1)
        );
    }
    #[tokio::test]
    async fn permanent_storage_and_authority_file_refusals_do_not_retry_ready() {
        for code in [
            "forbidden",
            "storage-unavailable",
            "authority-unavailable",
            "conflict",
        ] {
            let t = tokens();
            let calls = AtomicUsize::new(0);
            let result = capture(
                Instant::now() + Duration::from_secs(1),
                [&t[0], &t[1], &t[2], &t[3]],
                "ready-current-read",
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(Error::new(code, "permanent control"))
                },
            )
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
        assert_eq!(
            authority_error(DispatchError::AuthorityBusy).code,
            "authority-busy"
        );
        assert_eq!(
            authority_error(DispatchError::AuthorityUnavailable).code,
            "authority-unavailable"
        );
        assert_eq!(
            authority_error(DispatchError::StorageUnavailable).code,
            "storage-unavailable"
        );
    }
    #[tokio::test]
    async fn each_original_cancellation_wins_before_ready_retry_or_capture() {
        for index in 0..4 {
            let t = tokens();
            let calls = AtomicUsize::new(0);
            let result = capture(
                Instant::now() + Duration::from_secs(1),
                [&t[0], &t[1], &t[2], &t[3]],
                "ready-current-read",
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    t[index].cancel();
                    Err::<(), _>(Error::new("authority-busy", "excluded"))
                },
            )
            .await;
            assert_eq!(result.unwrap_err().code, "forbidden");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn fresh_ready_authority_refuses_revocation_after_busy_without_reusing_previous_proof() {
        let t = tokens();
        let calls = AtomicUsize::new(0);
        let result = capture(
            Instant::now() + Duration::from_secs(1),
            [&t[0], &t[1], &t[2], &t[3]],
            "ready-current-read",
            || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err::<(), _>(Error::new("authority-busy", "original SQL excluded"))
                } else {
                    Err(Error::forbidden())
                }
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn original_ready_expiry_joins_and_drops_an_actual_late_positive_proof() {
        let t = tokens();
        let held = Arc::new(AtomicUsize::new(0));
        let result = capture(
            Instant::now() + Duration::from_millis(5),
            [&t[0], &t[1], &t[2], &t[3]],
            "ready-current-read",
            || async {
                held.fetch_add(1, Ordering::SeqCst);
                let guard = Held(held.clone());
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(guard)
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code, "forbidden");
        assert_eq!(held.load(Ordering::SeqCst), 0);
    }
}
