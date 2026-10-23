//! Native controls retain exact wire intent and owned effect custody.
use connect_protocol::{
    native_streams::{Command, Intent},
    Error, Result,
};
use rvoip_uctp::v2::{Envelope, Scope};

pub(crate) fn intent(e: &Envelope) -> Result<Intent> {
    let command = match e.kind() {
        "stream.subscribe" => Command::Subscribe,
        "stream.unsubscribe" => Command::Unsubscribe,
        "connection.end" => Command::End,
        "message.send" => Command::Data,
        "capability.advertise" => Command::Advertise,
        "dtmf.send" => Command::Dtmf,
        "connection.update" => {
            if !matches!(e.payload(),rvoip_uctp::v2::Payload::ConnectionUpdate(u)
                if matches!(u.action, rvoip_uctp::v2::payloads::ConnectionUpdateAction::CodecRenegotiate | rvoip_uctp::v2::payloads::ConnectionUpdateAction::Quality | rvoip_uctp::v2::payloads::ConnectionUpdateAction::Hold | rvoip_uctp::v2::payloads::ConnectionUpdateAction::Resume | rvoip_uctp::v2::payloads::ConnectionUpdateAction::Mute | rvoip_uctp::v2::payloads::ConnectionUpdateAction::Unmute))
            {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "Native connection update action is not installed",
                ));
            }
            Command::Update
        }
        _ => return Err(Error::forbidden()),
    };
    if e.scope(Scope::Reply).is_some() {
        return Err(Error::forbidden());
    }
    let required = |scope| {
        e.scope(scope).map(str::to_owned).ok_or_else(|| {
            Error::new(
                "invalid-request",
                "Native stream command scopes are required",
            )
        })
    };
    let wire = Intent {
        operation_id: required(Scope::Operation)?,
        conversation_id: required(Scope::Conversation)?,
        session_id: required(Scope::Session)?,
        connection_id: required(Scope::Connection)?,
        expected_revision: e.expected_revision(),
        by_participant: e.original()["payload"][if command == Command::Data {
            "from"
        } else {
            "by_participant"
        }]
        .as_str()
        .map(str::to_owned),
        command,
        payload: e.original()["payload"].clone(),
    };
    wire.validate()?;
    Ok(wire)
}

use crate::canonical_media::streams::Plan;
use crate::{
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::transitions::OperationState;
use rvoip_core::{ids::ConnectionId, ConnectionAdapter};
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
enum Observed {
    Streams(rvoip_uctp::v2::native_subscriptions::NativeSubscriptionOutcome),
    Codecs(rvoip_uctp::v2::native_negotiation::NativeOfferOutcome),
    Gates(rvoip_uctp::v2::native_control::NativeControlOutcome),
}
fn gate_streams(
    streams: &[rvoip_core::stream_lifecycle::LogicalStream],
    connection_generation: u64,
) -> Result<Vec<connect_protocol::native_gates::GateStream>> {
    use rvoip_core::stream_lifecycle::{LogicalStreamState, Medium, StreamDirection};
    let direction = |d| {
        match d {
            StreamDirection::Send => "sendonly",
            StreamDirection::Receive => "recvonly",
            StreamDirection::Both => "sendrecv",
        }
        .to_owned()
    };
    streams
        .iter()
        .map(|stream| {
            let requested = &stream.negotiated.requested;
            Ok(connect_protocol::native_gates::GateStream {
                binding: connect_protocol::native_streams::CodecBinding {
                    stream_id: requested.logical_id.as_str().into(),
                    medium: match requested.medium {
                        Medium::Audio => "audio",
                        Medium::Video => "video",
                        Medium::Screen => "screen",
                        Medium::Data => "data",
                    }
                    .into(),
                    direction: direction(requested.direction),
                    codec: crate::peer_authority::native_stream_codec(&stream.negotiated.format)
                        .map_err(|_| Error::forbidden())?,
                    stream_local_id: stream.stream_local_id,
                    stream_generation: stream.generation,
                    connection_generation,
                },
                state: match stream.state {
                    LogicalStreamState::Active => "active",
                    LogicalStreamState::Paused => "paused",
                    LogicalStreamState::Ended => return Err(Error::forbidden()),
                }
                .into(),
                source_muted: stream.source_muted,
                held_direction: stream.hold_direction.map(direction),
            })
        })
        .collect()
}
fn codec_bindings(
    bindings: &[rvoip_core::stream_lifecycle::ReadyMediaBinding],
) -> Result<Vec<connect_protocol::native_streams::CodecBinding>> {
    use rvoip_core::stream_lifecycle::{Medium, StreamDirection};
    bindings
        .iter()
        .map(|b| {
            Ok(connect_protocol::native_streams::CodecBinding {
                stream_id: b.negotiated.requested.logical_id.as_str().into(),
                medium: match b.negotiated.requested.medium {
                    Medium::Audio => "audio",
                    Medium::Video => "video",
                    Medium::Screen => "screen",
                    Medium::Data => "data",
                }
                .into(),
                direction: match b.negotiated.requested.direction {
                    StreamDirection::Send => "sendonly",
                    StreamDirection::Receive => "recvonly",
                    StreamDirection::Both => "sendrecv",
                }
                .into(),
                codec: crate::peer_authority::native_stream_codec(&b.negotiated.format)
                    .map_err(|_| Error::forbidden())?,
                stream_local_id: b.stream_local_id,
                stream_generation: b.generation,
                connection_generation: b.connection_generation,
            })
        })
        .collect()
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|h| h.peer_guard())
        .ok_or_else(Error::forbidden)
}
impl Runtime {
    pub(crate) async fn native_stream(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
    ) -> Result<crate::canonical_native::Reply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let runtime = self.clone();
        {
            let mut work = self.work.lock().map_err(|_| Error::storage())?;
            while work.try_join_next().is_some() {}
            if self.stop.is_cancelled()
                || work.len() >= self.media.store.limits().connections.saturating_mul(2)
            {
                return Err(Error::new("busy", "Native control workers are at capacity"));
            }
            work.spawn(async move {
                runtime.stream_worker(a, physical, credential, tx).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    async fn stream_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<crate::canonical_native::Reply>>,
    ) {
        let store = &self.media.store;
        match proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p)) {
            Ok(Some(record)) if record.state != OperationState::Accepted => {
                let _ = tx.send(Ok(crate::canonical_native::Reply {
                    record,
                    answered: false,
                }));
                return;
            }
            Ok(_) => (),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
        // A foreign physical replay must not fail an already accepted original
        // root. Resolve this exact source before creating/mutating its ledger.
        let physical_source = (|| {
            physical.validate(
                &a,
                a.authority::<StoreInvocationAuthority>()
                    .ok_or_else(Error::forbidden)?,
            )?;
            if physical.lifetime.is_cancelled() {
                return Err(Error::forbidden());
            }
            let source = store.project_native_stream_with_peer(&credential, proof(&a)?)?;
            let id = ConnectionId::from_string(source.connection.runtime_connection_id);
            let bound = self
                .native
                .physical_source(&id)
                .map_err(|_| Error::forbidden())?;
            if !Arc::ptr_eq(&bound, &physical.binding) {
                return Err(Error::forbidden());
            }
            Ok(())
        })();
        if let Err(error) = physical_source {
            let _ = tx.send(Err(error));
            return;
        }
        let accepted =
            match proof(&a).and_then(|p| store.accept_native_stream_with_peer(&credential, p)) {
                Ok(record) => record,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
        if accepted.state != OperationState::Accepted {
            let _ = tx.send(Ok(crate::canonical_native::Reply {
                record: accepted,
                answered: false,
            }));
            return;
        }
        let ending = a.envelope().kind() == "connection.end";
        let quality = intent(a.envelope())
            .ok()
            .and_then(|wire| wire.quality_interval_ms())
            .is_some();
        let gates = a.envelope().kind() == "connection.update"
            && connect_protocol::native_gates::is_gate(&a.envelope().original()["payload"]);
        let codecs = a.envelope().kind() == "connection.update" && !quality && !gates;
        let advertising = a.envelope().kind() == "capability.advertise";
        let preflight = (|| {
            let p = proof(&a)?;
            let authority = a
                .authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?;
            physical.validate(&a, authority)?;
            if physical.lifetime.is_cancelled() {
                return Err(Error::forbidden());
            }
            let source = store.project_native_stream_with_peer(&credential, p)?;
            let id = ConnectionId::from_string(source.connection.runtime_connection_id.clone());
            let bound = self
                .native
                .physical_source(&id)
                .map_err(|_| Error::forbidden())?;
            if !Arc::ptr_eq(&bound, &physical.binding) {
                return Err(Error::forbidden());
            }
            if ending || quality || advertising {
                return Ok((id, None));
            }
            let plan = if codecs {
                self.native
                    .plan_native_codec_update(&id, &a)
                    .map(Plan::Codecs)
            } else if gates {
                self.native.plan_native_control(&id, &a).map(Plan::Gates)
            } else {
                self.native
                    .plan_native_subscriptions(&self.core, &id, &a)
                    .map(Plan::Subscriptions)
            }
            .map_err(|e| {
                tracing::debug!(?e, "Native stream plan refused");
                match e {
                    rvoip_uctp::v2::native_negotiation::NativeOfferError::Conflict => {
                        Error::new("conflict", "Native stream revision or catalog conflicts")
                    }
                    rvoip_uctp::v2::native_negotiation::NativeOfferError::Unsupported => {
                        if a.envelope().kind() == "stream.subscribe" {
                            return Error::new(
                                "incompatible-capabilities",
                                "Subscription capabilities have no usable intersection",
                            );
                        }
                        Error::new(
                            "unsupported-media-profile",
                            "Native control profile is not supported",
                        )
                    }
                    rvoip_uctp::v2::native_negotiation::NativeOfferError::NotReady
                        if a.envelope().kind() == "stream.subscribe" =>
                    {
                        Error::new("not-found", "Subscription source is unavailable")
                    }
                    _ => Error::forbidden(),
                }
            })?;
            if let Plan::Subscriptions(subscription) = &plan {
                if a.envelope().kind() == "stream.subscribe" {
                    let kinds = subscription
                        .default_media()
                        .iter()
                        .map(|kind| {
                            match kind {
                                rvoip_core::stream_lifecycle::Medium::Audio => "audio",
                                rvoip_core::stream_lifecycle::Medium::Video => "video",
                                rvoip_core::stream_lifecycle::Medium::Screen => "screen",
                                rvoip_core::stream_lifecycle::Medium::Data => "data",
                            }
                            .to_owned()
                        })
                        .collect::<Vec<_>>();
                    store.validate_native_subscription_sources_with_peer(&credential, p, &kinds)?;
                }
            }
            if let Some(media) = plan.media() {
                crate::peer_authority::bind_native_stream_plan(&a, media)?;
            }
            Ok((id, Some(plan)))
        })();
        let (id, plan) = match preflight {
            Ok(v) => v,
            Err(e) => {
                let result = proof(&a)
                    .and_then(|p| {
                        store.refuse_native_stream_before_effect_with_peer(&credential, p, e)
                    })
                    .map(|record| crate::canonical_native::Reply {
                        record,
                        answered: false,
                    });
                let _ = tx.send(result);
                return;
            }
        };
        let (lease, started) =
            match proof(&a).and_then(|p| store.begin_native_stream_with_peer(&credential, p)) {
                Ok(Some(value)) => value,
                Ok(None) => {
                    let result = proof(&a)
                        .and_then(|p| store.accept_native_stream_with_peer(&credential, p))
                        .map(|record| crate::canonical_native::Reply {
                            record,
                            answered: false,
                        });
                    let _ = tx.send(result);
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
        let seconds = (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let ready = self
            .native_preflight(&a, deadline, || {
                let invocation = plan
                    .clone()
                    .map(|plan| {
                        self.media
                            .stream_invocation(a.clone(), plan, &lease, deadline)
                    })
                    .transpose()?;
                store.validate_native_stream_attempt(&credential, proof(&a)?, &lease)?;
                physical.validate(
                    &a,
                    a.authority::<StoreInvocationAuthority>()
                        .ok_or_else(Error::forbidden)?,
                )?;
                if physical.lifetime.is_cancelled() || self.stop.is_cancelled() {
                    return Err(Error::forbidden());
                }
                let quality_invocation = if quality {
                    Some(crate::canonical_native_quality::Invocation::prepare(
                        &self,
                        a.clone(),
                        &lease,
                        &physical,
                        deadline,
                    )?)
                } else {
                    None
                };
                let advertisement = if advertising {
                    Some(crate::canonical_native_capabilities::Received::prepare(
                        &self,
                        a.clone(),
                        &lease,
                        &physical,
                        deadline,
                    )?)
                } else {
                    None
                };
                Ok((invocation, quality_invocation, advertisement))
            })
            .await;
        let (invocation, quality_invocation, advertisement) = match ready {
            Ok(v) => v,
            Err(e) => {
                let result = store
                    .fail_native_stream_without_effect(lease, e)
                    .map(|record| crate::canonical_native::Reply {
                        record,
                        answered: false,
                    });
                let _ = tx.send(result);
                return;
            }
        };
        // Media and quality receipts capture actual returned outcomes. A
        // capability receipt instead captures the verified declaration above;
        // replacement is metadata, committed atomically with its operation.
        // Possible media IO stays uncertain; definite local refusal does not.
        let mut capability = None;
        let receipt = if let Some(received) = &advertisement {
            if !received.current() {
                let result = store
                    .fail_native_stream_without_effect(lease, Error::forbidden())
                    .map(|record| crate::canonical_native::Reply {
                        record,
                        answered: false,
                    });
                let _ = tx.send(result);
                return;
            }
            match self
                .native
                .declare_native_capabilities(&self.core, &id, &a, received)
            {
                Ok(observed) => {
                    capability = Some((
                        Arc::new(observed),
                        received.entry(),
                        lease.record_id().to_owned(),
                    ));
                }
                Err(error) => {
                    // This synchronous local metadata setter returns errors only
                    // before commit. Interrupted calls remain uncertain instead.
                    let problem = match error {
                        rvoip_uctp::v2::native_negotiation::NativeOfferError::Conflict => {
                            Error::new("conflict", "Native declaration revision conflicts")
                        }
                        rvoip_uctp::v2::native_negotiation::NativeOfferError::Unsupported => {
                            Error::new(
                                "unsupported-media-profile",
                                "Native declaration profile is unsupported",
                            )
                        }
                        rvoip_uctp::v2::native_negotiation::NativeOfferError::Capacity => {
                            Error::new("capacity-exceeded", "Native declaration capacity exceeded")
                        }
                        _ => Error::forbidden(),
                    };
                    let result =
                        store
                            .fail_native_stream_without_effect(lease, problem)
                            .map(|record| crate::canonical_native::Reply {
                                record,
                                answered: false,
                            });
                    let _ = tx.send(result);
                    return;
                }
            }
            lease.advertisement_received()
        } else if let Some(invocation) = quality_invocation {
            match self.native.configure_native_quality(&id, &a, &invocation) {
                Ok(interval_ms) => lease.quality_configured(interval_ms),
                Err(error) => {
                    // Returned errors precede this synchronous local setter's
                    // commit. A panic/interruption still leaves its attempt
                    // uncertain rather than reconstructing or repeating it.
                    let problem = match error {
                        rvoip_uctp::v2::native_negotiation::NativeOfferError::Conflict => {
                            Error::new("conflict", "Native quality revision conflicts")
                        }
                        _ => Error::forbidden(),
                    };
                    let result =
                        store
                            .fail_native_stream_without_effect(lease, problem)
                            .map(|record| crate::canonical_native::Reply {
                                record,
                                answered: false,
                            });
                    let _ = tx.send(result);
                    return;
                }
            }
        } else if let Some(invocation) = invocation {
            let outcome = tokio::select! {
                r=async {
                    if gates {self.native.control_native_media(&id,a.clone(),invocation.clone(),deadline).await.map(Observed::Gates)}
                    else if codecs {self.native.renegotiate_native_codecs(&id,a.clone(),invocation.clone(),deadline).await.map(Observed::Codecs)}
                    else {self.native.subscribe_native_media(&self.core,&id,a.clone(),invocation.clone(),deadline).await.map(Observed::Streams)}
                }=>r,
                _=self.stop.cancelled()=>Err(rvoip_uctp::v2::native_negotiation::NativeOfferError::Cancelled),
            };
            if outcome.is_ok() {
                invocation.remember_confirmed_selection();
            }
            drop(invocation);
            match outcome {
                Ok(Observed::Gates(outcome)) => {
                    match gate_streams(&outcome.streams, lease.generation()) {
                        Ok(streams) => lease.gates_committed(outcome.revision, streams),
                        Err(error) => {
                            let _ = store.native_stream_attempt_uncertain(&lease);
                            let _ = tx.send(Err(error));
                            return;
                        }
                    }
                }
                Ok(Observed::Streams(outcome)) => {
                    lease.committed(outcome.revision, outcome.resources_changed)
                }
                Ok(Observed::Codecs(outcome)) => match codec_bindings(&outcome.bindings) {
                    Ok(bindings) => lease.codecs_committed(outcome.revision, bindings),
                    Err(error) => {
                        let _ = store.native_stream_attempt_uncertain(&lease);
                        let _ = tx.send(Err(error));
                        return;
                    }
                },
                Err(error) => {
                    tracing::debug!(?error, "Native stream engine outcome remains uncertain");
                    let result = store.native_stream_attempt_uncertain(&lease).map(|record| {
                        crate::canonical_native::Reply {
                            record,
                            answered: false,
                        }
                    });
                    let _ = tx.send(result);
                    return;
                }
            }
        } else {
            let outcome = tokio::select! {
                r=self.core.end_connection(id.clone(),rvoip_core::EndReason::Normal)=>r.map_err(|_| Error::new("reconciliation-required", "Native termination is uncertain")),
                _=self.stop.cancelled()=>Err(Error::forbidden()),
                _=tokio::time::sleep_until(deadline)=>Err(Error::new("reconciliation-required", "Native termination is uncertain")),
            };
            if outcome.is_err() || self.native.is_connection_live(&id) {
                let result = store.native_stream_attempt_uncertain(&lease).map(|record| {
                    crate::canonical_native::Reply {
                        record,
                        answered: false,
                    }
                });
                let _ = tx.send(result);
                return;
            }
            let absent = crate::canonical_native::Absent {
                native: self.native.clone(),
                id,
                generation: lease.generation(),
            };
            lease.ended(&absent)
        };
        let receipt = match receipt {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let first = store.complete_native_stream(&receipt);
        drop(advertisement);
        match first {
            Ok(record) => {
                let _ = tx.send(Ok(crate::canonical_native::Reply {
                    record,
                    answered: false,
                }));
                drop(a);
                drop(physical);
                drop(credential);
                if let Some((declaration, entry, record)) = capability.take() {
                    self.publish_capability(declaration, entry, record).await;
                }
                return;
            }
            Err(error) => {
                tracing::debug!(code=%error.code,"Native stream outcome awaits durable recording");
                let _ = tx.send(Ok(crate::canonical_native::Reply {
                    record: started,
                    answered: false,
                }));
            }
        }
        // Original command readers are bounded to control. This owned job now
        // retains only the captured engine receipt while storage recovers.
        drop(a);
        drop(physical);
        drop(credential);
        loop {
            tokio::select! {_=self.stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            match store.complete_native_stream(&receipt) {
                Ok(_) => {
                    if let Some((declaration, entry, record)) = capability.take() {
                        self.publish_capability(declaration, entry, record).await;
                    }
                    return;
                }
                Err(e) if e.code == "forbidden" => return,
                Err(_) => (),
            }
        }
    }
}
