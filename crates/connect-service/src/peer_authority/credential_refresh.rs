//! One owned, bounded refresh of the exact physical peer's current media.
use super::*;
use crate::canonical_media::credential_refresh::Invocation;
use rvoip_core::ids::ConnectionId;
use rvoip_uctp::v2::auth_dispatch::VerifiedCredentialRefresh;
use std::collections::BTreeMap;

struct PhysicalFence {
    refresh: Arc<VerifiedCredentialRefresh>,
    armed: bool,
}
impl Drop for PhysicalFence {
    fn drop(&mut self) {
        if self.armed {
            self.refresh.retire_physical();
        }
    }
}
// Reserve bounded factual-result custody before the first possible effect.
struct ReceiptSlot {
    runtime: Arc<crate::canonical_runtime::Runtime>,
    id: String,
    retained: bool,
}
impl Drop for ReceiptSlot {
    fn drop(&mut self) {
        if !self.retained {
            self.runtime
                .credential_refresh_receipts
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.id);
        }
    }
}
pub(super) async fn refresh(
    policy: &StorePeerAuthority,
    refresh: Arc<VerifiedCredentialRefresh>,
    held: AuthorityLease,
) -> Result<AuthorityLease, DispatchError> {
    let Some(runtime) = policy.runtime.upgrade() else {
        return Ok(held);
    };
    let Some(physical) = runtime
        .physical_for_peer(refresh.peer())
        .map_err(|_| DispatchError::Authority)?
    else {
        return Ok(held);
    };
    let mut entries = Vec::new();
    for entry in runtime
        .media
        .connections()
        .map_err(|_| DispatchError::Authority)?
    {
        let id = ConnectionId::from_string(entry.origin.runtime_connection_id());
        if runtime
            .native_adapter(physical.binding.transport())
            .map_err(|_| DispatchError::Authority)?
            .physical_source(&id)
            .is_ok_and(|p| Arc::ptr_eq(&p, &physical.binding))
        {
            if !rvoip_core::staged_streams::reliable_data::same_credential(
                &entry.principal,
                &refresh.previous().principal,
            ) {
                return Err(DispatchError::Authority);
            }
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return Ok(held);
    }
    if refresh.previous().token_id == refresh.next().token_id {
        refresh.mark_media_transition_started();
        let mut fence = PhysicalFence {
            refresh: refresh.clone(),
            armed: true,
        };
        for entry in entries {
            let id = ConnectionId::from_string(entry.origin.runtime_connection_id());
            let result = runtime
                .native_adapter(physical.binding.transport())
                .map_err(|_| DispatchError::Authority)?
                .refresh_unchanged_credential(&id, refresh.clone(), &held)
                .map_err(|_| DispatchError::Authority)?;
            result.install_data_origin()?;
            runtime.refresh_native_lifecycle(&id, &result, &physical)?;
            runtime.refresh_native_dtmf(&id, &result, &physical)?;
            runtime.refresh_capability_delivery(&id, &result, &physical)?;
            runtime.refresh_native_reconciliation(&id, &result)?;
        }
        if !held.current() || !refresh.current() {
            return Err(DispatchError::Authority);
        }
        fence.armed = false;
        return Ok(held);
    }
    let (request, binding) = {
        let a = held
            .authority::<StoreInvocationAuthority>()
            .ok_or(DispatchError::Authority)?;
        let g = a.guard::<Arc<Grant>>().ok_or(DispatchError::Authority)?;
        if g.policy != policy.id
            || !g.proof.current()
            || g.invocation_id != refresh.refresh_id()
            || g.request.credentials
                != vec![
                    refresh
                        .previous()
                        .token_id
                        .clone()
                        .ok_or(DispatchError::Authentication)?,
                    refresh
                        .next()
                        .token_id
                        .clone()
                        .ok_or(DispatchError::Authentication)?,
                ]
            || g.request.key_id != refresh.key_id()
            || g.request.algorithm != refresh.algorithm()
            || !matches!(g.request.command, PeerCommand::Binding)
        {
            return Err(DispatchError::Authority);
        }
        (g.request.clone(), a.binding().clone())
    };
    // Never retain the original Binding-only readers while acquiring a larger
    // union behind a waiting writer. The successor dispatch keeps only metadata.
    drop(held);
    let policy_id = policy.id.clone();
    let stop = policy.stop.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut work = runtime.work.lock().map_err(|_| DispatchError::Capacity)?;
        while work.try_join_next().is_some() {}
        if runtime.stop.is_cancelled()
            || work.len() >= runtime.media.store.limits().connections.saturating_mul(2)
        {
            return Err(DispatchError::Capacity);
        }
        let runtime = runtime.clone();
        work.spawn(async move {
            let result = worker(
                runtime,
                physical,
                entries,
                refresh.clone(),
                request,
                binding,
                policy_id,
                stop,
            )
            .await;
            let succeeded = result.is_ok();
            if tx.send(result).is_err() && succeeded {
                refresh.retire_physical();
            }
        });
    }
    rx.await.map_err(|_| DispatchError::Closed)?
}
async fn worker(
    runtime: Arc<crate::canonical_runtime::Runtime>,
    physical: Arc<crate::canonical_runtime::Physical>,
    entries: Vec<Arc<crate::canonical_media::Connection>>,
    refresh: Arc<VerifiedCredentialRefresh>,
    request: PeerAuthorityRequest,
    binding: crate::StoreCredentialBinding,
    policy: String,
    stop: CancellationToken,
) -> Result<AuthorityLease, DispatchError> {
    let remaining = refresh
        .deadline()
        .saturating_duration_since(tokio::time::Instant::now())
        .min(Duration::from_secs(2));
    let wait = AuthorityWait::new(remaining);
    let _cancel = wait.cancel_on_drop();
    let origins = entries.iter().map(|e| e.origin.clone()).collect::<Vec<_>>();
    let store = runtime.media.store.clone();
    let req = request.clone();
    let admission_wait = wait.clone();
    let admission = tokio::task::spawn_blocking(move || {
        store.admit_native_credential_refresh(&req, &origins, &admission_wait)
    });
    let (mut proof, plan) = tokio::select! {biased;
        r=admission=>r.map_err(|_|DispatchError::Authority)?.map_err(store_error)?,
        _=refresh.cancellation().cancelled_owned()=>return Err(DispatchError::Closed),
        _=runtime.stop.cancelled()=>return Err(DispatchError::Closed),
    };
    let expiry = refresh
        .previous()
        .principal
        .expires_at
        .ok_or(DispatchError::Authority)?
        .min(
            refresh
                .next()
                .principal
                .expires_at
                .ok_or(DispatchError::Authority)?,
        )
        .timestamp();
    proof.limit_expiry(expiry).map_err(store_error)?;
    let plan = Arc::new(plan);
    let grant = Arc::new(Grant {
        policy,
        invocation_id: refresh.refresh_id().into(),
        proof,
        request,
        credential: refresh.next().clone(),
        native_stream_plan: Mutex::new(None),
        native_data: Mutex::new(Weak::new()),
    });
    let current = grant.clone();
    let check = refresh.clone();
    let lease = AuthorityLease::checking(
        StoreInvocationAuthority::holding(binding, grant.clone()),
        refresh.deadline(),
        move || current.proof.current() && check.current() && !stop.is_cancelled(),
    );
    let mut pairs = Vec::new();
    let mut invocations = Vec::new();
    for (index, target) in plan.targets().iter().enumerate() {
        let old = entries
            .iter()
            .find(|e| Arc::ptr_eq(&e.origin, target.original()))
            .ok_or(DispatchError::Authority)?;
        let id = ConnectionId::from_string(old.origin.runtime_connection_id());
        let source = runtime
            .native_adapter(physical.binding.transport())
            .map_err(|_| DispatchError::Authority)?
            .physical_source(&id)
            .map_err(|_| DispatchError::Authority)?;
        let controller = runtime
            .native_adapter(physical.binding.transport())
            .map_err(|_| DispatchError::Authority)?
            .media_controller(&id)
            .map_err(|_| DispatchError::Authority)?;
        if !Arc::ptr_eq(&source, &physical.binding)
            || controller.credential_revision() != target.expected_credential_revision()
            || controller.revision() != target.stream_revision()
            || !controller.matches_data_credential(&refresh.previous().principal)
        {
            return Err(DispatchError::Authority);
        }
        let next = runtime
            .media
            .successor(old, target.next().clone(), refresh.next().principal.clone())
            .map_err(|_| DispatchError::Authority)?;
        pairs.push((old.clone(), next.clone()));
        invocations.push(Invocation {
            policy: runtime.media.clone(),
            plan: plan.clone(),
            grant: grant.clone(),
            refresh: refresh.clone(),
            index,
            next,
        });
    }
    if !lease.current() {
        return Err(DispatchError::Authority);
    }
    let mut receipt_slot = {
        let mut receipts = runtime
            .credential_refresh_receipts
            .lock()
            .map_err(|_| DispatchError::Capacity)?;
        if receipts.len() >= runtime.media.store.limits().connections.saturating_mul(2)
            || receipts.contains_key(refresh.refresh_id())
        {
            return Err(DispatchError::Capacity);
        }
        receipts.insert(refresh.refresh_id().into(), None);
        ReceiptSlot {
            runtime: runtime.clone(),
            id: refresh.refresh_id().into(),
            retained: false,
        }
    };
    let attempt = runtime
        .media
        .store
        .prepare_native_credential_refresh(&plan, &grant.proof, refresh.refresh_id())
        .map_err(store_error)?;
    refresh.mark_media_transition_started();
    let mut fence = PhysicalFence {
        refresh: refresh.clone(),
        armed: true,
    };
    if let Err(error) =
        runtime
            .media
            .store
            .begin_native_credential_refresh(&attempt, &plan, &grant.proof)
    {
        let _ = runtime
            .media
            .store
            .record_native_credential_refresh_stopped(&attempt, false);
        return Err(store_error(error));
    }
    let mut results = Vec::new();
    let mut revisions = BTreeMap::new();
    for invocation in &invocations {
        let target = &plan.targets()[invocation.index];
        let id = ConnectionId::from_string(target.original().runtime_connection_id());
        match runtime
            .native_adapter(physical.binding.transport())
            .map_err(|_| DispatchError::Authority)?
            .rebind_verified_credential_with_held(
                &id,
                refresh.clone(),
                target.expected_credential_revision(),
                target.stream_revision(),
                invocation,
                &lease,
            )
            .await
        {
            Ok(result) => {
                revisions.insert(
                    target.original().offer_record_id().into(),
                    result.credential_revision(),
                );
                results.push(result);
            }
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "Original native credential rotation did not return a complete observation"
                );
                let _ = runtime
                    .media
                    .store
                    .record_native_credential_refresh_stopped(&attempt, true);
                return Err(DispatchError::Authority);
            }
        }
    }
    let receipt = Arc::new(attempt.applied(revisions).map_err(store_error)?);
    runtime
        .credential_refresh_receipts
        .lock()
        .map_err(|_| DispatchError::Authority)?
        .insert(refresh.refresh_id().into(), Some(receipt.clone()));
    receipt_slot.retained = true;
    runtime
        .media
        .store
        .record_native_credential_refresh_applied(&receipt)
        .map_err(store_error)?;
    receipt_slot.retained = false;
    runtime
        .credential_refresh_receipts
        .lock()
        .map_err(|_| DispatchError::Authority)?
        .remove(refresh.refresh_id());
    runtime
        .media
        .install_successors(&pairs)
        .map_err(|_| DispatchError::Authority)?;
    for ((_, next), result) in pairs.iter().zip(results) {
        let id = ConnectionId::from_string(next.origin.runtime_connection_id());
        result.install_data_origin()?;
        runtime.refresh_native_lifecycle(&id, &result, &physical)?;
        runtime.refresh_native_dtmf(&id, &result, &physical)?;
        runtime.refresh_capability_delivery(&id, &result, &physical)?;
        runtime.refresh_native_reconciliation(&id, &result)?;
    }
    if !lease.current() {
        return Err(DispatchError::Authority);
    }
    fence.armed = false;
    Ok(lease)
}
