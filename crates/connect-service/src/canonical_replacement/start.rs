//! Actual signed target negotiation, exact replacement resources and initial mute.
use super::allocation::AllocatedReplacement;
use super::media::{StartInvocation, TargetPolicy};
use super::*;
use crate::{canonical_native::Reply, canonical_runtime::Physical, StoreInvocationAuthority};
use rvoip_core::{ids::ConversationId, staged_streams::StreamController};
use rvoip_uctp::{staged_media::StagedRtpScope, v2::auth_dispatch::AuthorizedEnvelope};
use std::sync::OnceLock;

pub(super) struct StartedReplacement {
    pub receipt: Arc<connect_store::NativeReplacementStarted>,
    pub policy: Arc<TargetPolicy>,
    pub controller: Arc<StreamController>,
    pub resource_streams: Vec<String>,
    pub lifecycle: super::webrtc::ReplacementLifecycle,
    pub(super) intent: connect_protocol::connection_control::OfferIntent,
    pub commit: std::sync::Mutex<Option<super::movement::Capture>>,
    pub phases: std::sync::Mutex<Vec<Arc<connect_store::NativeReplacementMediaCapture>>>,
    pub preparation:
        OnceLock<Arc<rvoip_core::staged_streams::endpoint_preparation::AudioEndpointPreparation>>,
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|a| a.peer_guard())
        .ok_or_else(Error::forbidden)
}
fn start_refused(stage: &'static str, error: Error) -> Error {
    tracing::debug!(target: "connect_service::canonical_replacement", stage, code=%error.code,
        "Original replacement start refused before resource preparation");
    error
}
impl Runtime {
    pub(crate) async fn start_native_replacement(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Reply> {
        tracing::debug!(target: "connect_service::canonical_replacement", stage="start-entry",
            "Original replacement start entered");
        // These first checks and exact historical replay publish no Core
        // owner/effect. The actual begin below already releases both original
        // authority aliases, then reacquires current authority and coordinator
        // custody before validating and starting the once-only effect.
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;
        let native = self.native_adapter(physical.binding.transport())?;
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
                    Error::new("not-ready", "Owned replacement preparation is not live")
                })?
        };
        let allocated = {
            saved
                .allocated
                .lock()
                .map_err(|_| Error::storage())?
                .clone()
                .ok_or_else(|| {
                    Error::new(
                        "not-ready",
                        "Signed target must allocate the original reservation first",
                    )
                })?
        };
        if !allocated
            .physical
            .upgrade()
            .is_some_and(|p| Arc::ptr_eq(&p, &physical))
        {
            return Err(Error::forbidden());
        }
        let previous = {
            allocated
                .started
                .lock()
                .map_err(|_| Error::storage())?
                .clone()
        };
        if let Some(previous) = previous {
            if previous.intent != intent {
                return Err(Error::new(
                    "conflict",
                    "Replacement media start differs from its original intent",
                ));
            }
            // Exact historical replay performs no new Core scene mutation.
            self.record_started_replacement_with_peer(&previous, a.clone())
                .await?;
            let store = self.media.store.clone();
            let a = a.clone();
            let key = credential.to_owned();
            let receipt = previous.receipt.clone();
            let record = store_call(move || {
                store.replay_native_replacement_start_with_peer(&receipt, &key, proof(&a)?)
            })
            .await?;
            return Ok(Reply {
                record,
                answered: false,
            });
        }
        if !allocated.recorded.load(Ordering::Acquire) {
            return Err(Error::new(
                "not-ready",
                "Original native allocation capture is not yet recorded",
            ));
        }
        // This phase has no start effect. The existing begin path obtains
        // coordinator custody and checks the exact current target/plan again
        // before the once-only mutation.
        let root_cancel = saved.handle.cancellation();
        let invocation_cancel = a.invocation_cancellation();
        let read_until = saved.deadline.min(
            tokio::time::Instant::now()
                + Duration::from_secs(
                    (proof(&a)?.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
                ),
        );
        if self.stop.is_cancelled()
            || physical.lifetime.is_cancelled()
            || root_cancel.is_cancelled()
            || invocation_cancel.is_cancelled()
            || tokio::time::Instant::now() >= read_until
        {
            return Err(Error::forbidden());
        }
        let read = {
            let wait = connect_store::AuthorityWait::new(
                read_until.saturating_duration_since(tokio::time::Instant::now()),
            );
            let _cancel_wait = wait.cancel_on_drop();
            let store = self.media.store.clone();
            let original = a.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            let read_wait = wait.clone();
            let read_call = self.owned_store_call(move || {
                store.native_replacement_start_read_with_peer_wait(
                    &receipt,
                    &key,
                    proof(&original)?,
                    &read_wait,
                )
            });
            tokio::pin!(read_call);
            let read = tokio::select! { biased;
                _ = self.stop.cancelled() => None,
                _ = physical.lifetime.cancelled() => None,
                _ = root_cancel.cancelled() => None,
                _ = invocation_cancel.cancelled() => None,
                _ = tokio::time::sleep_until(read_until) => None,
                result = &mut read_call => Some(result),
            };
            let read = match read {
                Some(result) => result.map_err(|error| start_refused("start-read", error))?,
                None => {
                    wait.cancel();
                    // Join the SAME original blocking owner; a late read is dropped
                    // here, never detached or allowed to begin after cancellation.
                    drop(read_call.await);
                    return Err(start_refused("start-read", Error::forbidden()));
                }
            };
            read
        };
        if self.stop.is_cancelled()
            || physical.lifetime.is_cancelled()
            || root_cancel.is_cancelled()
            || invocation_cancel.is_cancelled()
            || tokio::time::Instant::now() >= read_until
            || !read.current()
        {
            return Err(Error::forbidden());
        }
        physical.validate(
            &a,
            a.authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?,
        )?;
        let entries = self
            .media
            .replacement_preparation_entries(read.source())
            .map_err(|error| start_refused("start-entries", error))?;
        if crate::canonical_runtime::core_transport(&read.target().endpoint_transport)?
            != physical.binding.transport()
        {
            return Err(Error::forbidden());
        }
        let base = crate::canonical_native::base_profile()?;
        let (profile, plan) =
            crate::canonical_media::offer_plan(read.target(), &base, a.principal()).map_err(
                |_| {
                    Error::new(
                        "unsupported-media-profile",
                        "Replacement target has no accepted audio codec",
                    )
                },
            )?;
        let deadline = read_until.min(
            tokio::time::Instant::now()
                + Duration::from_secs(
                    (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
                ),
        );
        let mapped = super::movement::replacement_video(&saved.plan)?;
        let gate =
            // The captured start read bounds only dormant configure/negotiate/mute.
            // Decoder readiness owns the unchanged original reservation bound;
            // each private packet and Ready continuation still reacquires its
            // actual current Store authority. Never retain a five-second read
            // as the lifetime of the original preparation.
            super::readiness::ReadyGate::new_native(&read, &plan, saved.deadline, mapped.as_ref())?;
        let challenge = gate.native_challenge(record)?;
        let original_until = read.expires_at();
        let original_target = read.target().connection.clone();
        let participant = read.target().connection.participant_id.clone();
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();
        // Release both the original control and shared final-reply guards before
        // any pre-begin pause. This SAME signed offer retains only its original
        // bounded parameter-exchange capability; it cannot authorize media.
        drop(read);
        drop(entries);
        let original = Arc::try_unwrap(a).map_err(|_| {
            Error::new(
                "conflict",
                "Original replacement invocation is still shared",
            )
        })?;
        let a = Arc::new(
            original
                .into_resource_preparation(
                    participant,
                    plan.selected()
                        .iter()
                        .map(|s| s.requested.logical_id.as_str().to_owned())
                        .collect(),
                    deadline,
                )
                .map_err(|_| Error::forbidden())?,
        );
        let (attempt, read, start_origin, entries, admission, roles) = loop {
            if self.stop.is_cancelled()
                || physical.lifetime.is_cancelled()
                || root_cancel.is_cancelled()
                || a.invocation_cancellation().is_cancelled()
                || tokio::time::Instant::now() >= deadline
            {
                return Err(Error::forbidden());
            }
            // This policy call rechecks only the SAME already-signed command.
            // It does not send a frame, consume a signature or reserve an effect.
            let admission = match a.reauthorize_resource_preparation().await {
                Ok(admission) => Some(admission),
                Err(rvoip_uctp::v2::auth_dispatch::DispatchError::AuthorityUnavailable) => None,
                Err(_) => return Err(Error::forbidden()),
            };
            if let Some(admission) = admission {
                if root_cancel.is_cancelled()
                    || physical.lifetime.is_cancelled()
                    || !a.resource_activation_current(&admission)
                {
                    return Err(Error::forbidden());
                }
                let authority = admission
                    .authority::<StoreInvocationAuthority>()
                    .ok_or_else(Error::forbidden)?;
                physical.validate(&a, authority)?;
                if allocated
                    .physical
                    .upgrade()
                    .is_none_or(|p| !Arc::ptr_eq(&p, &physical))
                {
                    return Err(Error::forbidden());
                }
                // Never wait for the scene coordinator with a domain guard.
                if let Ok(roles) = self.role_projection.try_lock() {
                    let runtime = self.clone();
                    let current = admission.clone();
                    let receipt = allocated.receipt.clone();
                    let key = credential.to_owned();
                    let expected_target = original_target.clone();
                    let selected = plan.selected().to_vec();
                    let rejected = plan.rejected().to_vec();
                    let principal = a.principal().clone();
                    let resource = a.clone();
                    let original_root_cancel = root_cancel.clone();
                    let original_physical_cancel = physical.lifetime.clone();
                    let result = self
                        .owned_store_call(move || {
                            if original_root_cancel.is_cancelled()
                                || original_physical_cancel.is_cancelled()
                                || !resource.resource_activation_current(&current)
                            {
                                return Err(Error::forbidden());
                            }
                            let proof = current
                                .authority::<StoreInvocationAuthority>()
                                .and_then(|a| a.peer_guard())
                                .ok_or_else(Error::forbidden)?;
                            let mut read = match runtime
                                .media
                                .store
                                .native_replacement_start_read_with_peer(&receipt, &key, proof)
                            {
                                Ok(read) => read,
                                Err(error) if error.code == "authority-busy" => return Ok(None),
                                Err(error) => return Err(error),
                            };
                            read.limit(original_until)?;
                            let (_, fresh_plan) = crate::canonical_media::offer_plan(
                                read.target(),
                                &crate::canonical_native::base_profile()?,
                                &principal,
                            )
                            .map_err(|_| Error::forbidden())?;
                            if read.target().connection != expected_target
                                || fresh_plan.selected() != selected.as_slice()
                                || fresh_plan.rejected() != rejected.as_slice()
                            {
                                return Err(Error::forbidden());
                            }
                            let entries = match runtime
                                .media
                                .replacement_preparation_entries(read.source())
                            {
                                Ok(entries) => entries,
                                Err(error) if error.code == "authority-busy" => return Ok(None),
                                Err(error) => return Err(error),
                            };
                            if runtime.stop.is_cancelled()
                                || original_root_cancel.is_cancelled()
                                || original_physical_cancel.is_cancelled()
                                || !resource.resource_activation_current(&current)
                                || tokio::time::Instant::now() >= deadline
                            {
                                return Err(Error::forbidden());
                            }
                            // Only None proves no SQL/transaction/effect began. An
                            // error after SQL admission is NEVER put back in this loop.
                            runtime
                                .media
                                .store
                                .try_begin_native_replacement_start_with_read(
                                    receipt, &key, proof, read,
                                )
                                .map(|begun| {
                                    begun.map(|(attempt, read, origin)| {
                                        (attempt, read, origin, entries)
                                    })
                                })
                        })
                        .await;
                    match result {
                        Ok(Some((attempt, read, origin, entries))) => {
                            break (attempt, read, origin, entries, admission, roles);
                        }
                        Ok(None) => {
                            drop(roles);
                        }
                        Err(error) => {
                            drop(roles);
                            return Err(start_refused("start-begin", error));
                        }
                    }
                }
                // All callback captures have joined and dropped their aliases.
                // Release this exact fresh Writer BEFORE the original retry pause.
                drop(admission);
            }
            tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                _ = physical.lifetime.cancelled() => return Err(Error::forbidden()),
                _ = root_cancel.cancelled() => return Err(Error::forbidden()),
                _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+Duration::from_millis(20))) => {},
            }
        };
        let read = Arc::new(read);
        let id = saved.handle.intent().replacement.clone();
        let scope = StagedRtpScope {
            session: SessionId::from_string(intent.session_id.clone()),
            connection: id.clone(),
            owner: a.principal().ownership_key(),
            wire_conversation: Some(intent.conversation_id.clone()),
            wire_session: intent.session_id.clone(),
            wire_connection: intent.connection_id.clone(),
        };
        let data_observer = profile.supports_data().then(|| {
            Arc::new(crate::canonical_media::data_writes::PendingObserver::new(
                scope.clone(),
            ))
        });
        let policy = Arc::new(TargetPolicy {
            base: self.media.clone(),
            pending: saved.handle.prepared_binding(),
            principal: a.principal().clone(),
            profile,
            plan,
            root: Arc::downgrade(&saved),
            activation: OnceLock::new(),
            public_continuation: OnceLock::new(),
            origin: OnceLock::new(),
            preparation_operation: OnceLock::new(),
            receive_diagnostics: Default::default(),
            scene_snapshots: Default::default(),
            data_observer,
            readiness: Some(gate),
        });
        let invocation = Arc::new(StartInvocation {
            policy: policy.clone(),
            authority: super::media::StartAuthority::Reauthorized {
                resource: a.clone(),
                admission,
                peer: peer.clone(),
            },
            read,
            entries,
            deadline,
        });
        let result=async {
            if self.stop.is_cancelled() || root_cancel.is_cancelled() || !invocation.read.current()
                || a.invocation_cancellation().is_cancelled() || tokio::time::Instant::now() >= deadline {
                return Err(Error::forbidden());
            }
            self.media.register_replacement_target(&policy).map_err(|_|Error::forbidden())?;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="configure","Replacement resource start");
            let controller=native.configure_negotiated_media_with_held(&id,&a,ConversationId::from_string(intent.conversation_id.clone()),scope,self.media.clone(),policy.profile.clone(),policy.data_observer.clone().map(|observer| observer as Arc<dyn rvoip_uctp::v2::native_negotiation::NativeDataObserver>),8,invocation.as_ref()).map_err(|_|Error::forbidden())?;
            // Capture numeric metadata once, never a hot-path route lookup.
            policy.receive_diagnostics.install_io(native.media_counters(&id));
            let activation=native.native_media_activation(&id,&a).map_err(|_|Error::forbidden())?;
            policy.activation.set(Arc::new(super::media::Activation::Native(activation.clone()))).map_err(|_|Error::forbidden())?;
            // Reserve the exact dormant ports under a short planning read,
            // then discard every domain grant before polling adapter IO.
            let candidate = controller.stage_initial_selected_with_held(
                policy.plan.selected(), deadline, a.invocation_cancellation(), invocation.clone(),
            ).await.map_err(|_| Error::forbidden())?;
            drop(invocation);
            drop(roles);
            tracing::debug!(target:"connect_service::canonical_replacement",stage="quarantined-negotiate","Replacement resource start");
            let prepared = native.prepare_quarantined_native_offer(&id, a.clone(), candidate, Some(challenge))
                .await.map_err(|error| { tracing::debug!(target:"connect_service::canonical_replacement",?error,"Replacement resource preparation refused"); Error::forbidden() })?;
            let invocation = self.fresh_replacement_start_invocation(
                &start_origin, &policy, &peer, &physical, &allocated, deadline,
            ).await?;
            let outcome = prepared.activate_muted(invocation.clone()).await.map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",?error,"Replacement initial activation refused"); Error::forbidden()
            })?;
            let current = invocation.read.current();
            let confirmed = activation.confirmed_generation();
            let revision = controller.revision();
            let live = native.is_connection_live(&id);
            if !current || confirmed!=Some(policy.pending.generation()) || revision!=2 || !live {
                tracing::debug!(target:"connect_service::canonical_replacement", stage="initial-confirmation", current, ?confirmed, expected_generation=policy.pending.generation(), revision, live, "Replacement resource start refused");
                return Err(Error::forbidden());
            }
            let resource_streams=outcome.accepted.iter().map(|s|s.as_str().to_owned()).collect::<Vec<_>>();
            let observation=serde_json::json!({"candidate":id.as_str(),"generation":policy.pending.generation(),"stream_revision":controller.revision(),"streams":resource_streams,"resources_prepared":true,"prepared_muted":true,"media_ready":false});
            // Seal the original authenticated writer metadata while the signed
            // invocation exists. Keep it dormant through private preparation.
            let lifecycle=self.prepare_replacement_native_lifecycle(&id,&a,&physical).map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement", stage="lifecycle-capture", code=%error.code, "Replacement resource start refused");
                error
            })?;
            Ok((controller,observation,resource_streams,lifecycle))
        }.await;
        let (controller, observation, resource_streams, lifecycle) = match result {
            Ok(value) => value,
            Err(error) => {
                let uncertain = Arc::new(attempt.uncertain());
                *allocated
                    .failed_start
                    .lock()
                    .map_err(|_| Error::storage())? = Some(uncertain.clone());
                saved.handle.cancel();
                let store = self.media.store.clone();
                store_call(move || store.native_replacement_start_uncertain(&uncertain)).await?;
                return Err(error);
            }
        };
        tracing::debug!(target:"connect_service::canonical_replacement",stage="capture","Replacement resource start");
        let receipt = Arc::new(attempt.started(observation)?);
        let started = Arc::new(StartedReplacement {
            receipt,
            policy,
            controller,
            resource_streams,
            lifecycle: super::webrtc::ReplacementLifecycle::Native(lifecycle),
            intent,
            commit: Default::default(),
            phases: Default::default(),
            preparation: OnceLock::new(),
        });
        *allocated.started.lock().map_err(|_| Error::storage())? = Some(started.clone());
        let record = self.record_started_replacement(&started).await?;
        tracing::debug!(target:"connect_service::canonical_replacement",stage="origin","Replacement resource start");
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            store_call(move || store.native_replacement_media_origin(receipt))
        })
        .await?;
        started
            .policy
            .origin
            .set(origin)
            .map_err(|_| Error::forbidden())?;
        self.queue_replacement_media(saved, started, peer);
        tracing::debug!(target:"connect_service::canonical_replacement",stage="complete","Replacement resource start");
        Ok(Reply {
            record,
            answered: true,
        })
    }
    pub(super) async fn fresh_replacement_start_invocation(
        self: &Arc<Self>,
        origin: &Arc<connect_store::NativeReplacementStartOrigin>,
        policy: &Arc<TargetPolicy>,
        peer: &Arc<rvoip_core::peer_admission::PeerAdmission>,
        physical: &Arc<Physical>,
        allocated: &Arc<AllocatedReplacement>,
        deadline: tokio::time::Instant,
    ) -> Result<Arc<StartInvocation>> {
        let until = deadline.min(tokio::time::Instant::now() + Duration::from_secs(2));
        let wait = connect_store::AuthorityWait::new(
            until.saturating_duration_since(tokio::time::Instant::now()),
        );
        let _cancel_wait = wait.cancel_on_drop();
        let root = policy.root.upgrade().ok_or_else(Error::forbidden)?;
        let cancelled = root.handle.cancellation();
        let (proof, read, entries) = super::webrtc::admission::read(until, &wait, async {
            tokio::select! { _=self.stop.cancelled()=>{}, _=physical.lifetime.cancelled()=>{}, _=cancelled.cancelled()=>{} }
        }, || async {
            let store = self.media.store.clone(); let origin = origin.clone(); let wait = wait.clone();
            let (proof, read) = store_call(move || {
                // This exact Read covers only the dormant target's initial muted cut.
                let proof = store.resume_native_replacement_start_activation_authority(&origin, &wait)?;
                let read = store.native_replacement_start_read_under(&origin, &proof)?;
                Ok((Arc::new(proof), Arc::new(read)))
            }).await?;
            if allocated.physical.upgrade().is_none_or(|p| !Arc::ptr_eq(&p, physical)) ||
                physical.lifetime.is_cancelled() {
                return Err(Error::forbidden());
            }
            let entries = self.media.replacement_preparation_entries(read.source())?;
            let (_, plan) = crate::canonical_media::offer_plan(read.target(),
                &crate::canonical_native::base_profile()?, &policy.principal).map_err(|_| Error::forbidden())?;
            if plan.selected() != policy.plan.selected() || plan.rejected() != policy.plan.rejected() {
                return Err(Error::forbidden());
            }
            Ok((proof, read, entries))
        }).await?;
        Ok(Arc::new(StartInvocation {
            policy: policy.clone(),
            authority: super::media::StartAuthority::Resumed {
                proof,
                peer: peer.clone(),
                cancel: physical.lifetime.child_token(),
            },
            read,
            entries,
            deadline,
        }))
    }
    pub(super) async fn record_started_replacement_with_peer(
        &self,
        started: &StartedReplacement,
        a: Arc<AuthorizedEnvelope>,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            let a = a.clone();
            store_call(move || {
                store.record_native_replacement_started_with_peer(&receipt, proof(&a)?)
            })
        })
        .await
    }
    pub(super) async fn record_started_replacement(
        &self,
        started: &StartedReplacement,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            store_call(move || store.record_native_replacement_started(&receipt))
        })
        .await
    }
}
