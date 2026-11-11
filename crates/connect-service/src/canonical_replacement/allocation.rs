//! A signed target consumes the exact owned Core reservation, remaining closed.
use super::*;
use crate::{
    canonical_native::Reply,
    canonical_runtime::{current_member, Physical},
    StoreInvocationAuthority,
};
use rvoip_core::{
    ids::IdentityId,
    orchestrator::peer_native::{PeerNativeAuthority, PeerNativeAuthorityLease},
    relationships::{EndpointBinding, IdentityBinding},
};
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
use std::sync::Weak;

pub(super) struct AllocatedReplacement {
    pub receipt: Arc<connect_store::NativeReplacementAllocated>,
    pub physical: Weak<Physical>,
    pub(super) webrtc: Option<Arc<rvoip_webrtc::canonical::WebRtcPhysicalBinding>>,
    pub(super) intent: connect_protocol::connection_control::OfferIntent,
    pub(super) recorded: AtomicBool,
    pub(super) failed_start:
        std::sync::Mutex<Option<Arc<connect_store::NativeReplacementStartUncertain>>>,
    pub(super) started: std::sync::Mutex<Option<Arc<super::start::StartedReplacement>>>,
}
struct TargetLease {
    authority: PeerNativeAuthority,
    _read: connect_store::NativeReplacementAllocationRead,
}
impl PeerNativeAuthorityLease for TargetLease {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.authority
    }
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|a| a.peer_guard())
        .ok_or_else(Error::forbidden)
}
// One finite control-stage observation on the original error path. No payload,
// identity, new admission, retry or timing budget is introduced.
pub(super) fn retire_replacement_authority(a: Arc<AuthorizedEnvelope>) -> Result<()> {
    let original = Arc::try_unwrap(a).map_err(|_| {
        Error::new(
            "conflict",
            "Original replacement capture invocation is still shared",
        )
    })?;
    original
        .retire_replacement_authority()
        .map_err(|_| Error::forbidden())
}
fn allocation_refused(stage: &'static str, error: Error) -> Error {
    let code = match error.code.as_str() {
        "authority-busy" => "authority-busy",
        "authority-unavailable" => "authority-unavailable",
        "storage-unavailable" => "storage-unavailable",
        "not-ready" => "not-ready",
        "forbidden" => "forbidden",
        "conflict" => "conflict",
        "capacity-exceeded" => "capacity-exceeded",
        "reconciliation-required" => "reconciliation-required",
        _ => "other",
    };
    tracing::debug!(target: "connect_service::canonical_replacement::allocation",
        stage, code, "Original replacement allocation refused");
    error
}
impl Runtime {
    pub(crate) async fn allocate_native_replacement(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        tracing::debug!(target: "connect_service::canonical_replacement::allocation",
            stage="entry", "Original replacement allocation entered");
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical
            .validate(&a, authority)
            .map_err(|error| allocation_refused("preflight-physical", error))?;
        let intent = crate::canonical_native::intent(a.envelope())?;
        let record = intent.offer["replacement_preparation"]["operation_record_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        // An allocation holds current peer authority. Waiting here for the
        // preparation worker can retain a reader in front of its queued writer.
        // Refuse before any effect instead; callers observe the original
        // prepare effect's confirmation before submitting the target offer.
        let saved = self
            .replacements
            .lock()
            .map_err(|_| allocation_refused("preflight-owner-registry", Error::storage()))?
            .get(record)
            .cloned()
            .filter(|saved| saved.recorded.load(Ordering::Acquire))
            .ok_or_else(|| {
                allocation_refused(
                    "preflight-prepared",
                    Error::new("not-ready", "Original preparation is not recorded"),
                )
            })?;
        let mut pending = Some(a);
        let mut deferred_sql = false;
        let (a, _roles, attempt, read, (actor, endpoint, peer), target_key_id) = loop {
            let a = pending.take().ok_or_else(Error::storage)?;
            // No allocation effect or physical owner exists yet. Release BOTH
            // application aliases before waiting; only this unique signed command,
            // original root/cutoff, reply correlation and cancellation remain.
            let (a, _roles) = match self.role_projection.try_lock() {
                Ok(roles) if !deferred_sql => (a, roles),
                roles => {
                    // A pre-SQL deferral is effect-free. Release the coordinator
                    // and BOTH application aliases before any pause or fresh read.
                    drop(roles.ok());
                    let root_cancel = saved.handle.cancellation();
                    let invocation_cancel = a.invocation_cancellation();
                    let original = Arc::try_unwrap(a).map_err(|_| {
                        allocation_refused(
                            "preflight-owner",
                            Error::new(
                                "conflict",
                                "Original allocation invocation is still shared",
                            ),
                        )
                    })?;
                    let (a, deadline) = original
                        .into_replacement_allocation_wait(saved.deadline)
                        .map_err(|_| {
                        allocation_refused("preflight-suspend", Error::forbidden())
                    })?;
                    #[cfg(feature = "test-support")]
                    let wait_probe = self
                        .media
                        .take_preparation_registry_probe_for_test(&format!(
                            "allocation-wait:{record}"
                        ));
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = &wait_probe {
                        probe.store(1, Ordering::Release);
                    }
                    if deferred_sql {
                        tokio::select! { biased;
                            _ = self.stop.cancelled() => return Err(Error::forbidden()),
                            _ = physical.lifetime.cancelled() => return Err(Error::forbidden()),
                            _ = root_cancel.cancelled() => return Err(Error::forbidden()),
                            _ = invocation_cancel.cancelled() => return Err(Error::forbidden()),
                            _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(5))) => {},
                        }
                    }
                    // Waiting owns no Store/SQL/file proof. Fresh authorization is acquired
                    // only AFTER the coordinator; every cancellation uses the same owner.
                    let _roles = tokio::select! { biased;
                        _ = self.stop.cancelled() => return Err(Error::forbidden()),
                        _ = physical.lifetime.cancelled() => return Err(Error::forbidden()),
                        _ = root_cancel.cancelled() => return Err(Error::forbidden()),
                        _ = invocation_cancel.cancelled() => return Err(Error::forbidden()),
                        _ = tokio::time::sleep_until(deadline) => return Err(Error::forbidden()),
                        roles = self.role_projection.lock() => roles,
                    };
                    let fresh = loop {
                        if self.stop.is_cancelled()
                            || physical.lifetime.is_cancelled()
                            || root_cancel.is_cancelled()
                            || invocation_cancel.is_cancelled()
                            || tokio::time::Instant::now() >= deadline
                        {
                            return Err(Error::forbidden());
                        }
                        // Join the actual policy future. Racing its blocking Store acquisition
                        // against an outer select could detach a newly acquired file guard.
                        // Resource/currentness checks below refuse effects after any cutoff.
                        let result = a.reauthorize_resource_preparation().await;
                        match result {
                            Ok(fresh) => break fresh,
                            Err(
                                rvoip_uctp::v2::auth_dispatch::DispatchError::AuthorityUnavailable
                                | rvoip_uctp::v2::auth_dispatch::DispatchError::StorageUnavailable,
                            ) => {
                                tokio::select! { biased;
                                    _ = self.stop.cancelled() => return Err(Error::forbidden()),
                                    _ = physical.lifetime.cancelled() => return Err(Error::forbidden()),
                                    _ = root_cancel.cancelled() => return Err(Error::forbidden()),
                                    _ = invocation_cancel.cancelled() => return Err(Error::forbidden()),
                                    _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(20))) => {},
                                }
                                if tokio::time::Instant::now() >= deadline {
                                    return Err(Error::forbidden());
                                }
                            }
                            Err(_) => {
                                return Err(allocation_refused(
                                    "preflight-reauthorize",
                                    Error::forbidden(),
                                ))
                            }
                        }
                    };
                    if self.stop.is_cancelled()
                        || physical.lifetime.is_cancelled()
                        || root_cancel.is_cancelled()
                        || invocation_cancel.is_cancelled()
                        || tokio::time::Instant::now() >= deadline
                    {
                        return Err(Error::forbidden());
                    }
                    let a =
                        Arc::new(a.resume_replacement_allocation(fresh).map_err(|_| {
                            allocation_refused("preflight-resume", Error::forbidden())
                        })?);
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = &wait_probe {
                        probe.store(2, Ordering::Release);
                    }
                    (a, _roles)
                }
            };
            let authority = a
                .authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?;
            physical
                .validate(&a, authority)
                .map_err(|error| allocation_refused("preflight-physical", error))?;
            if self.stop.is_cancelled()
                || saved.handle.cancellation().is_cancelled()
                || a.invocation_cancellation().is_cancelled()
                || tokio::time::Instant::now() >= saved.deadline
            {
                return Err(Error::forbidden());
            }
            let previous = {
                saved
                    .allocated
                    .lock()
                    .map_err(|_| allocation_refused("preflight-owner-registry", Error::storage()))?
                    .clone()
            };
            if let Some(previous) = previous {
                if previous.intent != intent
                    || !previous
                        .physical
                        .upgrade()
                        .is_some_and(|p| Arc::ptr_eq(&p, &physical))
                {
                    return Err(Error::new(
                        "conflict",
                        "Replacement target belongs to another signed physical allocation",
                    ));
                }
                // This is only exact captured metadata replay, not a Core scene cut.
                drop(_roles);
                let record = self
                    .settle_original_allocation_reply(
                        a, &physical, &saved, &previous, credential, true,
                    )
                    .await
                    .map_err(|error| allocation_refused("replay-reply", error))?;
                return Ok(Reply {
                    record,
                    answered: false,
                });
            }
            if !saved.recorded.load(Ordering::Acquire) {
                return Err(Error::new(
                    "not-ready",
                    "Original Core preparation capture is not yet recorded",
                ));
            }
            // Validate the exact physical transport as pure metadata while the
            // Store retains one primary SQL guard through current plan and begin.
            // The signed Writer never waits on SQL or retries a durable boundary.
            let target_key_id = proof(&a)?.key_id().to_owned();
            let begun = {
                let store = self.media.store.clone();
                let a = a.clone();
                let receipt = saved.receipt.clone();
                let key = credential.to_owned();
                let transport = physical.binding.transport();
                let native_media = physical.binding.media().is_some();
                let webrtc = self.webrtc.get().is_some();
                store_call(move || {
                    store.try_begin_native_replacement_allocation_validated(
                        &receipt, &key, proof(&a)?, |read| {
                            let target = &read.source().context().target_transport;
                            let rtc = *target == connect_protocol::provisioning::EndpointTransport::Webrtc;
                            if if rtc {
                                transport != rvoip_core::Transport::WebSocket || !webrtc
                            } else {
                                !native_media || crate::canonical_runtime::core_transport(target)? != transport
                            } {
                                return Err(Error::new(
                                    "unsupported-media-profile",
                                    "Replacement target transport differs from the negotiated physical media transport",
                                ));
                            }
                            let context = read.source().context();
                            let actor = current_member(a.principal(), read.target())?;
                            let identity = IdentityBinding::from_authenticated(
                                a.principal(), IdentityId::from_string(context.identity_id.clone()),
                                chrono::Utc::now(),
                            ).map_err(|_| Error::forbidden())?;
                            let mut endpoint = EndpointBinding::provision(
                                identity.clone(), EndpointId::from_string(context.target_endpoint_id.clone()),
                                DeviceId::from_string(context.target_device_id.clone()),
                                crate::canonical_runtime::core_transport(&context.target_transport)?,
                                context.target_address.clone(),
                            ).map_err(|_| Error::forbidden())?;
                            endpoint.verify(&identity, "connect-store".into(), chrono::Utc::now())
                                .map_err(|_| Error::forbidden())?;
                            let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();
                            Ok((actor, endpoint, peer))
                        },
                    )
                })
                .await
                .map_err(|error| allocation_refused("validated-begin", error))?
            };
            match begun {
                Some((attempt, read, metadata)) => {
                    break (a, _roles, attempt, read, metadata, target_key_id);
                }
                None => {
                    tracing::debug!(target: "connect_service::canonical_replacement::allocation",
                        stage="validated-begin-deferred-primary-sql",
                        "Original allocation has no attempt; releasing admission before retry");
                    drop(_roles);
                    pending = Some(a);
                    deferred_sql = true;
                }
            }
        };
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        let rtc = read.source().context().target_transport
            == connect_protocol::provisioning::EndpointTransport::Webrtc;
        if let Err(error) = self.attach(
            read.target(),
            a.principal(),
            peer.clone(),
            authority.retained(),
        ) {
            drop(read);
            drop(_roles);
            self.record_allocation_uncertain(&saved, attempt, a)
                .await
                .map_err(|error| allocation_refused("capture-uncertain", error))?;
            return Err(allocation_refused("core-attach", error));
        }
        let target = TargetLease {
            authority: PeerNativeAuthority {
                actor,
                endpoint,
                may_prepare: true,
            },
            _read: read,
        };
        if !target._read.current() {
            drop(target);
            drop(_roles);
            self.record_allocation_uncertain(&saved, attempt, a)
                .await
                .map_err(|error| allocation_refused("capture-uncertain", error))?;
            return Err(Error::forbidden());
        }
        // One durable begin; never retry either Core allocation or retain.
        let webrtc = if rtc {
            match self
                .webrtc
                .get()
                .ok_or_else(Error::forbidden)
                .and_then(|host| host.reserve_physical(&peer, physical.lifetime.child_token()))
            {
                Ok(binding) => Some(binding),
                Err(error) => {
                    // Capacity refusal follows the original durable begin. Keep
                    // that attempt attributable and cancel only this target; a
                    // fresh offer must not silently retry the allocation effect.
                    drop(target);
                    saved.handle.cancel();
                    drop(_roles);
                    self.record_allocation_uncertain(&saved, attempt, a)
                        .await
                        .map_err(|error| allocation_refused("capture-uncertain", error))?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let allocated = if let Some(binding) = &webrtc {
            self.core.prepare_peer_webrtc_device_replacement(
                &saved.handle, peer, a.principal(), &target, binding.clone())
                .and_then(|prepared| prepared.retain(a.principal(), &target)
                    .map_err(|_| rvoip_core::orchestrator::participant_control::ParticipantControlError::Authority))
        } else {
            self.core.prepare_peer_native_device_replacement(
                &saved.handle, peer, a.principal(), &target, physical.binding.clone())
                .and_then(|prepared| prepared.retain(a.principal(), &target)
                    .map_err(|_| rvoip_core::orchestrator::participant_control::ParticipantControlError::Authority))
        };
        let valid = allocated.is_ok()
            && target._read.current()
            && self.native_connection_live(&saved.handle.intent().replacement)
            && self
                .core
                .connection_principal(&saved.handle.intent().replacement)
                .is_ok_and(|p| {
                    rvoip_core::staged_streams::reliable_data::same_credential(&p, a.principal())
                });
        drop(target);
        if !valid {
            saved.handle.cancel();
            drop(_roles);
            self.record_allocation_uncertain(&saved, attempt, a)
                .await
                .map_err(|error| allocation_refused("capture-uncertain", error))?;
            return Err(Error::new(
                "reconciliation-required",
                "Reserved native allocation outcome is uncertain",
            ));
        }
        let mut observation = match observed(&saved.handle, &saved.plan) {
            Ok(observation) => observation,
            Err(error) => {
                saved.handle.cancel();
                drop(_roles);
                self.record_allocation_uncertain(&saved, attempt, a)
                    .await
                    .map_err(|error| allocation_refused("capture-uncertain", error))?;
                return Err(allocation_refused("capture-observation", error));
            }
        };
        observation["allocated"] = serde_json::json!(true);
        observation["target_operation_id"] = serde_json::json!(intent.operation_id);
        observation["target_key_id"] = serde_json::json!(target_key_id);
        let receipt = match attempt.allocated(observation) {
            Ok(receipt) => Arc::new(receipt),
            Err((attempt, error)) => {
                saved.handle.cancel();
                drop(_roles);
                self.record_allocation_uncertain(&saved, attempt, a)
                    .await
                    .map_err(|error| allocation_refused("capture-uncertain", error))?;
                return Err(allocation_refused("capture-validation", error));
            }
        };
        let allocated = Arc::new(AllocatedReplacement {
            receipt,
            physical: Arc::downgrade(&physical),
            webrtc,
            intent,
            recorded: AtomicBool::new(false),
            started: std::sync::Mutex::new(None),
            failed_start: std::sync::Mutex::new(None),
        });
        let installed = match saved.allocated.lock() {
            Ok(mut slot) => {
                *slot = Some(allocated.clone());
                true
            }
            Err(_) => false,
        };
        drop(_roles);
        if !installed {
            // This capture has no registry owner to recover it. Preserve the
            // original historical outcome before reporting failed publication.
            saved.handle.cancel();
            let retirement = retire_replacement_authority(a);
            self.record_allocated_replacement(&allocated)
                .await
                .map_err(|error| allocation_refused("capture", error))?;
            retirement?;
            return Err(allocation_refused(
                "capture-owner-registry",
                Error::storage(),
            ));
        }
        let record = self
            .settle_original_allocation_reply(a, &physical, &saved, &allocated, credential, false)
            .await?;
        Ok(Reply {
            record,
            answered: false,
        })
    }
    /// The same immutable Core capture already has an owner. Release both
    /// original application aliases while recording it, then admit the same
    /// signed command only for its reply. This cannot begin another effect.
    async fn settle_original_allocation_reply(
        &self,
        a: Arc<AuthorizedEnvelope>,
        physical: &Physical,
        saved: &PreparedReplacement,
        allocated: &AllocatedReplacement,
        credential: &str,
        replay: bool,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        let original = Arc::try_unwrap(a).map_err(|_| {
            allocation_refused(
                "capture-owner",
                Error::new("conflict", "Original allocation invocation is still shared"),
            )
        })?;
        let (a, cutoff) = match original.into_replacement_allocation_wait(saved.deadline) {
            Ok(value) => value,
            Err(_) => {
                // The consuming conversion has retired both original guards
                // even when cancellation/expiry rejects further reply rights.
                self.record_allocated_replacement(allocated).await?;
                return Err(allocation_refused("capture-suspend", Error::forbidden()));
            }
        };
        #[cfg(feature = "test-support")]
        self.pause_original_allocation_capture_for_test(saved, &a, cutoff)
            .await;
        // This is historical attribution, which may still settle after the
        // original reply closes. The registry retains the same opaque capture.
        self.record_allocated_replacement(allocated)
            .await
            .map_err(|error| allocation_refused("capture", error))?;
        #[cfg(feature = "test-support")]
        if let Some(probe) = self.media.take_preparation_custody_probe_for_test(
            &saved.origin.request().operation_id,
            crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase::AllocationReply,
        ) {
            let expected = &saved.plan["expected_preparation"];
            if let (Some(target), Some(source)) = (
                expected["retained"].as_str().and_then(|id| self.media.origin(&ConnectionId::from_string(id.to_owned()))),
                expected["source"].as_str().and_then(|id| self.media.origin(&ConnectionId::from_string(id.to_owned()))),
            ) { probe.set_original_route(target, source); }
            probe.pause(&self.stop, &a.invocation_cancellation(), cutoff, None).await;
        }
        let root_cancel = saved.handle.cancellation();
        let invocation_cancel = a.invocation_cancellation();
        let current = || {
            !self.stop.is_cancelled()
                && !physical.lifetime.is_cancelled()
                && !root_cancel.is_cancelled()
                && !invocation_cancel.is_cancelled()
                && tokio::time::Instant::now() < cutoff
        };
        while current() {
            // Join the actual fresh policy acquisition even if cancellation
            // wins during its blocking Store work. Recheck before any reply.
            match a.reauthorize_resource_preparation().await {
                Ok(fresh) if current() => {
                    // The reply slot is still resource-only. Validate the actual
                    // captured allocation under the same fresh proof before
                    // restoring any result authority. A durable cancellation
                    // may already be visible before Core propagates its token.
                    let store = self.media.store.clone();
                    let receipt = allocated.receipt.clone();
                    let credential = credential.to_owned();
                    let checked = fresh.clone();
                    let observed = self
                        .owned_store_call(move || {
                            let proof = checked
                                .authority::<StoreInvocationAuthority>()
                                .and_then(|authority| authority.peer_guard())
                                .ok_or_else(Error::forbidden)?;
                            if replay {
                                store.replay_native_replacement_allocation_with_peer(
                                    &receipt,
                                    &credential,
                                    proof,
                                )
                            } else {
                                store.native_replacement_allocation_reply_with_peer(
                                    &receipt,
                                    &credential,
                                    proof,
                                )
                            }
                        })
                        .await;
                    match observed {
                        Ok(record) if current() => {
                            let authority = fresh
                                .authority::<StoreInvocationAuthority>()
                                .ok_or_else(Error::forbidden)?;
                            physical.validate(&a, authority).map_err(|error| {
                                allocation_refused("capture-reply-physical", error)
                            })?;
                            if !current() {
                                break;
                            }
                            a.resume_replacement_allocation(fresh).map_err(|_| {
                                allocation_refused("capture-resume", Error::forbidden())
                            })?;
                            return Ok(record);
                        }
                        Ok(_) => break,
                        Err(error)
                            if error.code == "authority-busy"
                                && error.message == "Current state is busy" =>
                        {
                            drop(fresh);
                        }
                        Err(error) if !replay && error.code == "unauthorized" => {
                            // The target signer was already freshly admitted.
                            // Withdrawal of the original source denies this result.
                            return Err(allocation_refused(
                                "capture-reply-state",
                                Error::forbidden(),
                            ));
                        }
                        Err(error) => return Err(allocation_refused("capture-reply-state", error)),
                    }
                }
                Err(
                    rvoip_uctp::v2::auth_dispatch::DispatchError::AuthorityUnavailable
                    | rvoip_uctp::v2::auth_dispatch::DispatchError::StorageUnavailable,
                ) => {}
                _ => break,
            }
            tokio::select! { biased;
                _ = self.stop.cancelled() => {},
                _ = physical.lifetime.cancelled() => {},
                _ = root_cancel.cancelled() => {},
                _ = invocation_cancel.cancelled() => {},
                _ = tokio::time::sleep_until(cutoff.min(tokio::time::Instant::now()+Duration::from_millis(20))) => {},
            }
        }
        Err(allocation_refused(
            "capture-reply-authority",
            Error::forbidden(),
        ))
    }

    #[cfg(feature = "test-support")]
    async fn pause_original_allocation_capture_for_test(
        &self,
        saved: &PreparedReplacement,
        a: &AuthorizedEnvelope,
        cutoff: tokio::time::Instant,
    ) {
        use crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase;
        if let Some(probe) = self.media.take_preparation_custody_probe_for_test(
            &saved.origin.request().operation_id,
            PreparationCustodyPhase::AllocationCapture,
        ) {
            probe
                .pause(&self.stop, &a.invocation_cancellation(), cutoff, None)
                .await;
        }
    }

    async fn record_allocation_uncertain(
        &self,
        saved: &PreparedReplacement,
        attempt: connect_store::NativeReplacementAllocationAttempt,
        a: Arc<AuthorizedEnvelope>,
    ) -> Result<()> {
        let capture = Arc::new(attempt.uncertain());
        // The watcher retains this exact no-IO capture across any failed first
        // settlement. There is exactly one allocation effect for this root.
        let retirement = retire_replacement_authority(a);
        saved
            .failed_allocation
            .set(capture.clone())
            .map_err(|_| Error::storage())?;
        // The opaque receipt owns no media/read. Normal unique ownership retires
        // both guards first. An unexpected retained alias can only delay this
        // bounded historical retry; it cannot discard or recreate the capture.
        let store = self.media.store.clone();
        let first = capture.clone();
        match self
            .owned_store_call(move || store.native_replacement_allocation_capture_uncertain(&first))
            .await
        {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) =>
            {
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    self.owned_store_call(move || {
                        store.native_replacement_allocation_capture_uncertain(&capture)
                    })
                })
                .await?;
            }
            Err(error) => return Err(error),
        }
        retirement
    }
    pub(super) async fn record_allocated_replacement(
        &self,
        allocated: &AllocatedReplacement,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        // Captured metadata can be recorded after cancellation/revocation.
        // Repeated recording never invokes Core or recreates a physical route.
        let record = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = allocated.receipt.clone();
            self.owned_store_call(move || store.record_native_replacement_allocated(&receipt))
        })
        .await?;
        allocated.recorded.store(true, Ordering::Release);
        Ok(record)
    }
}
