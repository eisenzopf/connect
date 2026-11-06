//! Retry only Core's typed, definitely unpublished VideoPlan exclusion.
use super::*;
use rvoip_core::staged_streams::device_replacement::{
    ReplacementVideoRequest, VideoReplacementPreparation, VideoReplacementPreparationAttempt,
};
use std::future::Future;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

enum Attempt<T> {
    MetadataBusy(tokio::time::Instant),
    Prepared(T),
    #[cfg(feature = "test-support")]
    ProbeBusy {
        deadline: tokio::time::Instant,
        probe: Arc<crate::canonical_media::video_preparation_fixture::Probe>,
    },
}

/// The projection is effect-free. Installation runs once per fresh capture;
/// only its exact unpublished outcome can ask for another capture. No role,
/// SQL/file reader or current grant survives the wait between attempts.
async fn prepare<C, P, F, Fut, I>(
    roles: &Mutex<()>,
    stop: &CancellationToken,
    cancel: &CancellationToken,
    mut until: tokio::time::Instant,
    mut current: F,
    mut install: I,
) -> Result<P>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(C, tokio::time::Instant)>>,
    I: FnMut(&C) -> Result<Attempt<P>>,
{
    loop {
        let read_stop = cancel.child_token();
        let observed = super::super::projection::read(roles, &read_stop, until, &mut current);
        tokio::pin!(observed);
        let (roles, (capture, deadline)) = tokio::select! { biased;
            _ = stop.cancelled() => {
                read_stop.cancel();
                let _ = observed.await;
                return Err(Error::forbidden());
            },
            _ = cancel.cancelled() => {
                read_stop.cancel();
                let _ = observed.await;
                return Err(Error::forbidden());
            },
            _ = tokio::time::sleep_until(until) => {
                read_stop.cancel();
                let _ = observed.await;
                return Err(Error::new("authority-busy", "Original video preparation cutoff elapsed"));
            },
            value = &mut observed => value?,
        };
        // Never extend an earlier captured authority cutoff on the next read.
        until = until.min(deadline);
        let outcome = if stop.is_cancelled() || cancel.is_cancelled() {
            Err(Error::forbidden())
        } else if tokio::time::Instant::now() >= until {
            Err(Error::new(
                "authority-busy",
                "Original video preparation cutoff elapsed",
            ))
        } else {
            install(&capture)
        };
        drop(capture);
        drop(roles);
        match outcome? {
            Attempt::Prepared(owner) => return Ok(owner),
            #[cfg(feature = "test-support")]
            Attempt::ProbeBusy { deadline, probe } => {
                until = until.min(deadline);
                probe.pause(stop, cancel, until).await?;
                // The next iteration takes a fresh role/Store capture. The
                // one-shot real lock injection was already consumed once.
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err(Error::forbidden()),
                    _ = cancel.cancelled() => return Err(Error::forbidden()),
                    _ = tokio::time::sleep_until(until) => return Err(Error::new("authority-busy", "Original video preparation cutoff elapsed")),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
            Attempt::MetadataBusy(deadline) => {
                until = until.min(deadline);
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err(Error::forbidden()),
                    _ = cancel.cancelled() => return Err(Error::forbidden()),
                    _ = tokio::time::sleep_until(until) => return Err(Error::new("authority-busy", "Original video preparation cutoff elapsed")),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
        }
    }
}

impl Runtime {
    pub(super) fn prepare_private_video<'a>(
        &'a self,
        root: &'a Arc<PreparedReplacement>,
        started: &'a Arc<StartedReplacement>,
        request: &'a ReplacementVideoRequest,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<VideoReplacementPreparation>> + Send + 'a>>
    {
        // Certify Send here rather than recursively embedding the complete
        // scene-reader future in the already-owned preparation task's type.
        Box::pin(async move {
            let until = root
                .deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(5));
            let expected_revision = started.controller.revision();
            let cancellation = root.handle.cancellation();
            prepare(
                &self.role_projection,
                &self.stop,
                &cancellation,
                until,
                || async {
                    let i = self
                        .continuation_read_once(
                            root,
                            started,
                            Phase::Preroll(request.clone(), expected_revision),
                        )
                        .await?;
                    let deadline = i.deadline;
                    Ok((i, deadline))
                },
                |i| {
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = self
                        .media
                        .take_video_preparation_probe_for_test(&root.origin.request().operation_id)
                    {
                        let (result, before, after) = self
                            .core
                            .with_staged_replacement_video_source_metadata_for_test(
                                &root.handle,
                                || {
                                    self.core.prepare_staged_device_replacement_video_with_held(
                                        &root.handle,
                                        request,
                                        i,
                                    )
                                },
                            )
                            .map_err(|_| Error::forbidden())?;
                        probe.record(
                            matches!(
                                &result,
                                Ok(VideoReplacementPreparationAttempt::MetadataBusy { .. })
                            ),
                            before,
                            after,
                        )?;
                        return match result {
                            Ok(VideoReplacementPreparationAttempt::MetadataBusy { deadline }) => {
                                Ok(Attempt::ProbeBusy { deadline, probe })
                            }
                            _ => Err(Error::forbidden()),
                        };
                    }
                    match self.core.prepare_staged_device_replacement_video_with_held(
                        &root.handle,
                        request,
                        i,
                    ) {
                        Ok(VideoReplacementPreparationAttempt::MetadataBusy { deadline }) => {
                            Ok(Attempt::MetadataBusy(deadline))
                        }
                        Ok(VideoReplacementPreparationAttempt::Prepared(owner)) => {
                            Ok(Attempt::Prepared(owner))
                        }
                        Err(error) => {
                            // This may follow actual route installation. Preserve its
                            // refusal and RAII cleanup; never turn it into a retry.
                            tracing::debug!(
                                ?error,
                                "Replacement private video preparation refused"
                            );
                            Err(Error::forbidden())
                        }
                    }
                },
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Capture(Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn unpublished_video_retry_releases_all_custody_and_rereads_before_one_effect() {
        let roles = Mutex::new(());
        let stop = CancellationToken::new();
        let cancel = CancellationToken::new();
        let live = Arc::new(AtomicUsize::new(0));
        let calls = AtomicUsize::new(0);
        let effects = AtomicUsize::new(0);
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(1);
        let work = prepare(
            &roles,
            &stop,
            &cancel,
            cutoff,
            || async {
                live.fetch_add(1, Ordering::SeqCst);
                Ok((Capture(live.clone()), cutoff))
            },
            |_| {
                assert!(roles.try_lock().is_err());
                assert_eq!(live.load(Ordering::SeqCst), 1);
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(Attempt::MetadataBusy(cutoff))
                } else {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(Attempt::Prepared(17))
                }
            },
        );
        let observer = async {
            tokio::task::yield_now().await;
            let _roles = tokio::time::timeout(Duration::from_millis(10), roles.lock())
                .await
                .unwrap();
            assert_eq!(live.load(Ordering::SeqCst), 0);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(effects.load(Ordering::SeqCst), 0);
        };
        let (result, ()) = tokio::join!(work, observer);
        assert_eq!(result.unwrap(), 17);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unpublished_video_retry_observes_mutation_cancel_and_original_cutoff() {
        for mode in ["mutation", "handle-cancel", "stop", "cutoff"] {
            let roles = Mutex::new(());
            let stop = CancellationToken::new();
            let cancel = CancellationToken::new();
            let revision = AtomicUsize::new(11);
            let calls = AtomicUsize::new(0);
            let effects = AtomicUsize::new(0);
            let cutoff = tokio::time::Instant::now()
                + Duration::from_millis(if mode == "cutoff" { 35 } else { 250 });
            let expected = revision.load(Ordering::SeqCst);
            let work = prepare(
                &roles,
                &stop,
                &cancel,
                cutoff,
                || async {
                    if revision.load(Ordering::SeqCst) != expected {
                        return Err(Error::forbidden());
                    }
                    Ok(((), cutoff))
                },
                |_| {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 || mode == "cutoff" {
                        Ok(Attempt::MetadataBusy(cutoff))
                    } else {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(Attempt::Prepared(()))
                    }
                },
            );
            let mutation = async {
                tokio::task::yield_now().await;
                let _roles = roles.lock().await;
                match mode {
                    "mutation" => {
                        revision.fetch_add(1, Ordering::SeqCst);
                    }
                    "handle-cancel" => cancel.cancel(),
                    "stop" => stop.cancel(),
                    _ => {}
                }
            };
            let (result, ()) = tokio::join!(work, mutation);
            assert_eq!(
                result.err().unwrap().code,
                if mode == "cutoff" {
                    "authority-busy"
                } else {
                    "forbidden"
                }
            );
            assert_eq!(effects.load(Ordering::SeqCst), 0);
            assert!(roles.try_lock().is_ok());
            assert!(tokio::time::Instant::now() < cutoff + Duration::from_millis(100));
            if mode != "cutoff" {
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[tokio::test]
    async fn first_busy_authority_cutoff_is_not_renewed_by_later_captures() {
        let roles = Mutex::new(());
        let calls = AtomicUsize::new(0);
        let started = tokio::time::Instant::now();
        let original = started + Duration::from_millis(35);
        let outer = started + Duration::from_millis(250);
        let result = prepare(
            &roles,
            &CancellationToken::new(),
            &CancellationToken::new(),
            outer,
            || async { Ok(((), outer)) },
            |_| {
                let count = calls.fetch_add(1, Ordering::SeqCst);
                let deadline = if count == 0 {
                    original
                } else {
                    tokio::time::Instant::now() + Duration::from_millis(35)
                };
                Ok::<Attempt<()>, _>(Attempt::MetadataBusy(deadline))
            },
        )
        .await;
        assert_eq!(result.err().unwrap().code, "authority-busy");
        assert!(tokio::time::Instant::now() < original + Duration::from_millis(100));
        assert!(roles.try_lock().is_ok());
    }

    #[tokio::test]
    async fn cancellation_joins_the_original_metadata_read_before_returning() {
        use std::sync::atomic::AtomicBool;
        let roles = Mutex::new(());
        let stop = CancellationToken::new();
        let cancel = CancellationToken::new();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let done = AtomicBool::new(false);
        let captures = Arc::new(AtomicUsize::new(0));
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(1);
        let work = async {
            let result = prepare(
                &roles,
                &stop,
                &cancel,
                cutoff,
                || async {
                    entered.notify_one();
                    release.notified().await;
                    captures.fetch_add(1, Ordering::SeqCst);
                    Ok((Capture(captures.clone()), cutoff))
                },
                |_| -> Result<Attempt<()>> { panic!("cancelled metadata installed a route") },
            )
            .await;
            done.store(true, Ordering::SeqCst);
            result
        };
        let observer = async {
            entered.notified().await;
            cancel.cancel();
            tokio::task::yield_now().await;
            assert!(
                !done.load(Ordering::SeqCst),
                "original metadata task was abandoned"
            );
            assert!(
                roles.try_lock().is_err(),
                "original current read no longer owns its coordinator"
            );
            release.notify_one();
        };
        let (result, ()) = tokio::join!(work, observer);
        assert_eq!(result.err().unwrap().code, "forbidden");
        assert_eq!(captures.load(Ordering::SeqCst), 0);
        assert!(roles.try_lock().is_ok());
    }

    #[tokio::test]
    async fn uncertain_or_post_registration_errors_never_resubmit_a_video_effect() {
        let roles = Mutex::new(());
        let calls = AtomicUsize::new(0);
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(1);
        let result = prepare(
            &roles,
            &CancellationToken::new(),
            &CancellationToken::new(),
            cutoff,
            || async { Ok(((), cutoff)) },
            |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<Attempt<()>, _>(Error::new(
                    "authority-busy",
                    "A post-registration failure is not an unpublished outcome",
                ))
            },
        )
        .await;
        assert_eq!(result.err().unwrap().code, "authority-busy");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(roles.try_lock().is_ok());
    }
}
