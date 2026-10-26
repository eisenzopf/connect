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
use rvoip_uctp::v2::{auth_dispatch::AuthorizedEnvelope, payloads::ConnectionAnswer};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// One original Store-issued attempt; the host must take custody synchronously.
/// It must not publish media or report operation success from SDP/DTLS alone.
pub struct Preparation {
    pub store: Arc<Store>,
    pub origin: Arc<NativeMediaOrigin>,
    pub attempt: NativeOfferLease,
    /// Bounded originating invocation custody; release before idle/cleanup.
    pub read: Arc<NativeMediaRead>,
    pub physical: CancellationToken,
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
    /// Return an owner even on allocation refusal so the original lease remains
    /// available for attributed cleanup. This must not start another attempt.
    fn start(&self, preparation: Preparation) -> Arc<dyn Attempt>;
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
        runtime
            .webrtc
            .set(host)
            .map_err(|_| Error::new("conflict", "WebRTC host already installed"))
    }
}
struct Closed {
    attempt: Arc<dyn Attempt>,
}
impl NativeOfferRuntimeObservation for Closed {
    fn observe(&self) -> Result<NativeOfferObservation> {
        let a = self.attempt.original_attempt();
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
        let mut owned = None;
        let outcome = self
            .webrtc_start(&a, &physical, &credential, &mut owned)
            .await;
        let succeeded = outcome.is_ok();
        let _ = tx.send(outcome);
        if let Some(attempt) = &owned {
            attempt.release_invocation();
        }
        // No command or domain read is held while waiting for physical retirement.
        drop(a);
        let Some(attempt) = owned else { return };
        if succeeded {
            let expiry = attempt.original_attempt().expires_at();
            tokio::select! {
                _ = attempt.closed() => {},
                _ = self.stop.cancelled() => {},
                _ = physical.lifetime.cancelled() => {},
                _ = tokio::time::sleep(Duration::from_secs((expiry-chrono::Utc::now().timestamp()).max(0) as u64)) => {},
            }
        }
        // The original owner confirms actual RTC close. A missing confirmation
        // cannot produce a durable retirement fact or release its owned capacity.
        if attempt.shutdown().await.is_err() {
            return;
        }
        loop {
            let store = self.media.store.clone();
            let owner = attempt.clone();
            let settled = tokio::task::spawn_blocking(move || {
                store.settle_native_offer_cleanup(
                    owner.original_attempt(),
                    &Closed {
                        attempt: owner.clone(),
                    },
                )
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
        a: &AuthorizedEnvelope,
        physical: &Physical,
        credential: &str,
        owned: &mut Option<Arc<dyn Attempt>>,
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
        let remaining = (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = Instant::now() + Duration::from_secs(remaining.min(30));
        // No suspension between issuance and transferring custody to the host.
        // Origin issuance uses the exact current originating invocation fence.
        let origin = match self
            .native_preflight(a, deadline, || {
                self.media
                    .store
                    .native_media_origin_with_peer(credential, proof, &lease)
            })
            .await
        {
            Ok(origin) => origin,
            Err(error) => {
                *owned = Some(Arc::new(Unallocated(lease)));
                return Err(error);
            }
        };
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
                *owned = Some(Arc::new(Unallocated(lease)));
                return Err(error);
            }
        };
        let attempt = host.start(Preparation {
            store: self.media.store.clone(),
            origin,
            attempt: lease,
            read,
            physical: physical.lifetime.child_token(),
            deadline,
        });
        *owned = Some(attempt.clone());
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

// No transport was allocated when origin issuance failed. Keep the exact lease
// in the tracked worker until its absence has been durably attributed.
struct Unallocated(NativeOfferLease);
#[async_trait]
impl Attempt for Unallocated {
    fn original_attempt(&self) -> &NativeOfferLease {
        &self.0
    }
    async fn answer(&self, _: Instant) -> Result<ConnectionAnswer> {
        Err(Error::forbidden())
    }
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
    async fn closed(&self) -> Result<()> {
        Ok(())
    }
}
