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
        let _roles = self.role_projection.try_lock().map_err(|_| {
            allocation_refused(
                "preflight-role",
                Error::new(
                    "not-ready",
                    "Replacement scene coordinator is busy before original effect",
                ),
            )
        })?;
        physical
            .validate(&a, authority)
            .map_err(|error| allocation_refused("preflight-physical", error))?;
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
            self.record_allocated_replacement_with_peer(&previous, a.clone())
                .await
                .map_err(|error| allocation_refused("replay-capture", error))?;
            let store = self.media.store.clone();
            let a = a.clone();
            let key = credential.to_owned();
            let receipt = previous.receipt.clone();
            let record = store_call(move || {
                store.replay_native_replacement_allocation_with_peer(&receipt, &key, proof(&a)?)
            })
            .await
            .map_err(|error| allocation_refused("replay-read", error))?;
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
        let (attempt, read, (actor, endpoint, peer)) = {
            let store = self.media.store.clone();
            let a = a.clone();
            let receipt = saved.receipt.clone();
            let key = credential.to_owned();
            let transport = physical.binding.transport();
            let native_media = physical.binding.media().is_some();
            let webrtc = self.webrtc.get().is_some();
            store_call(move || {
                store.begin_native_replacement_allocation_validated(
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
        let rtc = read.source().context().target_transport
            == connect_protocol::provisioning::EndpointTransport::Webrtc;
        if let Err(error) = self.attach(
            read.target(),
            a.principal(),
            peer.clone(),
            authority.retained(),
        ) {
            drop(read);
            self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
            self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
                    self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
            self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
                self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
                self.record_allocation_uncertain_with_peer(attempt, a.clone())
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
        if !installed {
            // A valid physical capture already exists. Preserve that actual
            // historical fact even if registry publication must fail closed.
            saved.handle.cancel();
        }
        let record = self
            .record_allocated_replacement_with_peer(&allocated, a.clone())
            .await
            .map_err(|error| allocation_refused("capture", error))?;
        if !installed {
            return Err(allocation_refused(
                "capture-owner-registry",
                Error::storage(),
            ));
        }
        Ok(Reply {
            record,
            answered: false,
        })
    }
    async fn record_allocation_uncertain_with_peer(
        &self,
        attempt: connect_store::NativeReplacementAllocationAttempt,
        a: Arc<AuthorizedEnvelope>,
    ) -> Result<()> {
        let attempt = Arc::new(attempt);
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let attempt = attempt.clone();
            let a = a.clone();
            // The existing owned call registry retains the actual blocking
            // handle if its waiter is dropped. Only this original historical
            // receipt repeats; no Core allocation or transport is replayed.
            self.owned_store_call(move || {
                store.native_replacement_allocation_uncertain_with_peer(&attempt, proof(&a)?)
            })
        })
        .await
    }
    async fn record_allocated_replacement_with_peer(
        &self,
        allocated: &AllocatedReplacement,
        a: Arc<AuthorizedEnvelope>,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        let record = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = allocated.receipt.clone();
            let a = a.clone();
            store_call(move || {
                store.record_native_replacement_allocated_with_peer(&receipt, proof(&a)?)
            })
        })
        .await?;
        allocated.recorded.store(true, Ordering::Release);
        Ok(record)
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
            store_call(move || store.record_native_replacement_allocated(&receipt))
        })
        .await?;
        allocated.recorded.store(true, Ordering::Release);
        Ok(record)
    }
}
