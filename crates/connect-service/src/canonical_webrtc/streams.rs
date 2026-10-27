//! Signed routing commands on the original RTC owner. No native counteroffer.
use super::*;
use crate::canonical_media::streams::Plan;
use connect_store::NativeStreamProjection;
use rvoip_core::{
    ids::{ParticipantId, StreamId},
    orchestrator::participant_control::ParticipantControlError,
    staged_streams::{
        subscription_plan::{MediaSelection, MediaSelectionChange, MediaSubscriptionPlan},
        StreamControlError,
    },
    stream_lifecycle::Medium,
};
use rvoip_uctp::v2::payloads::Payload;
use std::collections::BTreeSet;

fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
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
fn engine_error(error: ParticipantControlError) -> Error {
    use rvoip_core::stream_lifecycle::MediaTransitionError;
    match error {
        ParticipantControlError::Media(StreamControlError::Backpressure) => {
            Error::new("authority-busy", "RTC subscription catalog is busy")
        }
        ParticipantControlError::Media(StreamControlError::Model(
            MediaTransitionError::Conflict,
        )) => Error::new("conflict", "RTC subscription revision conflicts"),
        ParticipantControlError::Media(StreamControlError::NotReady) => {
            Error::new("not-found", "RTC subscription source is unavailable")
        }
        ParticipantControlError::Unsupported
        | ParticipantControlError::Media(StreamControlError::Unsupported) => Error::new(
            "unsupported-media-profile",
            "RTC subscription requires another media profile",
        ),
        _ => Error::forbidden(),
    }
}
enum RtcOutcome {
    Subscriptions(u64),
    Gates(u64, Vec<rvoip_core::stream_lifecycle::LogicalStream>),
}
impl Runtime {
    fn rtc_stream_source(
        &self,
        a: &AuthorizedEnvelope,
        physical: &Physical,
        credential: &str,
    ) -> Result<(ConnectionId, NativeStreamProjection)> {
        physical.validate(
            a,
            a.authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?,
        )?;
        if self.stop.is_cancelled()
            || physical.lifetime.is_cancelled()
            || physical.binding.transport() != rvoip_core::Transport::WebSocket
        {
            return Err(Error::forbidden());
        }
        let source = self
            .media
            .store
            .project_native_stream_with_peer(credential, proof(a)?)?;
        let id = ConnectionId::from_string(source.connection.runtime_connection_id.clone());
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?;
        let host = self.webrtc.get().ok_or_else(Error::forbidden)?;
        if source.connection.endpoint_transport != EndpointTransport::Webrtc
            || !host.adapter().is_peer_webrtc_preparation_current(&id, peer)
        {
            return Err(Error::forbidden());
        }
        Ok((id, source))
    }
    fn rtc_subscription_plan(
        &self,
        a: &AuthorizedEnvelope,
        id: &ConnectionId,
        source: &NativeStreamProjection,
    ) -> Result<MediaSubscriptionPlan> {
        let member = proof(a)?.native_member().ok_or_else(Error::forbidden)?.0;
        // The typed UCTP parser and Connect intent validator have already
        // validated selector shape, bounds and duplicates. The installed RTC
        // profile maps only audio onto its existing negotiated mixed port.
        let input = match a.envelope().payload() {
            Payload::StreamSubscribe(input) => {
                if input
                    .by_participant
                    .as_ref()
                    .is_some_and(|p| p.as_str() != member)
                {
                    return Err(Error::forbidden());
                }
                let mut selections = Vec::new();
                for s in &input.subscriptions {
                    match (&s.strm_id, &s.from_participant, &s.kinds) {
                        (Some(id), None, None) => selections
                            .push(MediaSelection::Stream(StreamId::from_string(id.as_str()))),
                        (None, Some(id), kinds) => {
                            if kinds
                                .as_ref()
                                .is_some_and(|k| k.len() != 1 || k[0] != "audio")
                            {
                                return Err(Error::new(
                                    "unsupported-media-profile",
                                    "RTC subscription profile supports audio only",
                                ));
                            }
                            let participant = ParticipantId::from_string(id.as_str());
                            let media = BTreeSet::from([Medium::Audio]);
                            selections.push(if kinds.is_none() {
                                MediaSelection::ParticipantAll { participant, media }
                            } else {
                                MediaSelection::Participant { participant, media }
                            });
                        }
                        _ => {
                            return Err(Error::new(
                                "invalid-request",
                                "Invalid RTC subscription selector",
                            ))
                        }
                    }
                }
                MediaSelectionChange::Subscribe(selections)
            }
            Payload::StreamUnsubscribe(input) => MediaSelectionChange::Unsubscribe(
                input
                    .strm_ids
                    .iter()
                    .map(|id| StreamId::from_string(id.as_str()))
                    .collect(),
            ),
            _ => {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "RTC stream command is not installed",
                ))
            }
        };
        let expected = a
            .envelope()
            .expected_revision()
            .ok_or_else(Error::forbidden)?;
        if expected != source.stream_revision {
            return Err(Error::new(
                "conflict",
                "RTC durable stream revision conflicts",
            ));
        }
        let plan = self
            .core
            .plan_staged_media_selection(
                &SessionId::from_string(source.connection.connection.session_id.clone()),
                id,
                expected,
                &input,
            )
            .map_err(engine_error)?;
        let controller = self
            .webrtc
            .get()
            .ok_or_else(Error::forbidden)?
            .adapter()
            .staged_controller(id)
            .ok_or_else(Error::forbidden)?;
        let bindings = controller.active_bindings();
        if controller.revision() != expected {
            return Err(Error::new("conflict", "RTC media revision conflicts"));
        }
        if plan.requires_reconciliation()
            || plan.desired().len() != bindings.len()
            || plan.desired().iter().any(|s| {
                s.medium != Medium::Audio || !bindings.iter().any(|b| &b.negotiated.requested == s)
            })
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "RTC subscription requires media renegotiation",
            ));
        }
        Ok(plan)
    }
    pub(crate) async fn webrtc_stream(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
    ) -> Result<Reply> {
        let gates = a.envelope().kind() == "connection.update"
            && connect_protocol::native_gates::is_gate(&a.envelope().original()["payload"]);
        if !gates
            && !matches!(
                a.envelope().payload(),
                Payload::StreamSubscribe(_)
                    | Payload::StreamUnsubscribe(_)
                    | Payload::ConnectionEnd(_)
            )
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "RTC stream command is not installed",
            ));
        }
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
                runtime.rtc_stream_worker(a, physical, credential, tx).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    async fn rtc_stream_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) {
        let store = &self.media.store;
        // Historical replay has no engine effect. Current Store authority is
        // still mandatory, while a foreign peer cannot begin an accepted root.
        match proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p)) {
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
        let ending = matches!(a.envelope().payload(), Payload::ConnectionEnd(_));
        let admission = if ending {
            self.accept_after_managed(&a, &physical, &credential).await
        } else {
            proof(&a).and_then(|p| store.accept_native_stream_with_peer(&credential, p))
        };
        let accepted = match admission {
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
        let remaining = accepted
            .expires_at
            .unwrap_or(0)
            .saturating_mul(1000)
            .saturating_sub(chrono::Utc::now().timestamp_millis())
            .max(0) as u64;
        let cutoff = Instant::now() + Duration::from_millis(remaining);
        let planned = self
            .native_preflight(&a, cutoff, || {
                let (id, source) = self.rtc_stream_source(&a, &physical, &credential)?;
                let plan = if ending {
                    None
                } else if a.envelope().kind() == "connection.update" {
                    Some(Plan::RtcGates(self.rtc_gate_plan(&a, &id, &source)?))
                } else {
                    Some(Plan::RtcSubscriptions(
                        self.rtc_subscription_plan(&a, &id, &source)?,
                    ))
                };
                if matches!(a.envelope().payload(), Payload::StreamSubscribe(_)) {
                    store.validate_native_subscription_sources_with_peer(
                        &credential,
                        proof(&a)?,
                        &["audio".into()],
                    )?;
                }
                let sid = SessionId::from_string(source.connection.connection.session_id.clone());
                Ok((id, plan, sid))
            })
            .await;
        let (id, plan, sid) = match planned {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(
                    proof(&a)
                        .and_then(|p| {
                            store.refuse_native_stream_before_effect_with_peer(&credential, p, e)
                        })
                        .map(reply),
                );
                return;
            }
        };
        let (lease, started) =
            match proof(&a).and_then(|p| store.begin_native_stream_with_peer(&credential, p)) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    let _ = tx.send(
                        proof(&a)
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
        let deadline = cutoff.min(
            Instant::now()
                + Duration::from_secs(
                    (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
                ),
        );
        let ready = self
            .native_preflight(&a, deadline, || {
                self.rtc_stream_source(&a, &physical, &credential)?;
                store.validate_native_stream_attempt(&credential, proof(&a)?, &lease)?;
                plan.clone()
                    .map(|plan| {
                        self.media
                            .stream_invocation(a.clone(), plan, &lease, deadline)
                    })
                    .transpose()
            })
            .await;
        let invocation = match ready {
            Ok(i) => i,
            Err(e) => {
                let _ = tx.send(store.fail_native_stream_without_effect(lease, e).map(reply));
                return;
            }
        };
        let receipt = if let (Some(invocation), Some(plan)) = (invocation, plan) {
            let outcome = tokio::select! {
                biased;
                _=self.stop.cancelled()=>Err(Error::forbidden()),
                r=async {
                    match plan {
                        Plan::RtcSubscriptions(plan) => self.core.change_staged_media_subscriptions(&sid, &id, plan.expected_revision(), plan.routing().clone(), deadline, a.invocation_cancellation(), Some(invocation.as_ref())).await.map(RtcOutcome::Subscriptions).map_err(engine_error),
                        Plan::RtcGates(plan) => {
                            let controller = self.webrtc.get().and_then(|h| h.adapter().staged_controller(&id)).ok_or_else(Error::forbidden)?;
                            controller.set_media_gate_with_held(plan.expected, &plan.streams, plan.action, deadline, invocation.clone()).await.map(|(revision, streams)| RtcOutcome::Gates(revision, streams)).map_err(|error| engine_error(ParticipantControlError::Media(error)))
                        }
                        _ => Err(Error::forbidden()),
                    }
                }=>r,
            };
            drop(invocation);
            let observed = match outcome {
                Ok(r) => r,
                Err(error) => {
                    tracing::debug!(?error, "RTC original stream effect requires reconciliation");
                    let _ = tx.send(store.native_stream_attempt_uncertain(&lease).map(reply));
                    return;
                }
            };
            match observed {
                RtcOutcome::Gates(revision, streams) => {
                    match crate::canonical_native_streams::gate_streams(
                        &streams,
                        lease.generation(),
                    ) {
                        Ok(streams) => lease.gates_committed(revision, streams),
                        Err(error) => {
                            let _ =
                                tx.send(store.native_stream_attempt_uncertain(&lease).map(reply));
                            tracing::debug!(?error, "RTC gate observation could not be captured");
                            return;
                        }
                    }
                }
                RtcOutcome::Subscriptions(revision) => {
                    let projection = self
                        .webrtc
                        .get()
                        .and_then(|h| h.adapter().staged_controller(&id))
                        .and_then(|c| {
                            if c.revision() != revision {
                                return None;
                            }
                            let bindings = c.active_bindings();
                            (c.revision() == revision)
                                .then(|| {
                                    crate::canonical_native_streams::codec_bindings(&bindings).ok()
                                })
                                .flatten()
                        });
                    lease.committed(revision, false, projection)
                }
            }
        } else {
            // The lease and original RTC route remain owned across the actual
            // close/join. Only its returned success plus confirmed absence may
            // become an ended receipt; cancellation or timeout stays uncertain.
            let outcome = tokio::select! { biased;
                _=self.stop.cancelled()=>Err(Error::forbidden()),
                _=tokio::time::sleep_until(deadline)=>Err(Error::new("reconciliation-required", "RTC termination is uncertain")),
                r=self.core.end_connection(id.clone(), EndReason::Normal)=>r.map_err(|_| Error::new("reconciliation-required", "RTC termination is uncertain")),
            };
            if outcome.is_err() || self.native_connection_live(&id) {
                let _ = tx.send(store.native_stream_attempt_uncertain(&lease).map(reply));
                return;
            }
            let absent = RtcAbsent {
                adapter: self.webrtc.get().map(|h| h.adapter()),
                id,
                generation: lease.generation(),
            };
            lease.ended(&absent)
        };
        let receipt = match receipt {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        match store.complete_native_stream(&receipt) {
            Ok(record) => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Err(_) => {
                let _ = tx.send(Ok(reply(started)));
            }
        }
        // Metadata retries retain only this actual result, never a live command
        // reader or an opportunity to perform another routing mutation.
        drop(a);
        drop(physical);
        drop(credential);
        loop {
            tokio::select! { _=self.stop.cancelled()=>return, _=tokio::time::sleep(Duration::from_millis(100))=>{} }
            match store.complete_native_stream(&receipt) {
                Ok(_) => return,
                Err(e) if e.code == "forbidden" => return,
                Err(_) => (),
            }
        }
    }
}

struct RtcAbsent {
    adapter: Option<Arc<CanonicalWebRtcAdapter>>,
    id: ConnectionId,
    generation: u64,
}
impl NativeOfferRuntimeObservation for RtcAbsent {
    fn observe(&self) -> Result<NativeOfferObservation> {
        let adapter = self.adapter.as_ref().ok_or_else(Error::forbidden)?;
        if adapter.is_connection_live(&self.id) {
            return Err(Error::new(
                "reconciliation-required",
                "Original RTC cleanup is unconfirmed",
            ));
        }
        Ok(NativeOfferObservation::Retired {
            runtime_connection_id: self.id.as_str().into(),
            generation: self.generation,
        })
    }
}
