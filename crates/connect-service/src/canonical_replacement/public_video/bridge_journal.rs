//! Signed optional pre-F bridge metadata. Parent activation wiring installs
//! the real runtime observer; this module NEVER enables a B writer/route.
use super::super::readiness::admission;
use super::*;
use crate::{canonical_native::Reply, canonical_runtime::Physical, StoreInvocationAuthority};
use connect_protocol::webrtc_public_video_bridge as bridge;
use connect_store::NativeReplacementPublicVideoBridgeObservation;
use rvoip_uctp::v2::auth_dispatch::{AuthorizedEnvelope, ResourceActivationAuthority};

pub(super) struct Journal {
    observation: OnceLock<Arc<dyn NativeReplacementPublicVideoBridgeObservation>>,
    decoded_recorded: AtomicBool,
    changed: tokio::sync::Notify,
}
impl Journal {
    pub(super) fn new() -> Self {
        Self {
            observation: OnceLock::new(),
            decoded_recorded: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        }
    }
}
impl Preparation {
    /// Activation owner must install this BEFORE any feeding/Ready observation.
    /// It must retain exact original Core bridge + B writer and revalidate
    /// cancellation, original cutoff and current authority through commit.
    /// There is deliberately no default/native/normal activated fallback.
    pub(in crate::canonical_replacement) fn own_bridge_journal_observation(
        &self,
        root: &Arc<PreparedReplacement>,
        observation: Arc<dyn NativeReplacementPublicVideoBridgeObservation>,
    ) -> Result<()> {
        if !std::ptr::eq(self, root.public_video.as_ref())
            || !self.required()
            || bridge::OptIn::from_request(root.origin.request())?.is_none()
            || self.reservation()?.operation_record_id() != root.origin.record_id()
            || self.cut.get().is_some()
            || self.warm.get().is_some()
            || self.ready.get().is_some()
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
        {
            return Err(Error::forbidden());
        }
        if let Some(old) = self.bridge_journal.observation.get() {
            return if Arc::ptr_eq(old, &observation) {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        match self.bridge_journal.observation.set(observation) {
            Ok(()) => Ok(()),
            Err(observation) => {
                if self
                    .bridge_journal
                    .observation
                    .get()
                    .is_some_and(|old| Arc::ptr_eq(old, &observation))
                {
                    Ok(())
                } else {
                    Err(Error::forbidden())
                }
            }
        }
    }
    pub(in crate::canonical_replacement) fn require_bridge_decoded_before_cut(
        &self,
        root: &Arc<PreparedReplacement>,
    ) -> Result<()> {
        if bridge::OptIn::from_request(root.origin.request())?.is_none() {
            return Ok(());
        }
        if !std::ptr::eq(self, root.public_video.as_ref())
            || self.bridge_journal.observation.get().is_none()
            || !self.bridge_decoded_recorded()
        {
            return Err(Error::new(
                "not-ready",
                "Explicit pre-cut B bridge is not decoded",
            ));
        }
        // This proves only completion of the negotiated bridge prerequisite.
        // Existing current Core/Store/F/physical/effect fences remain mandatory.
        Ok(())
    }
    pub(in crate::canonical_replacement) async fn wait_for_bridge_decoded(
        &self,
        stop: &tokio_util::sync::CancellationToken,
        cancel: &tokio_util::sync::CancellationToken,
        until: tokio::time::Instant,
    ) -> Result<()> {
        if !self.required() || self.bridge_journal.observation.get().is_none() {
            return Err(Error::forbidden());
        }
        loop {
            // Register BEFORE the flag load: notify_waiters retains no permit.
            let notified = self.bridge_journal.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if stop.is_cancelled() || cancel.is_cancelled() || tokio::time::Instant::now() >= until
            {
                return Err(Error::forbidden());
            }
            if self.bridge_journal.decoded_recorded.load(Ordering::Acquire) {
                return Ok(());
            }
            tokio::select! {biased;
                _=stop.cancelled()=>return Err(Error::forbidden()),
                _=cancel.cancelled()=>return Err(Error::forbidden()),
                _=tokio::time::sleep_until(until)=>return Err(Error::forbidden()),
                _=&mut notified=>{},
            }
        }
    }
    /// A bridge journal observation is NOT post-F ready and cannot satisfy the
    /// existing p.ready, cut_recorded, native submission or Activated gates.
    pub(in crate::canonical_replacement) fn bridge_decoded_recorded(&self) -> bool {
        self.bridge_journal.decoded_recorded.load(Ordering::Acquire)
    }
}
impl Runtime {
    /// One metadata attempt, no retry/effect/feeding. The existing bounded
    /// activation owner may retry ONLY this same effect-free SQL observation.
    pub(in crate::canonical_replacement) async fn record_pre_cut_public_video_bridge_feeding(
        &self,
        root: &Arc<PreparedReplacement>,
    ) -> Result<()> {
        if self.stop.is_cancelled()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
        {
            return Err(Error::forbidden());
        }
        let observation = root
            .public_video
            .bridge_journal
            .observation
            .get()
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let r = root.public_video.reservation()?.clone();
        let store = self.media.store.clone();
        store_call(move || {
            store.record_native_replacement_public_video_bridge_feeding(&r, observation.as_ref())
        })
        .await?;
        Ok(())
    }
    pub(crate) async fn replacement_public_video_bridge_ready(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;
        if physical.binding.transport() != rvoip_core::Transport::WebSocket {
            return Err(Error::forbidden());
        }
        let intent = crate::canonical_native::intent(a.envelope())?;
        let marker: bridge::ReadyMarker =
            serde_json::from_value(intent.offer["replacement_preparation"].clone())
                .map_err(|_| Error::forbidden())?;
        marker.validate_shape()?;
        let root = self
            .replacements
            .lock()
            .map_err(|_| Error::storage())?
            .get(&marker.operation_record_id)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        if !root.public_video.required()
            || bridge::OptIn::from_request(root.origin.request())?.is_none()
        {
            return Err(Error::forbidden());
        }
        let allocated = root
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(Error::forbidden)?;
        if allocated.webrtc.is_none()
            || allocated
                .physical
                .upgrade()
                .is_none_or(|original| !Arc::ptr_eq(&original, &physical))
        {
            return Err(Error::forbidden());
        }
        let started = allocated
            .started
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(Error::forbidden)?;
        let observation = root
            .public_video
            .bridge_journal
            .observation
            .get()
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let historical = root.public_video.bridge_decoded_recorded();
        let original = Arc::try_unwrap(a)
            .map_err(|_| Error::new("conflict", "Original bridge Ready command remains aliased"))?;
        // Drop BOTH original signed Readers before fresh admission or retry.
        let (resource, until) = original
            .into_replacement_ready_wait((!historical).then_some(root.deadline))
            .map_err(admission::authority_error)?;
        let resource = Arc::new(resource);
        let invocation = resource.invocation_cancellation();
        let cancel = if historical {
            invocation.clone()
        } else {
            root.handle.cancellation()
        };
        let cancellation = [&self.stop, &physical.lifetime, &cancel, &invocation];
        let (record, current) =
            admission::capture(until, cancellation, "pre-cut-bridge-ready", || {
                let attempt: admission::Attempt<'_, (_, Arc<ResourceActivationAuthority>)> =
                    Box::pin(async {
                        let current = resource
                            .reauthorize_resource_preparation()
                            .await
                            .map_err(admission::authority_error)?;
                        if !resource.resource_activation_current(&current)
                            || cancel.is_cancelled()
                            || self.stop.is_cancelled()
                            || physical.lifetime.is_cancelled()
                            || tokio::time::Instant::now() >= until
                        {
                            return Err(Error::forbidden());
                        }
                        let authority = current
                            .authority::<StoreInvocationAuthority>()
                            .ok_or_else(Error::forbidden)?;
                        physical.validate(&resource, authority)?;
                        let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                        if root.public_video.bridge_decoded_recorded() {
                            let record = self
                                .media
                                .store
                                .replay_native_replacement_public_video_bridge_ready_with_peer(
                                    &started.receipt,
                                    credential,
                                    proof,
                                )?;
                            return Ok((record, current));
                        }
                        if historical || root.handle.is_committed() {
                            return Err(Error::forbidden());
                        }
                        let store = self.media.store.clone();
                        let signed = current.clone();
                        let started = started.receipt.clone();
                        let reservation = root.public_video.reservation()?.clone();
                        let credential = credential.to_owned();
                        let observation = observation.clone();
                        // The same owned blocking Store task is joined before dropping
                        // signed custody or reporting cancellation; no SQL commit on
                        // the Tokio/media worker, no new effect or deadline.
                        let snapshot = store_call(move || {
                            let authority = signed
                                .authority::<StoreInvocationAuthority>()
                                .ok_or_else(Error::forbidden)?;
                            let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                            store.record_native_replacement_public_video_bridge_ready_with_peer(
                                &started,
                                &reservation,
                                &credential,
                                proof,
                                observation.as_ref(),
                            )
                        })
                        .await?;
                        if !resource.resource_activation_current(&current) {
                            return Err(Error::forbidden());
                        }
                        let mut record = snapshot.operation;
                        record.result = None;
                        record.problem = None;
                        record.effects.clear();
                        Ok((record, current))
                    });
                attempt
            })
            .await?;
        // The journal committed under actual signed/native custody. Release the
        // capture Reader BEFORE waking continuation. The reply read is separate
        // and grants no new bridge/Cut/native transition after that boundary.
        if !resource.resource_activation_current(&current) {
            return Err(Error::forbidden());
        }
        drop(current);
        root.public_video
            .bridge_journal
            .decoded_recorded
            .store(true, Ordering::Release);
        root.public_video.bridge_journal.changed.notify_waiters();
        let _captured_record = record;
        let (record, current) =
            admission::capture(until, cancellation, "pre-cut-bridge-ready-reply", || {
                let attempt: admission::Attempt<'_, (_, Arc<ResourceActivationAuthority>)> =
                    Box::pin(async {
                        let current = resource
                            .reauthorize_resource_preparation()
                            .await
                            .map_err(admission::authority_error)?;
                        if !resource.resource_activation_current(&current) {
                            return Err(Error::forbidden());
                        }
                        let authority = current
                            .authority::<StoreInvocationAuthority>()
                            .ok_or_else(Error::forbidden)?;
                        physical.validate(&resource, authority)?;
                        let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
                        let record = self
                            .media
                            .store
                            .replay_native_replacement_public_video_bridge_ready_with_peer(
                                &started.receipt,
                                credential,
                                proof,
                            )?;
                        Ok((record, current))
                    });
                attempt
            })
            .await?;
        if !resource.resource_activation_current(&current) {
            return Err(Error::forbidden());
        }
        resource
            .install_replacement_ready_reply_authority(&current)
            .map_err(admission::authority_error)?;
        Ok(Reply {
            record,
            answered: false,
        })
    }
}
