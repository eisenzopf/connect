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
    Transport,
};
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
use std::sync::Weak;

pub(super) struct AllocatedReplacement {
    pub receipt: Arc<connect_store::NativeReplacementAllocated>,
    pub physical: Weak<Physical>,
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
impl Runtime {
    pub(crate) async fn allocate_native_replacement(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        let _roles = self.role_projection.lock().await;
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;
        let intent = crate::canonical_native::intent(a.envelope())?;
        let record = intent.offer["replacement_preparation"]["operation_record_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        let saved = {
            self.replacements
                .lock()
                .map_err(|_| Error::storage())?
                .get(record)
                .cloned()
                .ok_or_else(|| {
                    Error::new(
                        "not-ready",
                        "Original owned replacement preparation is not live",
                    )
                })?
        };
        let previous = {
            saved
                .allocated
                .lock()
                .map_err(|_| Error::storage())?
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
            self.record_allocated_replacement_with_peer(&previous, a.clone())
                .await?;
            let store = self.media.store.clone();
            let a = a.clone();
            let key = credential.to_owned();
            let receipt = previous.receipt.clone();
            let record = store_call(move || {
                store.replay_native_replacement_allocation_with_peer(&receipt, &key, proof(&a)?)
            })
            .await?;
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
        let read = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let a = a.clone();
            let receipt = saved.receipt.clone();
            let key = credential.to_owned();
            store_call(move || {
                store.native_replacement_allocation_read_with_peer(&receipt, &key, proof(&a)?)
            })
        })
        .await?;
        // Only the preflight read is retried. Durable begin and Core allocation
        // each execute once, even when recording or the response is lost.
        let (attempt, read) = {
            let store = self.media.store.clone();
            let a = a.clone();
            let receipt = saved.receipt.clone();
            let key = credential.to_owned();
            store_call(move || {
                store.begin_native_replacement_allocation_with_read(
                    &receipt,
                    &key,
                    proof(&a)?,
                    read,
                )
            })
            .await?
        };
        let context = read.source().context();
        let actor = current_member(a.principal(), read.target())?;
        let identity = IdentityBinding::from_authenticated(
            a.principal(),
            IdentityId::from_string(context.identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(|_| Error::forbidden())?;
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::from_string(context.target_endpoint_id.clone()),
            DeviceId::from_string(context.target_device_id.clone()),
            Transport::Quic,
            context.target_address.clone(),
        )
        .map_err(|_| Error::forbidden())?;
        endpoint
            .verify(&identity, "connect-store".into(), chrono::Utc::now())
            .map_err(|_| Error::forbidden())?;
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();
        self.attach(
            read.target(),
            a.principal(),
            peer.clone(),
            authority.retained(),
        )?;
        let target = TargetLease {
            authority: PeerNativeAuthority {
                actor,
                endpoint,
                may_prepare: true,
            },
            _read: read,
        };
        if !target._read.current() {
            let store = self.media.store.clone();
            let a = a.clone();
            store_call(move || {
                store.native_replacement_allocation_uncertain_with_peer(attempt, proof(&a)?)
            })
            .await?;
            return Err(Error::forbidden());
        }
        // One durable begin; never retry either Core allocation or retain.
        let allocated=self.core.prepare_peer_native_device_replacement(&saved.handle,peer,a.principal(),&target,physical.binding.clone())
            .and_then(|prepared|prepared.retain(a.principal(),&target).map_err(|_|rvoip_core::orchestrator::participant_control::ParticipantControlError::Authority));
        let valid = allocated.is_ok()
            && target._read.current()
            && self
                .native
                .is_connection_live(&saved.handle.intent().replacement)
            && self
                .core
                .connection_principal(&saved.handle.intent().replacement)
                .is_ok_and(|p| {
                    rvoip_core::staged_streams::reliable_data::same_credential(&p, a.principal())
                });
        drop(target);
        if !valid {
            saved.handle.cancel();
            let store = self.media.store.clone();
            let a = a.clone();
            store_call(move || {
                store.native_replacement_allocation_uncertain_with_peer(attempt, proof(&a)?)
            })
            .await?;
            return Err(Error::new(
                "reconciliation-required",
                "Reserved native allocation outcome is uncertain",
            ));
        }
        let mut observation = observed(&saved.handle, &saved.plan)?;
        observation["allocated"] = serde_json::json!(true);
        observation["target_operation_id"] = serde_json::json!(intent.operation_id);
        observation["target_key_id"] = serde_json::json!(proof(&a)?.key_id());
        let receipt = Arc::new(attempt.allocated(observation)?);
        let allocated = Arc::new(AllocatedReplacement {
            receipt,
            physical: Arc::downgrade(&physical),
            intent,
            recorded: AtomicBool::new(false),
            started: std::sync::Mutex::new(None),
            failed_start: std::sync::Mutex::new(None),
        });
        *saved.allocated.lock().map_err(|_| Error::storage())? = Some(allocated.clone());
        let record = self
            .record_allocated_replacement_with_peer(&allocated, a.clone())
            .await?;
        Ok(Reply {
            record,
            answered: false,
        })
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
