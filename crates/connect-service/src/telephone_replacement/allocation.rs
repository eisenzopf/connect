//! Actual dormant reservation, pre-recorded Call-ID and once-only SIP submission.
use super::*;
use crate::{
    canonical_consultation::{retry_metadata, store_call},
    canonical_replacement::{observed, PreparedReplacement},
};
use connect_store::{
    TelephoneReplacementAllocated, TelephoneReplacementAttempt, TelephoneReplacementSubmitted,
    TelephoneReplacementUncertain,
};
use rvoip_sip::{SipDeviceReplacementContext, SipDeviceReplacementSignal};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Mutex, OnceLock,
};

fn allocation_stage<T>(stage: &'static str, result: Result<T>) -> Result<T> {
    if let Err(error) = &result {
        let code = match error.code.as_str() {
            "authority-busy"
            | "storage-unavailable"
            | "forbidden"
            | "conflict"
            | "invalid-request"
            | "unsupported-media-profile"
            | "not-ready"
            | "capacity-exceeded"
            | "busy" => error.code.as_str(),
            _ => "other",
        };
        tracing::debug!(target: "connect_service::telephone_replacement::allocation",
            stage, code, "Original telephone allocation stage refused");
    }
    result
}

pub(crate) struct Allocated {
    _packet_observers: connect_store::PacketObserverScope,
    pub context: Arc<SipDeviceReplacementContext>,
    pub receipt: Arc<TelephoneReplacementAllocated>,
    pub route: Arc<TelephoneReplacementRoute>,
    pub target: Arc<authority::Target>,
    pub writer: Arc<write::InviteAuthority>,
    pub signals: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<SipDeviceReplacementSignal>>,
    pub submission: OnceLock<Arc<TelephoneReplacementAttempt>>,
    pub submitted: Mutex<Option<Arc<TelephoneReplacementSubmitted>>>,
    pub uncertain: Mutex<Option<Arc<TelephoneReplacementUncertain>>>,
    pub resources: Mutex<Option<Arc<super::media::Started>>>,
    pub resource_failure: Mutex<Option<Arc<connect_store::TelephoneReplacementResourcesCapture>>>,
}
impl Runtime {
    pub(crate) async fn allocate_telephone_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        route: Arc<TelephoneReplacementRoute>,
    ) -> Result<Arc<Allocated>> {
        // The previous preparation invocation has released its file interval.
        // Acquire resource custody before role/leg registries or current reads;
        // this actual SIP target must survive its original native source ending.
        let store = self.media.store.clone();
        let observer_stop = root.handle.cancellation();
        let observer_deadline = root.deadline.into_std();
        let packet_observers = allocation_stage(
            "packet-observer-custody",
            store_call(move || {
                store.retain_packet_observers_until(observer_deadline, || {
                    observer_stop.is_cancelled()
                })
            })
            .await,
        )?;
        let _roles = self.role_projection.lock().await;
        let previous = { root.telephone.lock().map_err(|_| Error::storage())?.clone() };
        if let Some(previous) = previous {
            allocation_stage(
                "replay-record",
                self.record_telephone_allocation(&previous).await,
            )?;
            return Ok(previous);
        }
        let (adapter, configured) = allocation_stage(
            "configured-route",
            self.telephone_route(&route.policy.route_reference),
        )?;
        if !Arc::ptr_eq(&route, &configured) {
            return allocation_stage("configured-route-identity", Err(Error::forbidden()));
        }
        // These are synchronous primary-SQL lookups. Keep them on the existing
        // owned blocking-call path so the async owner of a contending SQL guard
        // can progress. No allocation/effect/Core/SIP action belongs in retry.
        // First actual source expiry remains the cap across metadata retries.
        let original_cutoff = Arc::new(AtomicI64::new(i64::MAX));
        let resolution_deadline = root
            .deadline
            .min(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
        let resolved = retry_metadata(&self.stop, || {
            let runtime = self.clone();
            let prepared = root.receipt.clone();
            let route = route.clone();
            let original_cutoff = original_cutoff.clone();
            let wait = connect_store::AuthorityWait::new(
                resolution_deadline.saturating_duration_since(tokio::time::Instant::now()),
            );
            let root_stop = root.handle.cancellation();
            let runtime_stop = self.stop.clone();
            async move {
                let _cancel_wait = wait.cancel_on_drop();
                let owned_wait = wait.clone();
                let worker_root_stop = root_stop.clone();
                let owner = runtime.clone();
                let work = owner.owned_store_call(move || {
                    if chrono::Utc::now().timestamp() >= original_cutoff.load(Ordering::Acquire) {
                        return allocation_stage(
                            "original-resolution-cutoff",
                            Err(Error::forbidden()),
                        );
                    }
                    let mut first_until = original_cutoff.load(Ordering::Acquire);
                    let result = runtime
                        .media
                        .store
                        .telephone_replacement_allocation_read_with_wait(
                            &prepared,
                            &route.credential_id,
                            &route.policy,
                            &mut first_until,
                            &owned_wait,
                        );
                    // Even a later file/SQL refusal cannot renew facts captured
                    // by the original first resolution attempt.
                    original_cutoff.fetch_min(first_until, Ordering::AcqRel);
                    let mut read = allocation_stage("planning-control-read", result)?;
                    original_cutoff.fetch_min(read.expires_at(), Ordering::AcqRel);
                    read.limit(original_cutoff.load(Ordering::Acquire))?;
                    let original = allocation_stage(
                        "source-owner",
                        runtime
                            .media
                            .replacement_source_owner(read.source())
                            .map_err(|_| Error::forbidden()),
                    )?;
                    let target = allocation_stage(
                        "route-target",
                        authority::allocated_target(&runtime, &read, &route, &original.member),
                    )?;
                    // A late synchronous lookup may join after expiry/cancellation;
                    // it can never authorize a dormant resource or a later begin.
                    if runtime.stop.is_cancelled()
                        || worker_root_stop.is_cancelled()
                        || tokio::time::Instant::now() >= resolution_deadline
                        || !read.current()
                    {
                        return allocation_stage("resolved-current", Err(Error::forbidden()));
                    }
                    Ok((read, Arc::new(target)))
                });
                tokio::pin!(work);
                let result = tokio::select! { biased;
                    _ = runtime_stop.cancelled() => None,
                    _ = root_stop.cancelled() => None,
                    _ = tokio::time::sleep_until(resolution_deadline) => None,
                    result = &mut work => Some(result),
                };
                match result {
                    Some(result) => result,
                    None => {
                        wait.cancel();
                        // Pure resolution has made no attempt or resource.
                        // Join and drop any late Read from this same owner.
                        drop(work.await);
                        Err(Error::forbidden())
                    }
                }
            }
        })
        .await;
        let (read, target) = allocation_stage("resolution", resolved)?;
        if self.stop.is_cancelled()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
            || !read.current()
        {
            return allocation_stage("joined-current", Err(Error::forbidden()));
        }
        let writer = Arc::new(write::InviteAuthority {
            runtime: Arc::downgrade(self),
            attempt: OnceLock::new(),
            established: OnceLock::new(),
            initial_from: OnceLock::new(),
            cancelled: root.handle.cancellation(),
            deadline: root.deadline,
        });
        let context = allocation_stage(
            "resource-context",
            adapter
                .device_replacement_context(
                    &self.core,
                    &root.handle,
                    target.principal.clone(),
                    target.endpoint.clone(),
                    route
                        .originate
                        .clone()
                        .with_request_write_authority(writer.clone()),
                )
                .map_err(|_| Error::forbidden()),
        )?;
        let signals = allocation_stage(
            "signal-custody",
            context.take_signals().map_err(|_| Error::forbidden()),
        )?;
        let begin_cutoff = read
            .expires_at()
            .min(original_cutoff.load(Ordering::Acquire));
        drop(read);
        // PRE-reservation contention has no effect to replay. Re-derive only
        // within the original cutoff; once an attempt returns, leave this loop.
        let admission =
            admission::Admission::new(root.deadline, root.handle.cancellation(), begin_cutoff);
        let prepared = root.receipt.clone();
        let original_route = route.clone();
        let begun = admission
            .run(self, move |store, until, wait| {
                let read = match store.telephone_replacement_allocation_read_with_wait(
                    &prepared,
                    &original_route.credential_id,
                    &original_route.policy,
                    until,
                    wait,
                ) {
                    Ok(read) => read,
                    Err(error) if error.code == "authority-busy" => return Ok(None),
                    Err(error) => return Err(error),
                };
                store.try_begin_telephone_replacement_allocation_with_wait(read, wait)
            })
            .await;
        let attempt = allocation_stage("allocation-begin", begun)?;
        let original_attempt = attempt.clone();
        let ready = admission
            .read(self, move |store, until, wait| {
                store.telephone_replacement_allocation_begun_read_with_wait(
                    original_attempt.clone(),
                    until,
                    wait,
                )
            })
            .await;
        let read = match allocation_stage("begun-current", ready) {
            Ok(read) => read,
            Err(error) => {
                root.handle.cancel();
                let capture = Arc::new(attempt.uncertain());
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    store_call(move || store.record_telephone_replacement_uncertain(&capture))
                })
                .await?;
                return Err(error);
            }
        };
        let invocation = control::Invocation {
            runtime: self.clone(),
            root: root.clone(),
            route: route.clone(),
            target: target.clone(),
            phase: control::Phase::Allocate(Arc::new(read)),
        };
        let result = self.core.prepare_adapter_device_replacement_with_held(
            &root.handle,
            context.clone(),
            &invocation,
        );
        drop(invocation);
        let capture = allocation_stage("core-allocate", result.map_err(|_| Error::forbidden()))
            .and_then(|actual| {
                if actual != root.handle.intent().replacement {
                    return Err(Error::forbidden());
                }
                let mut observation = observed(&root.handle, &root.plan)?;
                observation["allocated"] = serde_json::json!(true);
                observation["transport"] = serde_json::json!("sip");
                let reference = context
                    .prepared_call_reference()
                    .map_err(|_| Error::forbidden())?;
                observation["call_reference"] =
                    serde_json::json!({"kind":reference.kind(),"value":reference.expose_secret()});
                attempt.allocated(observation)
            });
        let receipt = match allocation_stage("capture", capture) {
            Ok(receipt) => Arc::new(receipt),
            Err(error) => {
                root.handle.cancel();
                let unknown = attempt.uncertain();
                let store = self.media.store.clone();
                store_call(move || store.record_telephone_replacement_uncertain(&unknown)).await?;
                return Err(error);
            }
        };
        let allocated = Arc::new(Allocated {
            _packet_observers: packet_observers,
            context,
            receipt,
            route,
            target,
            writer,
            signals: tokio::sync::Mutex::new(signals),
            submission: OnceLock::new(),
            submitted: Mutex::new(None),
            uncertain: Mutex::new(None),
            resources: Mutex::new(None),
            resource_failure: Mutex::new(None),
        });
        *root.telephone.lock().map_err(|_| Error::storage())? = Some(allocated.clone());
        allocation_stage(
            "record-allocation",
            self.record_telephone_allocation(&allocated).await,
        )?;
        Ok(allocated)
    }
    async fn record_telephone_allocation(&self, leg: &Allocated) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = leg.receipt.clone();
            store_call(move || store.record_telephone_replacement_allocated(&receipt))
        })
        .await?;
        Ok(())
    }
    pub(crate) async fn submit_telephone_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        leg: &Arc<Allocated>,
    ) -> Result<()> {
        // One original CONTROL budget, including first Read and typed begin.
        // No role mutex/current-authority reader survives ringing.
        let admission =
            admission::Admission::new(root.deadline, root.handle.cancellation(), i64::MAX);
        let receipt = leg.receipt.clone();
        let attempt = admission
            .run(self, move |store, until, wait| {
                let read = match store.telephone_replacement_submission_read_with_wait(
                    receipt.clone(),
                    until,
                    wait,
                ) {
                    Ok(read) => read,
                    Err(error) if error.code == "authority-busy" => return Ok(None),
                    Err(error) => return Err(error),
                };
                store.try_begin_telephone_replacement_submission_with_wait(read, wait)
            })
            .await?;
        leg.submission
            .set(attempt.clone())
            .map_err(|_| Error::forbidden())?;
        leg.writer
            .attempt
            .set(attempt.clone())
            .map_err(|_| Error::forbidden())?;
        let original_attempt = attempt.clone();
        let ready = admission
            .read(self, move |store, until, wait| {
                store.telephone_replacement_submission_begun_read_with_wait(
                    original_attempt.clone(),
                    until,
                    wait,
                )
            })
            .await;
        match ready {
            Ok(read) => drop(read),
            Err(error) => {
                tracing::debug!(code=%error.code, "Original telephone start admission deferred until its cutoff");
                return self
                    .uncertain_telephone_submission(root, leg, &attempt)
                    .await;
            }
        }
        let invocation = control::Invocation {
            runtime: self.clone(),
            root: root.clone(),
            route: leg.route.clone(),
            target: leg.target.clone(),
            phase: control::Phase::Start(attempt.clone()),
        };
        // Existing drive offloads individual synchronous polls; Pending releases
        // its thread. Do not park a blocking worker throughout SIP ringing.
        let core = self.core.clone();
        let original = root.handle.clone();
        let result = self
            .clone()
            .owned_control_call(async move {
                Ok(core
                    .start_adapter_device_replacement_with_held(&original, &invocation)
                    .await)
            })
            .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                return self
                    .uncertain_telephone_submission(root, leg, &attempt)
                    .await
            }
        };
        match result {
            Ok(activation) => {
                let references = activation.external_references();
                if references.len() != 1
                    || references[0].kind() != "sip.call-id"
                    || references[0].expose_secret() != leg.receipt.call_id()
                {
                    return self
                        .uncertain_telephone_submission(root, leg, &attempt)
                        .await;
                }
                let capture = Arc::new(attempt.submitted(serde_json::json!({"submitted":true,"call_reference":{"kind":references[0].kind(),"value":references[0].expose_secret()}}))?);
                *leg.submitted.lock().map_err(|_| Error::storage())? = Some(capture.clone());
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    store_call(move || store.record_telephone_replacement_submitted(&capture))
                })
                .await?;
                let result = self.prepare_telephone_resources(root, leg, capture).await;
                if result.is_err() {
                    root.handle.cancel();
                    let _ = self.abandon_telephone_submission(leg).await;
                }
                result
            }
            Err(error) => {
                tracing::debug!(target:"connect_service::telephone_replacement", ?error, "Actual Core telephone start did not complete");
                self.uncertain_telephone_submission(root, leg, &attempt)
                    .await
            }
        }
    }
    pub(crate) async fn abandon_telephone_submission(&self, leg: &Allocated) -> Result<()> {
        let submitted = { leg.submitted.lock().map_err(|_| Error::storage())?.clone() };
        if let Some(submitted) = submitted {
            retry_metadata(&self.stop, || {
                let store = self.media.store.clone();
                let submitted = submitted.clone();
                store_call(move || store.abandon_telephone_replacement(&submitted))
            })
            .await?;
        }
        Ok(())
    }
    async fn uncertain_telephone_submission(
        &self,
        root: &PreparedReplacement,
        leg: &Allocated,
        attempt: &Arc<TelephoneReplacementAttempt>,
    ) -> Result<()> {
        root.handle.cancel();
        let capture = Arc::new(attempt.uncertain());
        *leg.uncertain.lock().map_err(|_| Error::storage())? = Some(capture.clone());
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.record_telephone_replacement_uncertain(&capture))
        })
        .await?;
        Err(Error::new(
            "reconciliation-required",
            "Telephone submission requires reconciliation",
        ))
    }
}
