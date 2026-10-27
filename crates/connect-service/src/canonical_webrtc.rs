//! Protected WebRTC signaling composition. An SDP answer is not media readiness.
use crate::{
    canonical_native::Reply,
    canonical_runtime::{Physical, Runtime},
    Service, StoreInvocationAuthority,
};
use async_trait::async_trait;
use connect_protocol::{
    provisioning::EndpointTransport, transitions::OperationState, Error, Result,
};
use connect_store::{
    NativeMediaOrigin, NativeMediaRead, NativeOfferLease, NativeOfferObservation,
    NativeOfferRuntimeObservation, Store,
};
use rvoip_core::{
    adapter::{ConnectionAdapter, EndReason},
    ids::{ConnectionId, ConversationId, SessionId},
    orchestrator::peer_native::{PeerNativeAuthority, PeerNativeAuthorityLease, PeerWebRtcIntent},
};
use rvoip_uctp::v2::{auth_dispatch::AuthorizedEnvelope, payloads::ConnectionAnswer};
use rvoip_webrtc::canonical::{CanonicalWebRtcAdapter, WebRtcPhysicalBinding};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
mod admission;
mod streams;

/// One original Store-issued attempt; the host must take custody synchronously.
/// It must not publish media or report operation success from SDP/DTLS alone.
pub struct Preparation {
    pub store: Arc<Store>,
    pub origin: Arc<NativeMediaOrigin>,
    pub attempt: NativeOfferLease,
    /// Bounded originating invocation custody; release before idle/cleanup.
    pub read: Arc<NativeMediaRead>,
    pub physical: CancellationToken,
    pub binding: Arc<WebRtcPhysicalBinding>,
    pub deadline: Instant,
}
#[async_trait]
pub trait Attempt: Send + Sync {
    fn original_attempt(&self) -> &NativeOfferLease;
    /// Release every retained originating reader after answer/refusal delivery.
    fn release_invocation(&self) {}
    /// Actual engine answer with selected stream codecs. The service checks the
    /// current canonical negotiation plan before admitting the signed write.
    async fn answer(&self, deadline: Instant) -> Result<ConnectionAnswer>;
    /// Joins the original worker; a cancelled observer must not detach cleanup.
    async fn shutdown(&self) -> Result<()>;
    /// Observes spontaneous transport retirement without requesting a new close.
    async fn closed(&self) -> Result<()>;
}
pub trait Host: Send + Sync {
    fn adapter(&self) -> Arc<CanonicalWebRtcAdapter>;
    /// Reserve deployment capacity without starting physical RTC resources.
    fn reserve_physical(
        &self,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: CancellationToken,
    ) -> Result<Arc<WebRtcPhysicalBinding>>;
    /// Return an owner even on allocation refusal so the original lease remains
    /// available for attributed cleanup. This must not start another attempt.
    fn start(&self, preparation: Preparation) -> Arc<dyn Attempt>;
}
// Freeze the adapter installed in Core even when a trusted host delegates its
// construction to another component.
struct Installed {
    host: Arc<dyn Host>,
    adapter: Arc<CanonicalWebRtcAdapter>,
}
impl Host for Installed {
    fn adapter(&self) -> Arc<CanonicalWebRtcAdapter> {
        self.adapter.clone()
    }
    fn reserve_physical(
        &self,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: CancellationToken,
    ) -> Result<Arc<WebRtcPhysicalBinding>> {
        self.host.reserve_physical(peer, physical)
    }
    fn start(&self, preparation: Preparation) -> Arc<dyn Attempt> {
        self.host.start(preparation)
    }
}
impl Service {
    /// Trusted startup composition only, before the first physical admission.
    /// This enables SDP control, not a media capability advertisement.
    pub fn install_canonical_webrtc(&self, host: Arc<dyn Host>) -> Result<()> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let closed = runtime
            .host_configuration_closed
            .lock()
            .map_err(|_| Error::storage())?;
        if *closed || runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if runtime.webrtc.get().is_some() {
            return Err(Error::new("conflict", "WebRTC host already installed"));
        }
        let adapter = host.adapter();
        runtime
            .core
            .register(adapter.clone())
            .map_err(|_| Error::forbidden())?;
        runtime
            .webrtc
            .set(Arc::new(Installed { host, adapter }))
            .map_err(|_| Error::new("conflict", "WebRTC host already installed"))
    }
}
#[derive(Default)]
struct Owned {
    lease: Option<NativeOfferLease>,
    attempt: Option<Arc<dyn Attempt>>,
    binding: Option<Arc<WebRtcPhysicalBinding>>,
    id: Option<ConnectionId>,
    reserved: bool,
    entry: Option<Arc<crate::canonical_media::Connection>>,
    controller: Option<Arc<rvoip_core::staged_streams::StreamController>>,
    peer: Option<Arc<rvoip_core::peer_admission::PeerAdmission>>,
    deadline: Option<Instant>,
    correlation: Option<String>,
}
impl Owned {
    fn lease(&self) -> Result<&NativeOfferLease> {
        self.attempt
            .as_ref()
            .map(|a| a.original_attempt())
            .or(self.lease.as_ref())
            .ok_or_else(Error::storage)
    }
}
struct Closed<'a>(&'a Owned);
impl NativeOfferRuntimeObservation for Closed<'_> {
    fn observe(&self) -> Result<NativeOfferObservation> {
        let a = self.0.lease()?;
        Ok(NativeOfferObservation::Retired {
            runtime_connection_id: a.runtime_connection_id().into(),
            generation: a.generation(),
        })
    }
}
impl Runtime {
    pub(crate) async fn webrtc_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) {
        let mut owned = Owned::default();
        let outcome = Box::pin(self.webrtc_start(&a, &physical, &credential, &mut owned)).await;
        let handler_finished = a.invocation_cancellation();
        let succeeded = outcome.is_ok();
        let _ = tx.send(outcome);
        if let Some(attempt) = &owned.attempt {
            attempt.release_invocation();
        }
        // The dispatcher cancels this child when the original handler returns.
        // Release our Arc and every host read before observing actual resources.
        drop(a);
        if let Some(attempt) = &owned.attempt {
            if succeeded {
                let deadline = owned.deadline.unwrap_or_else(Instant::now);
                let finished = tokio::select! { biased;
                    _=self.stop.cancelled()=>false,
                    _=physical.lifetime.cancelled()=>false,
                    _=tokio::time::sleep_until(deadline)=>false,
                    _=handler_finished.cancelled()=>true,
                };
                if finished {
                    match Box::pin(self.webrtc_admit(&owned, physical.clone(), &credential)).await {
                        Ok(ready) => loop {
                            tokio::select! { biased;
                                _=self.stop.cancelled()=>break,
                                _=physical.lifetime.cancelled()=>break,
                                _=attempt.closed()=>break,
                                _=tokio::time::sleep(Duration::from_millis(100))=>{},
                            }
                            match ready.current() {
                                Ok(()) => {}
                                Err(e) if e.code == "authority-busy" => continue,
                                Err(_) => break,
                            }
                            // Project the latest durable Session role under its
                            // own bounded current read; no offer reader survives
                            // into this idle loop or becomes role authority.
                            if let Some(id) = &owned.id {
                                self.reconcile_connection_role(id).await;
                            }
                        },
                        Err(e) => {
                            tracing::debug!(code=%e.code,"RTC original media admission refused")
                        }
                    }
                }
            }
            // The original owner confirms actual RTC close. A missing confirmation
            // cannot produce a durable retirement fact or release its owned capacity.
            if attempt.shutdown().await.is_err() {
                return;
            }
        }
        if let Some(binding) = &owned.binding {
            if binding.shutdown().await.is_err() {
                return;
            }
        }
        if let Some(id) = &owned.id {
            let Some(host) = self.webrtc.get() else {
                return;
            };
            if owned.reserved
                && host
                    .adapter()
                    .end(id.clone(), EndReason::Normal)
                    .await
                    .is_err()
            {
                return;
            }
            if owned.reserved && host.adapter().is_connection_live(id) {
                return;
            }
            self.media.remove(id);
        }
        if owned.lease().is_err() {
            return;
        }
        let owned = Arc::new(owned);
        loop {
            let store = self.media.store.clone();
            let owner = owned.clone();
            let settled = tokio::task::spawn_blocking(move || {
                store.settle_native_offer_cleanup(owner.lease()?, &Closed(&owner))
            })
            .await;
            if matches!(settled, Ok(Ok(()))) || self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn webrtc_start(
        &self,
        a: &Arc<AuthorizedEnvelope>,
        physical: &Physical,
        credential: &str,
        owned: &mut Owned,
    ) -> Result<Reply> {
        let host = self.webrtc.get().ok_or_else(|| {
            Error::new(
                "unsupported-media-profile",
                "Canonical WebRTC media host is not configured",
            )
        })?;
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(a, authority)?;
        let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
        let intent = crate::canonical_native::intent(a.envelope())?;
        if physical.binding.transport() != rvoip_core::Transport::WebSocket
            || intent.endpoint_transport()? != EndpointTransport::Webrtc
        {
            return Err(Error::forbidden());
        }
        let record = self.media.store.prepare_native_offer_with_peer(
            credential,
            proof,
            Some(EndpointTransport::Webrtc),
        )?;
        if record.state != OperationState::Accepted {
            return Ok(Reply {
                record,
                answered: false,
            });
        }
        let Some((lease, record)) = self
            .media
            .store
            .begin_native_offer_with_peer(credential, proof)?
        else {
            return Ok(Reply {
                record: self.media.store.prepare_native_offer_with_peer(
                    credential,
                    proof,
                    Some(EndpointTransport::Webrtc),
                )?,
                answered: false,
            });
        };
        // Retain the non-cloneable original lease before the first suspension.
        owned.lease = Some(lease);
        let remaining =
            (owned.lease()?.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = Instant::now() + Duration::from_secs(remaining.min(30));
        owned.deadline = Some(deadline);
        owned.correlation = Some(a.envelope().id().to_owned());
        // Origin issuance uses the exact current originating invocation fence.
        // Every suspension retains the original lease in the outer worker.
        let origin = self
            .native_preflight(a, deadline, || {
                self.media
                    .store
                    .native_media_origin_with_peer(credential, proof, owned.lease()?)
            })
            .await?;
        let read = match self
            .native_preflight(a, deadline, || {
                self.media
                    .store
                    .native_media_read_with_peer(&origin, None, proof)
            })
            .await
        {
            Ok(read) => Arc::new(read),
            Err(error) => {
                return Err(error);
            }
        };
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();
        self.attach(
            &read.target().session,
            a.principal(),
            peer.clone(),
            authority.retained(),
        )?;
        let base = crate::canonical_native::base_profile()?;
        let entry = self
            .media
            .register(origin.clone(), a.principal().clone(), &base, &read)
            .map_err(|_| Error::forbidden())?;
        let id = ConnectionId::from_string(origin.runtime_connection_id().to_owned());
        // Register cleanup custody before any reservation/configuration can fail.
        owned.id = Some(id.clone());
        owned.entry = Some(entry.clone());
        owned.peer = Some(peer.clone());
        let binding = host.reserve_physical(&peer, physical.lifetime.child_token())?;
        owned.binding = Some(binding.clone());
        let owner = entry
            .owner(read.target(), a.principal(), false)
            .map_err(|_| Error::forbidden())?;
        let view = Preparing {
            authority: PeerNativeAuthority {
                actor: owner.member,
                endpoint: owner.endpoint,
                may_prepare: true,
            },
            _read: read.clone(),
        };
        let prepared = self
            .core
            .prepare_peer_webrtc_connection(
                peer.clone(),
                a.principal(),
                PeerWebRtcIntent {
                    session: SessionId::from_string(origin.session_id().to_owned()),
                    connection: id.clone(),
                },
                &view,
                binding.clone(),
            )
            .map_err(|_| Error::forbidden())?;
        // Only a successful preparation proves custody of this Core route.
        // A refusal must never end an unrelated conflicting route.
        owned.reserved = true;
        prepared
            .retain(a.principal(), &view)
            .map_err(|_| Error::forbidden())?;
        let invocation = crate::canonical_media::Invocation {
            policy: self.media.clone(),
            connection: entry.clone(),
            authorized: a.clone(),
            read: read.clone(),
            participant_revision: self
                .core
                .participant_revision(&SessionId::from_string(origin.session_id().to_owned()))
                .map_err(|_| Error::forbidden())?,
        };
        let (controller, activation) = host
            .adapter()
            .configure_media(
                &id,
                a.principal().clone(),
                ConversationId::from_string(origin.conversation_id().to_owned()),
                self.media.clone(),
                1,
                &invocation,
            )
            .map_err(|_| Error::forbidden())?;
        owned.controller = Some(controller);
        entry
            .set_activation(activation)
            .map_err(|_| Error::forbidden())?;
        let attempt = host.start(Preparation {
            store: self.media.store.clone(),
            origin,
            attempt: owned.lease.take().ok_or_else(Error::storage)?,
            read,
            physical: physical.lifetime.child_token(),
            binding,
            deadline,
        });
        owned.attempt = Some(attempt.clone());
        let answer = tokio::select! {
            biased;
            _=self.stop.cancelled()=>return Err(Error::forbidden()),
            _=physical.lifetime.cancelled()=>return Err(Error::forbidden()),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","WebRTC answer deadline")),
            answer=attempt.answer(deadline)=>answer?,
        };
        self.native_preflight(a, deadline, || {
            crate::peer_authority::bind_webrtc_answer(&self.media.store, a, &answer)
        })
        .await?;
        let payload = serde_json::to_value(answer).map_err(|_| Error::storage())?;
        let envelope = crate::canonical::reply(a.envelope(), "connection.answer", payload)
            .map_err(|_| Error::forbidden())?;
        a.answer_offer(envelope, deadline)
            .await
            .map_err(|_| Error::forbidden())?;
        Ok(Reply {
            record,
            answered: true,
        })
    }
}

struct Preparing {
    authority: PeerNativeAuthority,
    _read: Arc<NativeMediaRead>,
}
impl PeerNativeAuthorityLease for Preparing {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.authority
    }
}
