//! Original physical restart custody. Wire dispatch remains separately gated.
use super::*;
use crate::canonical_media::Policy;
use connect_store::{
    AuthorityWait, NativeIceOrigin, NativeStreamCommitted, NativeStreamLease, PeerAuthorityGuard,
};
use rvoip_core::{peer_admission::PeerAdmission, staged_streams::TransportUpdateSlot};
use rvoip_webrtc::peer::RvoipPeerConnection;

/// The ordinary signed reply writer owns the original invocation through its
/// actual write. Read the private original answer under that SAME current proof;
/// never embed SDP in operation.updated, shared results or journal projections.
pub(crate) fn ack_details(store: &Store, a: &AuthorizedEnvelope) -> Result<serde_json::Value> {
    let mut details = serde_json::json!({"reply_complete":true});
    if a.envelope().kind() != "connection.update"
        || a.envelope().original()["payload"]["action"] != "ice-restart"
    {
        return Ok(details);
    }
    let authority = a
        .authority::<StoreInvocationAuthority>()
        .ok_or_else(Error::forbidden)?;
    let proof = authority.peer_guard().ok_or_else(Error::forbidden)?;
    let wire = crate::canonical_native_streams::intent(a.envelope())?;
    let update = connect_protocol::webrtc_ice_restart::parse_update(&wire.payload)?;
    if let Some(answer) =
        store.native_ice_answer_with_peer(authority.binding().credential_id(), proof)?
    {
        details["ice_restart"] =
            serde_json::to_value(connect_protocol::webrtc_ice_restart::IceRestartAnswer::new(
                update.details.expected_ice_generation,
                answer,
            )?)
            .map_err(|_| Error::storage())?;
    }
    Ok(details)
}

/// One owned local attempt. Only the caller's tracked worker may own this
/// value. Idle custody holds the exclusive Core slot, never a Store reader or
/// originating AuthorizedEnvelope. Retain it through answer delivery and the
/// original-generation observation; uncertain recovery must never call start.
pub(super) struct Restart {
    policy: Arc<Policy>,
    adapter: Arc<CanonicalWebRtcAdapter>,
    peer: Arc<PeerAdmission>,
    origin: Arc<NativeIceOrigin>,
    credential: String,
    id: ConnectionId,
    lease: Option<NativeStreamLease>,
    receipt: Option<NativeStreamCommitted>,
    slot: Option<TransportUpdateSlot>,
    rtc: Option<Arc<RvoipPeerConnection>>,
    submitted: bool,
    deadline: Instant,
    stop: CancellationToken,
    physical: CancellationToken,
}
impl Restart {
    /// Consume only a fresh already-begun Store lease. A saved baseline/answer
    /// requires original-attempt reconciliation, never a new physical worker.
    pub(super) fn new(
        policy: Arc<Policy>,
        adapter: Arc<CanonicalWebRtcAdapter>,
        peer: Arc<PeerAdmission>,
        credential: String,
        proof: &PeerAuthorityGuard,
        lease: NativeStreamLease,
        deadline: Instant,
        stop: CancellationToken,
        physical: CancellationToken,
    ) -> std::result::Result<Self, (Error, NativeStreamLease)> {
        if lease
            .ice_restart()
            .is_none_or(|ice| ice.previous_local_answer().is_some() || ice.answer().is_some())
        {
            return Err((
                Error::new(
                    "reconciliation-required",
                    "Original ICE attempt already has negotiation custody",
                ),
                lease,
            ));
        }
        let origin = match policy
            .store
            .native_ice_origin_with_peer(&credential, proof, &lease)
        {
            Ok(origin) => origin,
            Err(e) => return Err((e, lease)),
        };
        let deadline = deadline.min(Instant::now() + Duration::from_secs(30)).min(
            Instant::now()
                + Duration::from_secs(
                    (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
                ),
        );
        let id = ConnectionId::from_string(lease.runtime_connection_id().to_owned());
        Ok(Self {
            policy,
            adapter,
            peer,
            origin,
            credential,
            id,
            lease: Some(lease),
            receipt: None,
            slot: None,
            rtc: None,
            submitted: false,
            deadline,
            stop,
            physical,
        })
    }
    fn lease(&self) -> Result<&NativeStreamLease> {
        self.lease.as_ref().ok_or_else(Error::forbidden)
    }
    fn invocation(&self) -> Result<crate::canonical_media::ice::Invocation> {
        if self.stop.is_cancelled()
            || self.physical.is_cancelled()
            || Instant::now() >= self.deadline
        {
            return Err(Error::forbidden());
        }
        let proof = self.policy.store.resume_native_ice_authority(
            &self.credential,
            &self.origin,
            self.lease()?,
            &AuthorityWait::new(Duration::from_millis(100)),
        )?;
        self.policy.ice_invocation(
            Arc::new(proof),
            self.lease()?,
            self.peer.clone(),
            self.deadline,
        )
    }
    /// Exactly one physical invocation, never called by status/replay. Every
    /// error after submission remains possible-effect; no regather/retry path.
    /// Returns the persisted original answer, not an ICE-ready receipt.
    pub(super) async fn negotiate(
        &mut self,
        authority: &StoreInvocationAuthority,
        cancelled: &CancellationToken,
    ) -> Result<String> {
        tokio::select! { biased;
            _ = cancelled.cancelled() => Err(Error::forbidden()),
            answer = self.negotiate_held(authority) => answer,
        }
    }
    async fn negotiate_held(&mut self, authority: &StoreInvocationAuthority) -> Result<String> {
        if self.submitted {
            return Err(Error::forbidden());
        }
        // Derive from the handler's SAME interval instead of reacquiring
        // readers behind a waiting authority writer during first negotiation.
        let invocation = self.policy.held_ice_invocation(
            authority,
            self.lease()?,
            self.peer.clone(),
            self.deadline,
        )?;
        let controller = self
            .adapter
            .staged_controller(&self.id)
            .ok_or_else(Error::forbidden)?;
        if self.slot.is_none() {
            let expected = self.lease()?.expected_revision();
            self.slot = Some(
                controller
                    .reserve_transport_update_with_held(expected, self.deadline, &invocation)
                    .map_err(|error| {
                        if matches!(
                            error,
                            rvoip_core::staged_streams::StreamControlError::Model(
                                rvoip_core::stream_lifecycle::MediaTransitionError::Conflict
                            )
                        ) && controller.revision() == expected
                        {
                            Error::new("authority-busy", "Original ICE control slot is busy")
                        } else {
                            media_error(error)
                        }
                    })?,
            );
        }
        let rtc = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            self.adapter
                .original_peer(&self.id, &self.peer, self.deadline),
        )
        .await?
        .map_err(|_| Error::forbidden())?;
        let descriptions = bounded(self.deadline, &self.stop, &self.physical, async {
            (
                rtc.peer_connection().local_description().await,
                rtc.peer_connection().remote_description().await,
            )
        })
        .await?;
        let (Some(local), Some(remote)) = descriptions else {
            return Err(Error::forbidden());
        };
        let proposed = self
            .lease()?
            .ice_restart()
            .ok_or_else(Error::forbidden)?
            .proposed_remote_offer()
            .to_owned();
        let lease = self.lease.as_mut().ok_or_else(Error::forbidden)?;
        self.policy
            .store
            .capture_native_ice_baseline(lease, &local.sdp, &remote.sdp)?;
        let current = self
            .slot
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .current_with_held(&invocation)
            .map_err(media_error)?;
        self.rtc = Some(rtc.clone());
        // Set before the first possible physical effect, including an engine
        // error or cancelled observer. Those outcomes cannot become no-effect.
        self.submitted = true;
        let answer = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            rtc.restart_ice_as_answerer(&proposed),
        )
        .await?
        .map_err(|_| {
            Error::new(
                "reconciliation-required",
                "Original ICE restart outcome is uncertain",
            )
        })?;
        self.slot
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .current_with_held(&invocation)
            .map_err(media_error)?;
        if !self
            .adapter
            .is_peer_webrtc_preparation_current(&self.id, &self.peer)
        {
            return Err(Error::forbidden());
        }
        self.policy.store.capture_native_ice_answer(
            self.lease.as_mut().ok_or_else(Error::forbidden)?,
            &answer,
        )?;
        drop(current);
        drop(invocation);
        Ok(answer)
    }
    /// One short fresh-authority observation, without sleeping or retaining
    /// readers after return. A true result has captured an immutable original
    /// receipt; metadata persistence may retry without another RTC invocation.
    pub(super) async fn observe(&mut self) -> Result<bool> {
        if self.receipt.is_some() {
            return Ok(true);
        }
        let invocation = self.invocation()?;
        let slot = self.slot.as_ref().ok_or_else(Error::forbidden)?;
        let current = slot.current_with_held(&invocation).map_err(media_error)?;
        let rtc = self.rtc.as_ref().ok_or_else(Error::forbidden)?;
        let same = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            self.adapter
                .original_peer(&self.id, &self.peer, self.deadline),
        )
        .await?
        .map_err(|_| Error::forbidden())?;
        if !Arc::ptr_eq(rtc, &same) {
            return Err(Error::forbidden());
        }
        let intent = self.lease()?.ice_restart().ok_or_else(Error::forbidden)?;
        let answer = intent.answer().ok_or_else(Error::forbidden)?;
        let ready = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            rtc.peer_connection()
                .ice_descriptions_ready(answer, intent.proposed_remote_offer()),
        )
        .await?;
        // Verify the same slot/owner/ports/credential and physical lifetime
        // again after the live core lock observation, before taking the receipt.
        slot.current_with_held(&invocation).map_err(media_error)?;
        if !self
            .adapter
            .is_peer_webrtc_preparation_current(&self.id, &self.peer)
        {
            return Err(Error::forbidden());
        }
        if ready {
            let generation = intent.expected_generation() + 1;
            self.receipt = Some(
                self.lease
                    .take()
                    .ok_or_else(Error::forbidden)?
                    .ice_committed(generation)?,
            );
        }
        drop(current);
        drop(invocation);
        Ok(ready)
    }
    pub(super) fn complete(&self) -> Result<connect_protocol::transitions::OperationRecord> {
        self.policy
            .store
            .complete_native_stream(self.receipt.as_ref().ok_or_else(Error::forbidden)?)
    }
    pub(super) fn uncertain(&self) -> Result<connect_protocol::transitions::OperationRecord> {
        self.policy
            .store
            .native_stream_attempt_uncertain(self.lease()?)
    }
    pub(super) fn submitted(&self) -> bool {
        self.submitted
    }
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }
    /// Release exclusive negotiation custody before Core's original close/join.
    /// The actual attempt and peer remain owned until retirement is confirmed.
    pub(super) fn release_slot(&mut self) {
        self.slot.take();
    }
    async fn close_original(&mut self) -> Result<()> {
        self.release_slot();
        let rtc = self.rtc.as_ref().ok_or_else(Error::forbidden)?;
        // Historical cleanup is bound to actual allocation identity, even
        // after authority or the physical signaling peer has retired.
        self.adapter
            .end_original(self.id.clone(), &self.peer, rtc, EndReason::Normal)
            .await
            .map_err(|_| {
                Error::new(
                    "reconciliation-required",
                    "Original ICE cleanup is unconfirmed",
                )
            })
    }
    /// Definitive refusal is possible only before calling the physical engine.
    pub(super) fn refuse(
        self,
        error: Error,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        if self.submitted {
            return Err(Error::forbidden());
        }
        self.policy
            .store
            .fail_native_stream_without_effect(self.lease.ok_or_else(Error::forbidden)?, error)
    }
}

fn original_proof(a: &AuthorizedEnvelope) -> Result<&PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|h| h.peer_guard())
        .ok_or_else(Error::forbidden)
}
fn reply(record: connect_protocol::transitions::OperationRecord) -> Reply {
    Reply {
        record,
        answered: false,
    }
}
impl Runtime {
    pub(super) async fn ice_stream(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
    ) -> Result<Reply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let runtime = self.clone();
        {
            let mut work = self.work.lock().map_err(|_| Error::storage())?;
            while work.try_join_next().is_some() {}
            if self.stop.is_cancelled()
                || work.len() >= self.media.store.limits().connections.saturating_mul(2)
            {
                return Err(Error::new("busy", "RTC control workers are at capacity"));
            }
            work.spawn(async move {
                Box::pin(runtime.ice_worker(a, physical, credential, tx)).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    async fn ice_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) {
        let store = &self.media.store;
        // Exact retained intent/current authority, with no live peer needed
        // for historical metadata and no second attempt on Unknown/replay.
        match original_proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p))
        {
            Ok(Some(record)) if record.state != OperationState::Accepted => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Ok(_) => (),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
        if let Err(e) = self.rtc_stream_source(&a, &physical, &credential) {
            let _ = tx.send(Err(e));
            return;
        }
        let accepted = match original_proof(&a)
            .and_then(|p| store.accept_native_stream_with_peer(&credential, p))
        {
            Ok(record) => record,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        if accepted.state != OperationState::Accepted {
            let _ = tx.send(Ok(reply(accepted)));
            return;
        }
        let lease = match original_proof(&a)
            .and_then(|p| store.begin_native_stream_with_peer(&credential, p))
        {
            Ok(Some((lease, _))) => lease,
            Ok(None) => {
                let _ = tx.send(
                    original_proof(&a)
                        .and_then(|p| store.native_stream_record_with_peer(&credential, p))
                        .and_then(|r| r.ok_or_else(Error::storage))
                        .map(reply),
                );
                return;
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let host = self.webrtc.get().expect("RTC source already verified");
        let peer = a
            .peer_admission()
            .expect("RTC source already verified")
            .clone();
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .expect("RTC source already verified");
        let mut attempt = match Restart::new(
            self.media.clone(),
            host.adapter(),
            peer,
            credential.clone(),
            original_proof(&a).expect("RTC source already verified"),
            lease,
            Instant::now() + Duration::from_secs(30),
            self.stop.clone(),
            physical.lifetime.clone(),
        ) {
            Ok(attempt) => attempt,
            Err((error, lease)) => {
                let result = if error.code == "reconciliation-required" {
                    store.native_stream_attempt_uncertain(&lease)
                } else {
                    store.fail_native_stream_without_effect(lease, error)
                };
                let _ = tx.send(result.map(reply));
                return;
            }
        };
        let finished = a.invocation_cancellation();
        let outcome = loop {
            match attempt.negotiate(authority, &finished).await {
                Err(e)
                    if e.code == "authority-busy"
                        && !attempt.submitted()
                        && Instant::now() < attempt.deadline()
                        && !finished.is_cancelled() =>
                {
                    tokio::select! { biased;
                        _ = self.stop.cancelled() => break Err(Error::forbidden()),
                        _ = physical.lifetime.cancelled() => break Err(Error::forbidden()),
                        _ = finished.cancelled() => break Err(Error::forbidden()),
                        _ = tokio::time::sleep(Duration::from_millis(5)) => (),
                    }
                }
                result => break result,
            }
        };
        if let Err(error) = outcome {
            if !attempt.submitted() {
                let _ = tx.send(attempt.refuse(error).map(reply));
                return;
            }
            let _ = tx.send(attempt.uncertain().map(reply));
            drop(a);
            let _ = attempt.close_original().await;
            return;
        }
        let response = original_proof(&a)
            .and_then(|p| store.native_stream_record_with_peer(&credential, p))
            .and_then(|r| r.ok_or_else(Error::storage))
            .map(reply);
        let valid = response.is_ok();
        let delivered = tx.send(response).is_ok();
        drop(a);
        // The physical actor owns the original reply readers through its write;
        // none of those readers survives in this tracked worker's idle custody.
        let finished = valid
            && delivered
            && tokio::select! { biased;
                _ = self.stop.cancelled() => false,
                _ = physical.lifetime.cancelled() => false,
                _ = tokio::time::sleep_until(attempt.deadline()) => false,
                _ = finished.cancelled() => true,
            };
        if finished {
            loop {
                match attempt.observe().await {
                    Ok(true) => {
                        attempt.release_slot();
                        // Captured original receipt only: storage retries cannot
                        // gather SDP, read another peer or perform RTC IO again.
                        loop {
                            match attempt.complete() {
                                Ok(_) => return,
                                Err(e) if e.code == "forbidden" => return,
                                Err(_) => (),
                            }
                            tokio::select! { biased;
                                _ = self.stop.cancelled() => return,
                                _ = physical.lifetime.cancelled() => return,
                                _ = tokio::time::sleep(Duration::from_millis(100)) => (),
                            }
                        }
                    }
                    Ok(false) => (),
                    Err(e) if e.code == "authority-busy" => (),
                    Err(_) => break,
                }
                tokio::select! { biased;
                    _ = self.stop.cancelled() => break,
                    _ = physical.lifetime.cancelled() => break,
                    _ = tokio::time::sleep_until(attempt.deadline()) => break,
                    _ = tokio::time::sleep(Duration::from_millis(50)) => (),
                }
            }
        }
        let _ = attempt.uncertain();
        let _ = attempt.close_original().await;
        // The original parent custodian observes this actual joined close and
        // records its own attributed lifetime retirement. ICE stays Unknown.
    }
}
async fn bounded<T>(
    deadline: Instant,
    stop: &CancellationToken,
    physical: &CancellationToken,
    future: impl std::future::Future<Output = T>,
) -> Result<T> {
    tokio::select! { biased;
        _ = stop.cancelled() => Err(Error::forbidden()),
        _ = physical.cancelled() => Err(Error::forbidden()),
        _ = tokio::time::sleep_until(deadline) => Err(Error::new("reconciliation-required", "Original ICE observation expired")),
        result = future => Ok(result),
    }
}

fn media_error(error: rvoip_core::staged_streams::StreamControlError) -> Error {
    use rvoip_core::{
        staged_streams::StreamControlError as E, stream_lifecycle::MediaTransitionError,
    };
    match error {
        E::NotReady | E::Backpressure => {
            Error::new("authority-busy", "ICE current authority is busy")
        }
        E::Model(MediaTransitionError::Conflict) => {
            Error::new("conflict", "Original ICE media binding changed")
        }
        E::Deadline => Error::new("reconciliation-required", "Original ICE control expired"),
        _ => Error::forbidden(),
    }
}
