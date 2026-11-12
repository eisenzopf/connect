//! Only an explicit no-attempt result may retry before one physical Commit.
use connect_protocol::{Error, Result};
use std::{future::Future, pin::Pin, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) type Attempt<'a, T> = Pin<Box<dyn Future<Output = Result<Option<T>>> + Send + 'a>>;

pub(super) async fn admit<'a, T, F>(
    until: Instant,
    cancellation: [&CancellationToken; 2],
    mut current: F,
) -> Result<T>
where
    F: FnMut(usize) -> Attempt<'a, T>,
{
    let mut attempt = 0;
    loop {
        if cancellation.iter().any(|c| c.is_cancelled()) {
            return Err(Error::forbidden());
        }
        if Instant::now() >= until {
            return Err(Error::new(
                "authority-busy",
                "Original Commit admission cutoff reached",
            ));
        }
        attempt += 1;
        // Always join the actual owner. A Some may contain a durable attempt:
        // return it EVEN if time/cancellation advanced, so the caller owns its
        // exact NotSubmitted capture instead of losing attribution/retrying.
        match current(attempt).await? {
            Some(value) => return Ok(value),
            None => {}
        }
        // None contains no attempt or grant; its producer has dropped every
        // original Writer/role/owner alias before this grant-free pause.
        tokio::select! { biased;
            _=cancellation[0].cancelled()=>return Err(Error::forbidden()),
            _=cancellation[1].cancelled()=>return Err(Error::forbidden()),
            _=tokio::time::sleep_until(until)=>return Err(Error::new("authority-busy","Original Commit admission cutoff reached")),
            _=tokio::time::sleep_until(until.min(Instant::now()+Duration::from_millis(20)))=>{},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_store::{AuthorityMode, AuthorityScope, Store};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn definite_no_attempt_drops_real_writer_and_role_before_same_root_retry() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&directory.path().join("state.sqlite3"), Default::default()).unwrap();
        let roles = tokio::sync::Mutex::new(());
        let stop = CancellationToken::new();
        let root_cancel = CancellationToken::new();
        let original = Arc::new(());
        let original_address = Arc::as_ptr(&original) as usize;
        let calls = AtomicUsize::new(0);
        let core_calls = AtomicUsize::new(0);
        let scope = AuthorityScope::identity("original-root-member", AuthorityMode::Write).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        let work = admit(until, [&stop, &root_cancel], |attempt| {
            let original = &original;
            let calls = &calls;
            let core_calls = &core_calls;
            let roles = &roles;
            let store = &store;
            let scope = &scope;
            let future: Attempt<'_, _> = Box::pin(async move {
                assert_eq!(Arc::as_ptr(original) as usize, original_address);
                calls.fetch_add(1, Ordering::SeqCst);
                let role = roles.lock().await;
                let writer = store.try_acquire_authority(std::slice::from_ref(scope))?;
                assert_eq!(core_calls.load(Ordering::SeqCst), 0);
                if attempt == 1 {
                    drop(writer);
                    drop(role);
                    Ok(None)
                } else {
                    Ok(Some((role, writer)))
                }
            });
            future
        });
        let observer = async {
            // join! polls work through its None/backoff before this branch;
            // acquire immediately, without a scheduling race against retry.
            let role = tokio::time::timeout_at(until, roles.lock()).await.unwrap();
            let reader = store
                .try_acquire_authority(&[AuthorityScope::identity(
                    "original-root-member",
                    AuthorityMode::Read,
                )
                .unwrap()])
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            drop(reader);
            drop(role);
        };
        let (result, ()) = tokio::join!(work, observer);
        let (role, writer) = result.unwrap();
        core_calls.fetch_add(1, Ordering::SeqCst); // Physical effect lives OUTSIDE admission.
        drop(writer);
        drop(role);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(core_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_even_named_busy_never_retry_an_existing_or_uncertain_attempt() {
        for code in [
            "authority-busy",
            "storage-unavailable",
            "forbidden",
            "conflict",
            "reconciliation-required",
        ] {
            let stop = CancellationToken::new();
            let root_cancel = CancellationToken::new();
            let calls = AtomicUsize::new(0);
            let result = admit::<(), _>(
                Instant::now() + Duration::from_secs(1),
                [&stop, &root_cancel],
                |_| {
                    Box::pin(async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Err(Error::new(code, "not an explicit no-attempt result"))
                    })
                },
            )
            .await;
            assert_eq!(result.err().unwrap().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
        // A definite no-attempt result grants no work after root cancellation.
        let stop = CancellationToken::new();
        let root_cancel = CancellationToken::new();
        let calls = AtomicUsize::new(0);
        let effects = AtomicUsize::new(0);
        let until = Instant::now() + Duration::from_secs(1);
        let result = admit::<(), _>(until, [&stop, &root_cancel], |_| {
            Box::pin(async {
                calls.fetch_add(1, Ordering::SeqCst);
                root_cancel.cancel();
                Ok(None)
            })
        })
        .await;
        if result.is_ok() {
            effects.fetch_add(1, Ordering::SeqCst);
        }
        assert_eq!(result.err().unwrap().code, "forbidden");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        // Repeated None results share ONE original cutoff. The outer bound is
        // only a test watchdog; the helper must terminate at the earlier until.
        let stop = CancellationToken::new();
        let root_cancel = CancellationToken::new();
        let calls = AtomicUsize::new(0);
        let until = Instant::now() + Duration::from_millis(40);
        let result = tokio::time::timeout_at(
            until + Duration::from_millis(200),
            admit::<(), _>(until, [&stop, &root_cancel], |_| {
                Box::pin(async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(None)
                })
            }),
        )
        .await
        .expect("None must not renew the original admission cutoff");
        if result.is_ok() {
            effects.fetch_add(1, Ordering::SeqCst);
        }
        assert_eq!(result.err().unwrap().code, "authority-busy");
        assert!(Instant::now() >= until);
        let at_expiry = calls.load(Ordering::SeqCst);
        assert!(at_expiry >= 1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            at_expiry,
            "no factory survives the joined expired admission"
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    struct Held(Arc<AtomicUsize>);
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    #[tokio::test]
    async fn late_success_is_joined_and_retained_for_exact_not_submitted_custody() {
        let stop = CancellationToken::new();
        let root_cancel = CancellationToken::new();
        let held = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let until = Instant::now() + Duration::from_millis(5);
        let value = admit(until, [&stop, &root_cancel], |_| {
            Box::pin(async {
                calls.fetch_add(1, Ordering::SeqCst);
                held.fetch_add(1, Ordering::SeqCst);
                let value = Held(held.clone());
                tokio::time::sleep(Duration::from_millis(20)).await;
                stop.cancel();
                Ok(Some(value))
            })
        })
        .await
        .unwrap();
        assert!(Instant::now() >= until && stop.is_cancelled());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            held.load(Ordering::SeqCst),
            1,
            "late opaque attempt must return to its historical owner"
        );
        drop(value); // Caller records NotSubmitted; no physical effect is authorized here.
        assert_eq!(held.load(Ordering::SeqCst), 0);
    }
}
