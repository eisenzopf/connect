//! Bounded Session-policy jobs use actual current peers, then retain only
//! historical outcome metadata. Neither a copied job nor a retry grants IO.
use crate::{
    canonical_media::{self, reconciliation::Grant, Connection},
    canonical_runtime::Runtime,
};
use async_trait::async_trait;
use connect_protocol::Error;
use connect_store::{
    NativeStreamCommitted, NativeStreamLease, NativeStreamNoEffect, SessionCodecTarget, Store,
};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, DispatchError},
    native_negotiation::NativeOfferError,
    native_requests::{NativeRequestPolicy, NativeRequestScope},
    native_session_codecs::{NativeSessionCodecPlan, NativeSessionCodecPolicy},
    Envelope,
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

mod admission;
use admission::Slot;

struct Policy {
    runtime: Weak<Runtime>,
    entry: Arc<Connection>,
    target: SessionCodecTarget,
    domain: Arc<()>,
    attempt: Arc<Slot<NativeStreamLease>>,
}
#[async_trait]
impl NativeSessionCodecPolicy for Policy {
    async fn authorize_codec(
        &self,
        scope: &NativeRequestScope,
        plan: &NativeSessionCodecPlan,
        operation: &str,
    ) -> Result<AuthorityLease, DispatchError> {
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        if runtime.stop.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        // Claim once without retaining a mutex across Store work or await.
        let pending = self.attempt.claim()?;
        let source = Arc::new(
            runtime
                .media
                .store
                .try_native_session_codec_read(&self.entry.origin, &self.target)
                .map_err(|error| { tracing::debug!(target:"connect_service::canonical_session_codecs",code=%error.code,"Current codec source deferred"); DispatchError::Authority })?,
        );
        // The temporary complete grant checks the original selected publisher
        // set, but none of its source Reads may survive into SQL mutation.
        let grant = runtime.media.codec_grant(
            self.domain.clone(),
            self.entry.clone(),
            scope,
            source.clone(),
            plan,
            operation.into(),
            runtime.stop.child_token(),
        )?;
        let descriptor = grant.descriptor()?;
        drop(grant);
        let source = Arc::try_unwrap(source).map_err(|_| DispatchError::Authority)?;
        let store = runtime.media.store.clone();
        let original_operation = operation.to_owned();
        runtime
            .owned_store_call(move || {
                // Store the opaque original reservation before returning to a
                // cancellable receiver. No dropped waiter can lose custody.
                pending.run(|| {
                    store.begin_native_session_codec(source, &original_operation, descriptor)
                })
            })
            .await
            .map_err(|error| { tracing::debug!(target:"connect_service::canonical_session_codecs",code=%error.code,"Codec admission deferred"); DispatchError::Authority })?;
        if runtime.stop.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        // An original reservation already exists. A transient immediate SQL
        // refusal must retain it and retry only this fresh read, never begin.
        let until = self.attempt.with_ready(|lease| Ok(lease.expires_at()))?;
        let cutoff = tokio::time::Instant::now()
            + Duration::from_secs((until - chrono::Utc::now().timestamp()).max(0) as u64);
        retry_original_codec_current(
            &runtime.stop,
            cutoff,
            || chrono::Utc::now().timestamp() < until,
            || self.current_attempt_grant(&runtime, scope, plan, operation),
        )
        .await
    }
    async fn authorize_codec_activation(
        &self,
        original: &rvoip_uctp::v2::native_session_codecs::NativeSessionCodecActivation,
    ) -> Result<AuthorityLease, DispatchError> {
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        // Borrow only the original Slot receipt; never claim or begin again.
        retry_original_codec_current(
            &runtime.stop,
            original.deadline(),
            || original.current(),
            || {
                self.current_attempt_grant(
                    &runtime,
                    original.scope(),
                    original.plan(),
                    original.operation(),
                )
            },
        )
        .await
    }
}
async fn retry_original_codec_current<T>(
    stop: &CancellationToken,
    cutoff: tokio::time::Instant,
    current: impl Fn() -> bool,
    mut read: impl FnMut() -> Result<T, DispatchError>,
) -> Result<T, DispatchError> {
    loop {
        if stop.is_cancelled() || tokio::time::Instant::now() >= cutoff || !current() {
            return Err(DispatchError::Closed);
        }
        match read() {
            Err(DispatchError::StorageUnavailable) => {
                tokio::select! { biased;
                    _ = stop.cancelled() => return Err(DispatchError::Closed),
                    _ = tokio::time::sleep_until(cutoff) => return Err(DispatchError::Closed),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
            result => return result,
        }
    }
}
#[cfg(all(test, feature = "test-support"))]
mod current_read_tests;

fn current_codec_error(error: Error) -> DispatchError {
    tracing::debug!(target:"connect_service::canonical_session_codecs", code=%error.code,
        "Original codec attempt current admission refused");
    if matches!(
        error.code.as_str(),
        "authority-busy" | "storage-unavailable"
    ) {
        DispatchError::StorageUnavailable
    } else {
        DispatchError::Authority
    }
}
impl Policy {
    fn current_attempt_grant(
        &self,
        runtime: &Arc<Runtime>,
        scope: &NativeRequestScope,
        plan: &NativeSessionCodecPlan,
        operation: &str,
    ) -> Result<AuthorityLease, DispatchError> {
        self.attempt.with_ready(|lease| {
            let source = Arc::new(
                runtime
                    .media
                    .store
                    .try_native_session_codec_attempt_read(&self.entry.origin, &self.target, lease)
                    .map_err(current_codec_error)?,
            );
            let grant = runtime.media.codec_grant(
                self.domain.clone(),
                self.entry.clone(),
                scope,
                source,
                plan,
                operation.into(),
                runtime.stop.child_token(),
            )?;
            runtime
                .media
                .store
                .validate_native_session_codec_plan(
                    grant.source.codec()?,
                    lease,
                    &grant.descriptor()?,
                )
                .map_err(current_codec_error)?;
            Ok(canonical_media::reconciliation::lease(grant))
        })
    }
}
#[async_trait]
impl NativeRequestPolicy for Policy {
    async fn authorize(
        &self,
        scope: &NativeRequestScope,
        proposal: &Envelope,
        held: Option<&AuthorityLease>,
    ) -> Result<AuthorityLease, DispatchError> {
        let held = held.ok_or(DispatchError::Authority)?;
        let grant = held
            .authority::<Arc<Grant>>()
            .ok_or(DispatchError::Authority)?;
        if !held.current()
            || !Arc::ptr_eq(&grant.domain, &self.domain)
            || grant.source.codec().is_err()
        {
            return Err(DispatchError::Authority);
        }
        grant.authorize_proposal(scope, proposal)?;
        Ok(held.retained())
    }
}
impl Runtime {
    pub(crate) fn ensure_session_codec_worker(self: &Arc<Self>) -> connect_protocol::Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::new("unavailable", "Session codec runtime is not running"))?;
        let mut worker = self
            .session_codec_worker
            .lock()
            .map_err(|_| Error::storage())?;
        if worker.as_ref().is_some_and(|w| !w.is_finished()) {
            return Ok(());
        }
        let runtime = Arc::downgrade(self);
        let store = self.media.store.clone();
        let stop = self.stop.child_token();
        *worker = Some(handle.spawn(run(runtime, store.clone(), stop)));
        // A failed installation cannot leave a usable availability marker.
        if let Err(error) = store.install_session_codec_coordinator() {
            if let Some(worker) = worker.take() {
                worker.abort();
            }
            return Err(error);
        }
        Ok(())
    }
}
struct Installed(Arc<Store>);
impl Drop for Installed {
    fn drop(&mut self) {
        self.0.uninstall_session_codec_coordinator();
    }
}
async fn run(runtime: Weak<Runtime>, store: Arc<Store>, stop: CancellationToken) {
    let _installed = Installed(store.clone());
    let mut jobs = tokio::task::JoinSet::new();
    let mut active = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut recovery = tokio::time::Instant::now();
    loop {
        tokio::select! { biased;
            _ = stop.cancelled() => break,
            result = jobs.join_next_with_id(), if !jobs.is_empty() => {
                match result { Some(Ok((id, ()))) => { active.remove(&id); }, Some(Err(error)) => { active.remove(&error.id()); }, None => () }
            }
            _ = interval.tick() => {
                let Some(host) = runtime.upgrade() else { break };
                host.reap_control_polls();
                if let Ok(mut origins) = host.codec_origins.lock() { origins.retain(|_, origin| origin.current()); }
                drop(host);
                let recover = recovery.elapsed() >= Duration::from_secs(1);
                if recover { recovery = tokio::time::Instant::now(); }
                let db = store.clone();
                let scan = crate::runtime_retention::spawn_blocking(crate::runtime_retention::Kind::CodecScan, move || {
                    // Existing owned scan, before taking any SQL/file guards.
                    // Lease Drop only queued this exact original close batch.
                    db.reap_idle_packet_observers()?;
                    if recover { db.recover_native_streams(64)?; }
                    db.reconcile_session_codec_jobs(db.limits().connections.min(64))?;
                    db.pending_session_codec_targets(db.limits().connections.min(32))
                }).await;
                let Ok(Ok(targets)) = scan else { continue };
                for target in targets {
                    if active.len() >= 16 { break; }
                    let key = (target.batch_record_id.clone(), target.connection_id.clone());
                    if active.values().any(|v| v == &key) { continue; }
                    let job = jobs.spawn(execute(runtime.clone(), store.clone(), stop.child_token(), target));
                    active.insert(job.id(), key);
                }
            }
        }
    }
    // Stop each original execution, then join its pending admission and captured
    // outcome. Runtime drain retains the same outer deadline; a timeout remains
    // an incomplete drain, never permission to claim natural closure.
    stop.cancel();
    while let Some(result) = jobs.join_next().await {
        if result.is_err() {
            if let Some(host) = runtime.upgrade() {
                host.retirement_failed
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
    }
}
enum Captured {
    Success(Arc<NativeStreamCommitted>),
    NoEffect(Arc<NativeStreamNoEffect>),
    Unknown(Arc<NativeStreamLease>),
}
async fn record(store: Arc<Store>, stop: &CancellationToken, captured: Captured) {
    let captured = Arc::new(captured);
    loop {
        let db = store.clone();
        let value = captured.clone();
        let result = tokio::task::spawn_blocking(move || match value.as_ref() {
            Captured::Success(receipt) => db.complete_native_stream(receipt).map(|_| ()),
            Captured::NoEffect(receipt) => {
                db.record_native_stream_without_effect(receipt).map(|_| ())
            }
            Captured::Unknown(lease) => db.native_stream_attempt_uncertain(lease).map(|_| ()),
        })
        .await;
        if matches!(result, Ok(Ok(()))) {
            return;
        }
        tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_millis(100)) => () }
    }
}
async fn execute(
    runtime: Weak<Runtime>,
    store: Arc<Store>,
    stop: CancellationToken,
    target: SessionCodecTarget,
) {
    use sha2::{Digest, Sha256};
    let operation = format!(
        "scodec_child_{}",
        hex::encode(Sha256::digest(
            format!("{}:{}", target.batch_record_id, target.connection_id).as_bytes()
        ))
    );
    while !stop.is_cancelled() && chrono::Utc::now().timestamp() < target.expires_at {
        // Take metadata from an observed signed initial offer or credential
        // refresh. No database snapshot can manufacture a physical origin.
        let parts = runtime.upgrade().and_then(|host| {
            let origin = host
                .codec_origins
                .lock()
                .ok()?
                .get(&target.connection_id)?
                .clone();
            let id = origin.request_scope(0).origin.media.connection.clone();
            let entry = host.media.entry(&id).ok()?;
            Some((host.connection_native_adapter(&id).ok()?, origin, id, entry))
        });
        if let Some((native, origin, id, entry)) = parts {
            match native.plan_native_session_codecs(&id, &origin, &target.codec_preferences) {
                Ok(plan) => {
                    let policy = Arc::new(Policy {
                        runtime: runtime.clone(),
                        entry: entry.clone(),
                        target: target.clone(),
                        domain: Arc::new(()),
                        attempt: Arc::new(Slot::default()),
                    });
                    let result = native
                        .execute_native_session_codecs(&id, plan, operation.clone(), policy.clone())
                        .await;
                    if let Err(error) = &result {
                        tracing::debug!(target:"connect_service::canonical_session_codecs",?error,"Native codec execution returned");
                    }
                    // The upstream cancellation/deadline select may have dropped
                    // authorize while its original blocking admission still ran.
                    let lease = policy.attempt.take_when_done().await;
                    // Drop policy metadata before recording; engine-owned
                    // readers were released when execution returned.
                    drop(policy);
                    if let Some(lease) = lease {
                        let captured = match result {
                            Ok(outcome) => {
                                entry.remember_bindings(outcome.bindings());
                                match crate::canonical_native_streams::codec_bindings(
                                    outcome.bindings(),
                                ) {
                                    Ok(bindings) => match lease.session_codecs_committed(
                                        outcome.revision(),
                                        outcome.resources_changed(),
                                        bindings,
                                    ) {
                                        Ok(receipt) => Captured::Success(Arc::new(receipt)),
                                        // This is a host invariant failure after
                                        // physical success. Expiry recovery
                                        // preserves uncertainty; never retry IO.
                                        Err(_) => return,
                                    },
                                    Err(_) => Captured::Unknown(Arc::new(lease)),
                                }
                            }
                            Err(error) if !error.effect_possible => {
                                Captured::NoEffect(Arc::new(lease.without_effect(Error::new(
                                    "codec-refused",
                                    "Native Session codec preflight was refused",
                                ))))
                            }
                            Err(_) => Captured::Unknown(Arc::new(lease)),
                        };
                        record(store, &stop, captured).await;
                        return;
                    }
                }
                Err(NativeOfferError::Invalid | NativeOfferError::Unsupported) => {
                    let db = store.clone();
                    let target = target.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        db.refuse_session_codec_target(&target, false)
                    })
                    .await;
                    return;
                }
                Err(_) => (),
            }
        }
        // No admission occurred: a busy controller, credential transition or
        // missing physical origin may settle before the fixed batch deadline.
        tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_millis(100)) => () }
    }
}
