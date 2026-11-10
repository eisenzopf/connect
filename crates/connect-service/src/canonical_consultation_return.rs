//! Restore only the actual original Core audience handle under fresh return authority.
use crate::{
    canonical_consultation::{descriptor, retry_metadata, store_call},
    canonical_media::consultation::{AudienceInvocation, AudienceRead},
    canonical_runtime::Runtime,
};
use connect_protocol::{transitions::TransitionCommand, Error, Result};
use rvoip_core::ids::SessionId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};
struct Job {
    runtime: std::sync::Weak<Runtime>,
    record: String,
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime
                .consultation_jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.record);
        }
    }
}
impl Runtime {
    pub(crate) fn queue_consultation_return(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) {
        let Ok(mut queued) = self.consultation_jobs.lock() else {
            return;
        };
        if queued.len() >= self.media.store.limits().connections || !queued.insert(record.clone()) {
            return;
        }
        let Ok(mut work) = self.work.lock() else {
            queued.remove(&record);
            return;
        };
        let runtime = self.clone();
        let job = Job {
            runtime: Arc::downgrade(self),
            record: record.clone(),
        };
        work.spawn(async move {let _job=job;if let Err(error)=runtime.apply_consultation_return(credential,command,record).await{tracing::debug!(target:"connect_service::canonical_consultation",code=%error.code,"Native return did not complete");}});
    }
    async fn begin_original_return(
        self: &Arc<Self>,
        lease: Arc<connect_store::NativeConsultationReturnLease>,
    ) -> Result<(
        connect_store::NativeConsultationReturnAttempt,
        connect_store::NativeReturnRead,
    )> {
        // One original metadata-admission window. Never renew it on a fresh
        // read or after a file/SQL deferral; the first successful read narrows it.
        let until = Arc::new(AtomicI64::new(
            lease.expires_at().min(chrono::Utc::now().timestamp() + 5),
        ));
        let cutoff = tokio::time::Instant::now() + Duration::from_secs(5);
        let admission_wait = connect_store::AuthorityWait::new(Duration::from_secs(5));
        let _cancel_admission = admission_wait.cancel_on_drop();
        loop {
            if self.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            if tokio::time::Instant::now() >= cutoff
                || chrono::Utc::now().timestamp() >= until.load(Ordering::Acquire)
            {
                return Err(Error::new(
                    "authority-busy",
                    "Original native return begin admission ended",
                ));
            }
            let store = self.media.store.clone();
            let original = lease.clone();
            let original_until = until.clone();
            let stop = self.stop.clone();
            let wait = admission_wait.clone();
            // Actual blocking owner is retained and joined. No metadata SQL,
            // validation or five-ms file drain runs on the async media thread.
            let pending = self.owned_store_call(move || {
                if stop.is_cancelled() || tokio::time::Instant::now() >= cutoff {
                    return Err(Error::forbidden());
                }
                let mut captured_until = original_until.load(Ordering::Acquire);
                let resolved = store.native_consultation_return_attempt_read_with_wait(
                    &original, &mut captured_until, &wait);
                // A failed later admission may still have captured an earlier
                // original expiry. Preserve it across every pre-effect retry.
                original_until.fetch_min(captured_until, Ordering::AcqRel);
                let read = match resolved {
                    Ok(read) => read,
                    Err(error) if error.code == "authority-busy" => {
                        tracing::debug!(target: "connect_service::canonical_consultation", stage="return-current", code=%error.code,
                            "Original return current read deferred before begin");
                        return Ok(None);
                    }
                    Err(error) => return Err(error),
                };
                original_until.fetch_min(read.expires_at(), Ordering::AcqRel);
                if stop.is_cancelled() || tokio::time::Instant::now() >= cutoff {
                    return Err(Error::forbidden());
                }
                // Only Store's typed pre-DML None may loop. SQL/policy/commit
                // errors after entry return immediately, never replaying begin.
                store.try_begin_native_consultation_return_with_wait(
                    original, read, original_until.load(Ordering::Acquire), &wait)
            });
            tokio::pin!(pending);
            let result = tokio::select! { biased;
                result = &mut pending => result,
                _ = self.stop.cancelled() => {
                    admission_wait.cancel();
                    pending.await
                }
                _ = tokio::time::sleep_until(cutoff) => {
                    admission_wait.cancel();
                    pending.await
                }
            }?;
            if let Some(started) = result {
                // Even if stop raced the joined result, retain this exact
                // attempt: the existing Core/capture path owns its uncertainty.
                return Ok(started);
            }
            tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                _ = tokio::time::sleep_until(cutoff) => return Err(Error::new("authority-busy", "Original native return begin admission ended")),
                _ = tokio::time::sleep(Duration::from_millis(20)) => {},
            }
        }
    }
    async fn apply_consultation_return(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) -> Result<()> {
        let saved = self
            .consultation_returns
            .lock()
            .map_err(|_| Error::storage())?
            .get(&record)
            .cloned();
        if let Some(receipt) = saved {
            return self.settle_consultation_return(receipt).await;
        }
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let credential = credential.clone();
            let command = command.clone();
            store_call(move || store.native_consultation_return_origin(&credential, command))
        })
        .await.map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-origin",code=%error.code,"Native return refused");error})?;
        let (session, handle, start_receipt, original_receivers) = {
            let audiences = self
                .consultation_audiences
                .lock()
                .map_err(|_| Error::storage())?;
            let a = audiences
                .get(origin.consultation_record_id())
                .ok_or_else(Error::forbidden)?;
            (
                a.session.clone(),
                a.handle.clone(),
                a.receipt.clone(),
                a.original_receivers.clone(),
            )
        };
        if session != command.session_id
            || handle.audience().operation_id() != origin.consultation_record_id()
        {
            return Err(Error::forbidden());
        }
        tracing::debug!(target:"connect_service::canonical_consultation",stage="return-handle","Native return original handle matched");
        // A retained receipt only records a previous actual Core result. It cannot execute.
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = start_receipt.clone();
            store_call(move || store.record_native_consultation_applied(&receipt))
        })
        .await.map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-original-observation",code=%error.code,"Native return refused");error})?;
        let sid = SessionId::from_string(session);
        // A refused metadata reservation consumes its planning readers. Retry
        // with current sources and a fresh pure Core plan, never an old lease.
        // Attempt admission and the actual Core change remain outside this loop.
        let planning_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let planning_until = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp() + 5));
        let planning_wait = connect_store::AuthorityWait::new(Duration::from_secs(5));
        let _cancel_planning = planning_wait.cancel_on_drop();
        let (lease, plan) = loop {
            if self.stop.is_cancelled()
                || tokio::time::Instant::now() >= planning_deadline
                || chrono::Utc::now().timestamp() >= planning_until.load(Ordering::Acquire)
            {
                return Err(Error::new(
                    "authority-busy",
                    "Original return planning admission ended",
                ));
            }
            let sources = self
                .media
                .scene_origins(&command.session_id)
                .map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-scene",?error,"Native return refused");Error::forbidden()})?;
            let sips = self
                .media
                .scene_sip_origins(&command.session_id)
                .map_err(|_| Error::forbidden())?;
            let store = self.media.store.clone();
            let read_origin = origin.clone();
            let read_sources = sources.clone();
            let read_sips = sips.clone();
            let captured_cap = planning_until.clone();
            let current_wait = planning_wait.clone();
            let stop = self.stop.clone();
            let resolved =
                joined_return_control(self, &planning_wait, planning_deadline, move || {
                    if stop.is_cancelled() {
                        return Err(Error::forbidden());
                    }
                    let mut cap = captured_cap.load(Ordering::Acquire);
                    let result = store.native_return_planning_read_with_wait(
                        &read_origin,
                        &read_sources,
                        &read_sips,
                        &mut cap,
                        &current_wait,
                    );
                    captured_cap.fetch_min(cap, Ordering::AcqRel);
                    result
                })
                .await;
            let read = match resolved {
                Ok(read) => read,
                Err(error) if error.code == "authority-busy" => {
                    tracing::debug!(target:"connect_service::canonical_consultation", stage="return-read", code=%error.code,
                        "Original return planning admission deferred");
                    return_planning_pause(&self.stop, planning_deadline, &planning_until).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            planning_until.fetch_min(read.expires_at(), Ordering::AcqRel);
            if self.stop.is_cancelled() || tokio::time::Instant::now() >= planning_deadline {
                return Err(Error::forbidden());
            }
            let revision = self
                .core
                .audio_audience_revision(&sid)
                .map_err(|error| {tracing::debug!(target:"connect_service::canonical_consultation",stage="return-revision",?error,"Native return planning refused");Error::forbidden()})?;
            let plan = self
                .core
                .plan_restore_staged_audio_audience(
                    &sid,
                    handle.audience(),
                    origin.record_id().into(),
                    revision,
                )
                .map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-plan",?error,"Native return refused");Error::forbidden()})?;
            tracing::debug!(target:"connect_service::canonical_consultation",stage="return-planned",current=read.current(),source_count=sources.len(),receiver_count=plan.expected_receiver_revisions().len(),"Native return planning snapshot");
            // EOF can remove a receiver from Core before its original Store
            // lifetime/registry retirement is observed. This is still pure
            // planning: release every reader and retry fresh metadata rather
            // than reserve a plan against different source incarnations.
            // Store's exact selection/revision checks remain mandatory below.
            let current = sources
                .iter()
                .map(|source| source.runtime_connection_id())
                .chain(sips.iter().map(|source| source.runtime_connection_id()))
                .collect::<BTreeSet<_>>();
            let planned = plan
                .expected_receiver_revisions()
                .keys()
                .map(|id| id.as_str())
                .collect::<BTreeSet<_>>();
            if current != planned {
                tracing::debug!(target:"connect_service::canonical_consultation",stage="return-scene-pending",code="authority-busy",source_count=current.len(),receiver_count=planned.len(),"Native return planning observations differ");
                drop(read);
                return_planning_pause(&self.stop, planning_deadline, &planning_until).await?;
                continue;
            }
            let mut intent = descriptor(&plan)?;
            intent["consultation_record_id"] = serde_json::json!(origin.consultation_record_id());
            intent["restore"] = serde_json::json!(true);
            intent["known_departed_connections"] = serde_json::json!(original_receivers
                .iter()
                .filter(|id| !plan
                    .expected_receiver_revisions()
                    .keys()
                    .any(|current| current.as_str() == id.as_str()))
                .collect::<Vec<_>>());
            let store = self.media.store.clone();
            let original_until = planning_until.load(Ordering::Acquire);
            let reserve_wait = planning_wait.clone();
            let stop = self.stop.clone();
            let reservation =
                joined_return_control(self, &planning_wait, planning_deadline, move || {
                    if stop.is_cancelled() || tokio::time::Instant::now() >= planning_deadline {
                        return Err(Error::forbidden());
                    }
                    store.prepare_native_consultation_return_with_wait(
                        read,
                        intent,
                        original_until,
                        &reserve_wait,
                    )
                })
                .await?;
            match reservation {
                connect_store::NativeConsultationReturnPreparation::Deferred(error) => {
                    tracing::debug!(target:"connect_service::canonical_consultation",stage="return-prepare",code=%error.code,
                        "Original return reservation deferred before DML");
                    return_planning_pause(&self.stop, planning_deadline, &planning_until).await?;
                }
                connect_store::NativeConsultationReturnPreparation::NoWork => return Ok(()),
                // Even after a cancellation race this SAME durable reservation
                // leaves planning exactly once. Begin owns its current refusal.
                connect_store::NativeConsultationReturnPreparation::Reserved(lease) => {
                    break (lease, plan)
                }
            }
        };
        let (attempt, read) = self.begin_original_return(Arc::new(lease)).await?;
        let remaining = (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let invocation = AudienceInvocation {
            policy: self.media.clone(),
            plan: plan.clone(),
            read: AudienceRead::Return(Arc::new(read)),
            cancel: self.stop.child_token(),
        };
        let actual = self
            .core
            .change_staged_audio_audience_with_held(
                &sid,
                plan,
                tokio::time::Instant::now() + Duration::from_secs(remaining),
                invocation.cancel.clone(),
                &invocation,
            )
            .await;
        let (outcome, handle) = match actual {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_consultation",stage="return-core",?error,"Actual native return did not complete");
                drop(invocation);
                let attempt = Arc::new(attempt);
                settle_return_recording(&self.stop, || {
                    let store = self.media.store.clone();
                    let attempt = attempt.clone();
                    self.owned_store_call(move || {
                        store.native_consultation_return_uncertain(&attempt)
                    })
                })
                .await?;
                return Err(Error::new(
                    "reconciliation-required",
                    "Native restoration attempt is uncertain",
                ));
            }
        };
        let observation = serde_json::json!({"operation_id":outcome.operation_id,"audience_revision":outcome.audience_revision,"receiver_revisions":outcome.receiver_revisions.iter().map(|(k,v)|(k.as_str().to_owned(),*v)).collect::<BTreeMap<_,_>>(),"restored":outcome.restored,"departed_connections":outcome.departed_connections.iter().map(|c|c.as_str()).collect::<Vec<_>>(),"participant_handoff":null,"participant_replacement":null});
        let receipt = Arc::new(attempt.applied(
            observation,
            crate::canonical_runtime::capture_audience_metadata(handle.commit())?,
        )?);
        self.consultation_returns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(origin.record_id().into(), receipt.clone());
        let carry_failed = !self.carry_audio_audience_commit(handle.commit(), &invocation);
        drop(invocation);
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = receipt.clone();
            self.owned_store_call(move || store.record_native_consultation_return_applied(&receipt))
        })
        .await?;
        if carry_failed {
            return Err(Error::new(
                "native-confirmation-lost",
                "Native return confirmation could not follow the actual audience commit",
            ));
        }
        self.settle_consultation_return(receipt).await
    }
    async fn settle_consultation_return(
        &self,
        receipt: Arc<connect_store::NativeConsultationReturnApplied>,
    ) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = receipt.clone();
            self.owned_store_call(move || store.record_native_consultation_return_applied(&receipt))
        })
        .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let store = self.media.store.clone();
            let proof = receipt.clone();
            match self
                .owned_store_call(move || store.complete_native_consultation_return(&proof))
                .await
            {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {}
                // Same retained historical receipt, never another Core restore.
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable" | "capacity-exceeded"
                    ) => {}
                Err(error) => return Err(error),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::new(
                    "reconciliation-required",
                    "Native private retirement is not yet observed",
                ));
            }
            tokio::select! {_=self.stop.cancelled()=>return Err(Error::forbidden()),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
        }
    }
}

// Both current planning and reservation keep one original wait/cancellation
// owner. Join the SAME callback on stop/expiry; never detach or discard a late
// Reserved result that already owns durable metadata custody.
async fn joined_return_control<T: Send + 'static>(
    runtime: &Runtime,
    wait: &connect_store::AuthorityWait,
    deadline: tokio::time::Instant,
    call: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let pending = runtime.owned_store_call(call);
    tokio::pin!(pending);
    tokio::select! { biased;
        result = &mut pending => result,
        _ = runtime.stop.cancelled() => { wait.cancel(); pending.await },
        _ = tokio::time::sleep_until(deadline) => { wait.cancel(); pending.await },
    }
}
async fn return_planning_pause(
    stop: &tokio_util::sync::CancellationToken,
    deadline: tokio::time::Instant,
    until: &AtomicI64,
) -> Result<()> {
    if chrono::Utc::now().timestamp() >= until.load(Ordering::Acquire) {
        return Err(Error::new(
            "authority-busy",
            "Original return planning facts expired",
        ));
    }
    tokio::select! { biased;
        _ = stop.cancelled() => Err(Error::forbidden()),
        _ = tokio::time::sleep_until(deadline) => Err(Error::new("authority-busy", "Original return planning admission ended")),
        _ = tokio::time::sleep(Duration::from_millis(20)) => Ok(()),
    }
}

// Only historical metadata retries. The original worker owns the SAME attempt
// after all cut/IO aliases leave, and never invokes another physical restore.
async fn settle_return_recording<T, F, Fut>(
    stop: &tokio_util::sync::CancellationToken,
    mut record: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        match record().await {
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable" | "capacity-exceeded"
                ) =>
            {
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err(error),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            }
            result => return result,
        }
    }
}
#[cfg(test)]
mod return_recording_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn original_return_capture_stays_owned_through_temporary_recording_pressure() {
        struct Original(Arc<AtomicUsize>);
        impl Drop for Original {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let capture = Arc::new(Original(drops.clone()));
        let original = capture.clone();
        let calls = AtomicUsize::new(0);
        settle_return_recording(&tokio_util::sync::CancellationToken::new(), || {
            assert!(Arc::ptr_eq(&capture, &original));
            let n = calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(match n {
                0 => Err(Error::new("authority-busy", "Original reader drains")),
                1 => Err(Error::storage()),
                2 => Err(Error::new("capacity-exceeded", "Original journal full")),
                _ => Ok(()),
            })
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(original);
        drop(capture);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn return_recording_stops_on_runtime_stop_or_permanent_refusal() {
        for code in [
            "authority-busy",
            "storage-unavailable",
            "capacity-exceeded",
            "forbidden",
        ] {
            let stop = tokio_util::sync::CancellationToken::new();
            let calls = AtomicUsize::new(0);
            let result: Result<()> = settle_return_recording(&stop, || {
                calls.fetch_add(1, Ordering::SeqCst);
                stop.cancel();
                std::future::ready(Err(Error::new(code, "Original outcome")))
            })
            .await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}
