//! One owned native exchange, real engine observations and isolated cleanup.
use crate::{
    canonical_media::Invocation,
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::transitions::{OperationRecord, OperationState};
use connect_protocol::{Error, Result};
use connect_store::{
    NativeOfferLease, NativeOfferObservation, NativeOfferPreflightRefusal,
    NativeOfferRuntimeObservation, NativeReadyObservation,
};
use rvoip_core::{
    adapter::ConnectionAdapter,
    ids::{ConnectionId, ConversationId, ParticipantId, SessionId},
    orchestrator::peer_native::{PeerNativeAuthority, PeerNativeAuthorityLease, PeerNativeIntent},
    participant_transitions::{ParticipantChange, ParticipantMutation},
    staged_streams::{MediaOwner, StreamController},
    stream_lifecycle::{EndpointMediaCapabilities, MediaCapability, MediaFormat, Medium},
};
use rvoip_uctp::{
    staged_media::StagedRtpScope,
    v2::{
        auth_dispatch::AuthorizedEnvelope,
        native_negotiation::{NativeMediaActivation, NativeMediaProfile},
        Scope,
    },
};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
/// Preserve temporary metadata unavailability while installing observers on an
/// already admitted original route. The caller retains its original deadline;
/// no negotiation, media effect, or physical writer is repeated by this retry.
pub(crate) fn registration_error(
    error: rvoip_uctp::v2::native_negotiation::NativeOfferError,
) -> Error {
    use rvoip_uctp::v2::{auth_dispatch::DispatchError, native_negotiation::NativeOfferError};
    match error {
        NativeOfferError::NotReady
        | NativeOfferError::Dispatch(DispatchError::StorageUnavailable) => {
            Error::new("authority-busy", "Native observer metadata is busy")
        }
        _ => Error::forbidden(),
    }
}
pub(crate) fn intent(
    e: &rvoip_uctp::v2::Envelope,
) -> Result<connect_protocol::connection_control::OfferIntent> {
    if e.kind() != "connection.offer" || e.scope(Scope::Reply).is_some() {
        return Err(Error::forbidden());
    }
    let required = |scope| {
        e.scope(scope)
            .map(str::to_owned)
            .ok_or_else(|| Error::new("invalid-request", "Native offer scopes are required"))
    };
    let mut intent = connect_protocol::connection_control::OfferIntent {
        operation_id: required(Scope::Operation)?,
        conversation_id: required(Scope::Conversation)?,
        session_id: required(Scope::Session)?,
        connection_id: required(Scope::Connection)?,
        by_participant: e.original()["payload"]["by_participant"]
            .as_str()
            .map(str::to_owned),
        expected_revision: e.expected_revision(),
        offer: e.original()["payload"].clone(),
    };
    intent.normalize()?;
    Ok(intent)
}
fn denied<T>(_: T) -> Error {
    Error::forbidden()
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|a| a.peer_guard())
        .ok_or_else(Error::forbidden)
}
pub(crate) fn base_profile() -> Result<NativeMediaProfile> {
    use rvoip_core::capability::AssuranceLevel;
    let encoded = |name: &str, clock_rate_hz, channels| MediaFormat::Encoded {
        name: name.into(),
        clock_rate_hz,
        channels,
        fmtp: None,
    };
    let caps = vec![
        MediaCapability {
            medium: Medium::Audio,
            formats: vec![
                encoded("opus", 48000, 1),
                encoded("pcmu", 8000, 1),
                encoded("pcma", 8000, 1),
            ],
        },
        MediaCapability {
            medium: Medium::Video,
            formats: vec![encoded("vp8", 90000, 0)],
        },
        MediaCapability {
            medium: Medium::Screen,
            formats: vec![encoded("vp8", 90000, 0)],
        },
        MediaCapability {
            medium: Medium::Data,
            formats: vec![
                MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Text),
                MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Json),
                MediaFormat::ReliableOrdered(rvoip_core::capability::DataProtocol::Binary),
            ],
        },
    ];
    NativeMediaProfile::new(
        EndpointMediaCapabilities {
            send: caps.clone(),
            receive: caps,
            max_streams: 32,
            renegotiation: true,
        },
        AssuranceLevel::Identified,
        AssuranceLevel::Identified,
    )
    .map_err(denied)
}
struct Preparing {
    authority: PeerNativeAuthority,
    _read: Arc<connect_store::NativeMediaRead>,
}
impl PeerNativeAuthorityLease for Preparing {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.authority
    }
}
struct Ready {
    native: Arc<NativeMediaActivation>,
    controller: Arc<StreamController>,
    owner: MediaOwner,
    runtime: Arc<Runtime>,
    physical: Arc<Physical>,
    expected: NativeReadyObservation,
}
impl NativeOfferRuntimeObservation for Ready {
    fn observe(&self) -> Result<NativeOfferObservation> {
        if self.physical.lifetime.is_cancelled()
            || self.controller.revision() != self.expected.stream_revision
        {
            return Err(Error::forbidden());
        }
        let owner = self.native.current(self.owner.clone()).map_err(denied)?;
        if owner.connection.connection().as_str() != self.expected.runtime_connection_id
            || owner.connection.generation() != self.expected.generation
            || self.native.confirmed_generation() != Some(self.expected.generation)
        {
            return Err(Error::forbidden());
        }
        let session = self
            .runtime
            .core
            .session(&SessionId::from_string(self.expected.session_id.clone()))
            .ok_or_else(Error::forbidden)?;
        let session = session
            .try_read()
            .map_err(|_| Error::new("authority-busy", "Managed Session observation is busy"))?;
        if !session.connections.contains_key(&ConnectionId::from_string(
            self.expected.runtime_connection_id.clone(),
        )) {
            return Err(Error::forbidden());
        }
        Ok(NativeOfferObservation::Ready(self.expected.clone()))
    }
}
pub(crate) struct Absent {
    pub(crate) natives: [Arc<rvoip_uctp::v2::native::CanonicalNativeAdapter>; 2],
    pub(crate) id: ConnectionId,
    pub(crate) generation: u64,
}
pub(crate) struct Recovery(pub(crate) [Arc<rvoip_uctp::v2::native::CanonicalNativeAdapter>; 2]);
impl connect_store::NativeOfferRecoveryObservation for Recovery {
    fn observe_retired(&self, id: &str, generation: u64) -> Result<NativeOfferObservation> {
        let id = ConnectionId::from_string(id.to_owned());
        if self.0.iter().any(|native| native.is_connection_live(&id)) {
            return Err(Error::new(
                "reconciliation-required",
                "Native route cleanup is not confirmed",
            ));
        }
        Ok(NativeOfferObservation::Retired {
            runtime_connection_id: id.as_str().into(),
            generation,
        })
    }
}
impl NativeOfferRuntimeObservation for Absent {
    fn observe(&self) -> Result<NativeOfferObservation> {
        if self
            .natives
            .iter()
            .any(|native| native.is_connection_live(&self.id))
        {
            return Err(Error::new(
                "reconciliation-required",
                "Native route cleanup is not confirmed",
            ));
        }
        Ok(NativeOfferObservation::Retired {
            runtime_connection_id: self.id.as_str().into(),
            generation: self.generation,
        })
    }
}
pub(crate) struct Reply {
    pub record: OperationRecord,
    pub answered: bool,
}
impl Runtime {
    pub(crate) async fn native_offer(
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
                return Err(Error::new(
                    "busy",
                    "Native connection workers are at capacity",
                ));
            }
            work.spawn(async move {
                runtime.native_worker(a, physical, credential, tx).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    async fn native_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) {
        let native = match self.native_adapter(physical.binding.transport()) {
            Ok(native) => native,
            Err(error) => {
                let _ = tx.send(Err(error));
                return;
            }
        };
        let mut lease = None;
        let mut stage = "source";
        let outcome = self
            .native_start(
                a.clone(),
                physical.clone(),
                &credential,
                &mut lease,
                &mut stage,
            )
            .await;
        if let Err(error) = &outcome {
            tracing::debug!(stage,code = %error.code, "Native offer did not complete");
        }

        let succeeded = outcome.is_ok();
        let _ = tx.send(outcome);
        // No idle original command/reader survives negotiation or durable commit.
        drop(a);
        let Some(lease) = lease else { return };
        let lease = Arc::new(lease);
        let id = ConnectionId::from_string(lease.runtime_connection_id().to_owned());
        if succeeded {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if self.stop.is_cancelled()
                    || physical.lifetime.is_cancelled()
                    || !native.is_connection_live(&id)
                {
                    break;
                }
                let origin = self.media.origin(&id);
                let Some(origin) = origin else { break };
                match self.media.store.try_native_connection_current(&origin) {
                    Ok(true) => (),
                    Ok(_) => break,
                    Err(e) if e.code == "forbidden" => break,
                    // Fresh packet grants still refuse unavailable state. An
                    // unavailable policy read is not proof that the source
                    // ended; retain custody until definite retirement or EOF.
                    Err(_) => (),
                }
                self.reconcile_connection_role(&id).await;
            }
        }
        // Actual end is scoped to this Core route. An unconfirmed drain keeps
        // custody/quota; no other Connection or the physical peer is closed.
        loop {
            if native.is_connection_live(&id) {
                let _ = self
                    .core
                    .end_connection(id.clone(), rvoip_core::adapter::EndReason::Normal)
                    .await;
            }
            // Cleanup takes current authority fences and may wait behind a live
            // command's reader. Blocking this executor prevents that command's
            // reply from releasing its reader, starving unrelated peers too.
            // The tracked native worker awaits each bounded Store attempt and
            // retains its exact opaque lease until actual cleanup is settled.
            let store = self.media.store.clone();
            let natives = self.native_adapters();
            let retired = id.clone();
            let attempt = lease.clone();
            let settled = tokio::task::spawn_blocking(move || {
                let absent = Absent {
                    natives,
                    id: retired,
                    generation: attempt.generation(),
                };
                store.settle_native_offer_cleanup(&attempt, &absent)
            })
            .await;
            if matches!(settled, Ok(Ok(()))) {
                break;
            }
            if self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.media.remove(&id);
    }
    /// Retry only synchronous metadata preflight. Callers must not put Core or
    /// transport effects in this closure: a lost effect result is never retried.
    pub(crate) async fn native_preflight<T>(
        &self,
        authorized: &AuthorizedEnvelope,
        deadline: Instant,
        mut read: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        loop {
            if self.stop.is_cancelled()
                || authorized.invocation_cancellation().is_cancelled()
                || Instant::now() >= deadline
            {
                return Err(Error::forbidden());
            }
            match read() {
                Err(error) if error.code == "authority-busy" => {
                    tokio::select! {
                        biased;
                        _ = self.stop.cancelled() => return Err(Error::forbidden()),
                        _ = tokio::time::sleep_until(deadline) => return Err(Error::forbidden()),
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {},
                    }
                }
                result => return result,
            }
        }
    }
    async fn native_source_read(
        &self,
        origin: &connect_store::NativeMediaOrigin,
        a: &AuthorizedEnvelope,
        deadline: Instant,
    ) -> Result<Arc<connect_store::NativeMediaRead>> {
        loop {
            match self
                .media
                .store
                .native_media_read_with_peer(origin, None, proof(a)?)
            {
                Ok(read) => return Ok(Arc::new(read)),
                Err(error)
                    if error.code == "authority-busy"
                        && Instant::now() < deadline
                        && !a.invocation_cancellation().is_cancelled() =>
                {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => return Err(error),
            }
        }
    }
    async fn native_start(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
        lease_slot: &mut Option<NativeOfferLease>,
        stage: &mut &'static str,
    ) -> Result<Reply> {
        if a.envelope().original()["payload"]
            .get("replacement_preparation")
            .is_some()
        {
            if a.envelope().original()["payload"]["replacement_preparation"]["profile"]
                == "connect-replacement-media-v1"
            {
                return self.start_native_replacement(a, physical, credential).await;
            }
            return self
                .allocate_native_replacement(a, physical, credential)
                .await;
        }
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;

        let transport = physical.binding.transport();
        let native = self.native_adapter(transport)?;
        let endpoint_transport = crate::canonical_runtime::endpoint_transport(transport)?;
        *stage = "prepare-status";
        let record = self.media.store.prepare_native_offer_with_peer(
            credential,
            proof(&a)?,
            physical.binding.media().map(|_| endpoint_transport.clone()),
        )?;
        if record.state != OperationState::Accepted {
            return Ok(Reply {
                record,
                answered: false,
            });
        }
        if physical.binding.media().is_none() {
            return Err(Error::new(
                "unsupported-media-profile",
                "This physical transport negotiated canonical control only",
            ));
        }
        *stage = "profile";
        let projection = self
            .media
            .store
            .project_connection_with_peer(credential, proof(&a)?)?;
        if projection.endpoint_transport != endpoint_transport {
            return Err(Error::forbidden());
        }
        let base = base_profile()?;
        if crate::canonical_media::offer_plan(&projection, &base, a.principal()).is_err() {
            let record = self
                .media
                .store
                .refuse_native_offer_before_effect_with_peer(
                    credential,
                    proof(&a)?,
                    NativeOfferPreflightRefusal::UnsupportedProfile,
                )?;
            return Ok(Reply {
                record,
                answered: false,
            });
        }
        *stage = "begin";
        let Some((lease, _)) = self
            .media
            .store
            .begin_native_offer_with_peer(credential, proof(&a)?)?
        else {
            return Ok(Reply {
                record: self.media.store.prepare_native_offer_with_peer(
                    credential,
                    proof(&a)?,
                    physical.binding.media().map(|_| endpoint_transport.clone()),
                )?,
                answered: false,
            });
        };
        *lease_slot = Some(lease);
        let lease = lease_slot.as_ref().ok_or_else(Error::storage)?;
        let remaining = (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = Instant::now() + Duration::from_secs(remaining);
        *stage = "origin";
        let origin = self
            .native_preflight(&a, deadline, || {
                self.media
                    .store
                    .native_media_origin_with_peer(credential, proof(&a)?, lease)
            })
            .await?;
        let read = self.native_source_read(&origin, &a, deadline).await?;
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();

        *stage = "attach";
        self.attach(
            &read.target().session,
            a.principal(),
            peer.clone(),
            authority.retained(),
        )?;

        *stage = "register";
        let entry = self
            .media
            .register(origin, a.principal().clone(), &base, &read)
            .map_err(denied)?;
        let sid = SessionId::from_string(entry.origin.session_id().to_owned());
        let id = ConnectionId::from_string(entry.origin.runtime_connection_id().to_owned());
        let owner = entry
            .owner(read.target(), a.principal(), false)
            .map_err(denied)?;
        let view = Preparing {
            authority: PeerNativeAuthority {
                actor: owner.member.clone(),
                endpoint: owner.endpoint.clone(),
                may_prepare: true,
            },
            _read: read.clone(),
        };

        *stage = "prepare-engine";
        self.core
            .prepare_peer_native_connection(
                peer.clone(),
                a.principal(),
                PeerNativeIntent {
                    session: sid.clone(),
                    connection: id.clone(),
                    transport,
                },
                &view,
                physical.binding.clone(),
            )
            .map_err(denied)?
            .retain(a.principal(), &view)
            .map_err(denied)?;
        let invocation = Arc::new(Invocation {
            policy: self.media.clone(),
            connection: entry.clone(),
            authorized: a.clone(),
            read: read.clone(),
            participant_revision: self.core.participant_revision(&sid).map_err(denied)?,
        });

        *stage = "configure";
        let scope = StagedRtpScope {
            session: sid.clone(),
            connection: id.clone(),
            owner: a.principal().ownership_key(),
            wire_conversation: Some(entry.origin.conversation_id().into()),
            wire_session: entry.origin.session_id().into(),
            wire_connection: entry.origin.connection_id().into(),
        };
        let observer = Arc::new(
            self.native_preflight(&a, deadline, || {
                crate::canonical_media::data_writes::Observer::new(
                    self.media.store.clone(),
                    &entry.origin,
                    scope.clone(),
                )
            })
            .await?,
        );
        let controller = native
            .configure_negotiated_media_with_held(
                &id,
                &a,
                ConversationId::from_string(entry.origin.conversation_id().to_owned()),
                scope,
                self.media.clone(),
                entry.profile.clone(),
                Some(observer),
                32,
                invocation.as_ref(),
            )
            .map_err(denied)?;
        let activation = native.native_media_activation(&id, &a).map_err(denied)?;
        entry.set_activation(activation.clone()).map_err(denied)?;
        *stage = "negotiate";
        let outcome = native
            .negotiate_native_offer(&id, a.clone(), invocation.clone(), deadline)
            .await
            .map_err(|error| {
                tracing::debug!(?error, "Native engine negotiation failed");
                denied(error)
            })?;
        *stage = "confirmed-source";
        let current = self.native_source_read(&entry.origin, &a, deadline).await?;
        let role = current.target().presence_role.clone();
        let private = current.target().private_consultation.clone();
        let handoff_preparation = current.target().handoff_preparation;
        let isolated = private.is_some() || handoff_preparation;
        let admission_operation = if isolated {
            lease.record_id()
        } else {
            lease.operation_id()
        };
        if isolated {
            *stage = "isolated-mute";
            native
                .mute_native_audio_for_private_admission(&id, &a, invocation.clone(), deadline)
                .await
                .map_err(denied)?;
        }
        // This private revision is computed at admission, not held across the
        // remote exchange. A conflicting concurrent admission can retry only
        // before the controller records/commits this stable mutation ID.

        *stage = "admit";
        let mut observed = None;
        for _ in 0..8 {
            let mut i = (*invocation).clone();
            i.read = current.clone();
            i.participant_revision = self.core.participant_revision(&sid).map_err(denied)?;
            let mutation = ParticipantMutation {
                operation_id: admission_operation.into(),
                correlation_id: a.envelope().id().into(),
                expected_revision: i.participant_revision,
                participant: ParticipantId::from_string(entry.origin.participant_id().to_owned()),
                change: ParticipantChange::Admit {
                    connection: id.clone(),
                    role_override: Some(crate::canonical_runtime::participant_role(&role)),
                },
            };
            let result = if isolated {
                self.core
                    .admit_private_peer_session_connection_with_held(
                        peer.clone(),
                        &sid,
                        a.principal().clone(),
                        mutation,
                        deadline,
                        a.invocation_cancellation(),
                        &i,
                    )
                    .await
            } else {
                self.core
                    .admit_peer_session_connection_with_held(
                        peer.clone(),
                        &sid,
                        a.principal().clone(),
                        mutation,
                        deadline,
                        a.invocation_cancellation(),
                        &i,
                    )
                    .await
            };
            match result {
                Ok(receipt)=>{observed=Some(receipt);break},
                Err(rvoip_core::orchestrator::participant_control::ParticipantControlError::Transition(rvoip_core::participant_transitions::ParticipantTransitionError::Conflict))=>continue,
                Err(error)=>return Err(denied(error)),
            }
        }
        let admitted = observed
            .ok_or_else(|| Error::new("conflict", "Concurrent native admission did not settle"))?;
        if !admitted.admitted.contains(&id)
            || admitted.operation_id != admission_operation
            || admitted.membership_epoch != entry.origin.membership_epoch()
        {
            return Err(Error::forbidden());
        }
        *stage = "observed-owner";
        let owner = entry
            .owner(current.target(), a.principal(), false)
            .map_err(denied)?;
        let owner = activation.current(owner).map_err(denied)?;
        let evidence = Ready {
            native: activation,
            controller: controller.clone(),
            owner,
            runtime: self.clone(),
            physical,
            expected: NativeReadyObservation {
                private_consultation: private,
                handoff_preparation,
                conversation_id: entry.origin.conversation_id().into(),
                session_id: entry.origin.session_id().into(),
                runtime_connection_id: id.as_str().into(),
                participant_id: admitted.participant.as_str().into(),
                membership_epoch: admitted.membership_epoch,
                endpoint_id: current.target().connection.endpoint_id.clone(),
                device_id: current.target().device_id.clone(),
                generation: entry.origin.generation(),
                stream_revision: controller.revision(),
                participant_revision: admitted.revision,
                streams: outcome.accepted.iter().map(|v| v.as_str().into()).collect(),
            },
        };

        *stage = "ready-commit";
        let record = loop {
            match self.media.store.complete_native_offer_with_peer(
                credential,
                lease,
                proof(&a)?,
                &evidence,
            ) {
                Ok(record) => break record,
                Err(error)
                    if error.code == "authority-busy"
                        && Instant::now() < deadline
                        && !a.invocation_cancellation().is_cancelled() =>
                {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => return Err(error),
            }
        };
        if handoff_preparation {
            self.queue_handoff_preparation(entry.clone(), evidence.physical.lifetime.clone());
        }
        *stage = "subscription-reconciliation";
        self.native_preflight(&a, deadline, || self.enable_native_reconciliation(&id, &a))
            .await?;
        *stage = "lifecycle-registration";
        self.native_preflight(&a, deadline, || {
            self.enable_native_lifecycle(&id, &a, &evidence.physical)
        })
        .await?;
        *stage = "capability-registration";
        // Capturing listener metadata uses Core try-locks. Contention with the
        // real lifecycle worker is transient; retain the original offer and
        // bounded admission rather than retiring a successfully admitted route.
        self.native_preflight(&a, deadline, || {
            self.enable_capability_delivery(&id, &a, &evidence.physical)
        })
        .await?;
        *stage = "dtmf-registration";
        self.native_preflight(&a, deadline, || {
            self.enable_native_dtmf(&id, &a, &evidence.physical)
        })
        .await?;
        Ok(Reply {
            record,
            answered: true,
        })
    }
}
