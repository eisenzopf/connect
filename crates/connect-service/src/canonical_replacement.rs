//! Owned managed replacement phases; retained tickets contain no domain reader.
mod allocation;
pub(crate) mod continuation;
mod media;
pub(crate) use media::TargetPolicy;
mod movement;
mod preparation;
mod start;
use crate::{
    authentication::StoreValidator,
    canonical_consultation::{retry_metadata, store_call},
    canonical_media::replacement::ReplacementPreparationInvocation,
    canonical_runtime::Runtime,
};
use connect_protocol::{
    transitions::{TransitionCommand, TransitionIntent},
    Error, Result,
};
use connect_store::{
    NativeReplacementOrigin, NativeReplacementPrepared, ReplacementConfirmation,
    ReplacementPreparationRequest,
};
use rvoip_core::{
    adapter::ConnectionAdapter,
    ids::{ConnectionId, DeviceId, EndpointId, ParticipantId, SessionId},
    orchestrator::participant_control::DeviceReplacementPreparation,
    participant_transitions::{EndpointConfirmationPolicy, ParticipantReplacementMutation},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

pub(crate) struct PreparedReplacement {
    pub handle: Arc<DeviceReplacementPreparation>,
    pub(crate) operator: rvoip_core::AuthenticatedPrincipal,
    pub receipt: Arc<NativeReplacementPrepared>,
    origin: NativeReplacementOrigin,
    pub(crate) plan: serde_json::Value,
    recorded: AtomicBool,
    pub(crate) deadline: tokio::time::Instant,
    pub(crate) telephone:
        std::sync::Mutex<Option<Arc<crate::telephone_replacement::allocation::Allocated>>>,
    allocated: std::sync::Mutex<Option<Arc<allocation::AllocatedReplacement>>>,
}
impl Drop for PreparedReplacement {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}
struct Job {
    runtime: std::sync::Weak<Runtime>,
    record: String,
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.upgrade() {
            r.consultation_jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.record);
        }
    }
}
fn text(value: &serde_json::Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(Error::storage)
}
fn number(value: &serde_json::Value, key: &str) -> Result<u64> {
    value[key].as_u64().ok_or_else(Error::storage)
}
fn intent(plan: &serde_json::Value) -> Result<ParticipantReplacementMutation> {
    let expected = &plan["expected_preparation"];
    let confirmation: ReplacementConfirmation =
        serde_json::from_value(plan["confirmation"].clone()).map_err(|_| Error::storage())?;
    Ok(ParticipantReplacementMutation {
        operation_id: text(expected, "operation_id")?,
        correlation_id: text(expected, "operation_id")?,
        expected_revision: number(expected, "participant_revision")?,
        participant: ParticipantId::from_string(text(expected, "participant")?),
        source: ConnectionId::from_string(text(expected, "source")?),
        retained: ConnectionId::from_string(text(expected, "retained")?),
        replacement: ConnectionId::from_string(text(expected, "candidate")?),
        endpoint: EndpointId::from_string(text(expected, "endpoint")?),
        device: DeviceId::from_string(text(expected, "device")?),
        confirmation: match confirmation {
            ReplacementConfirmation::MediaOnly => EndpointConfirmationPolicy::MediaOnly,
            ReplacementConfirmation::AnsweredDtmf { digits } => {
                EndpointConfirmationPolicy::AnsweredDtmf { digits }
            }
        },
    })
}
pub(crate) fn observed(
    handle: &DeviceReplacementPreparation,
    plan: &serde_json::Value,
) -> Result<serde_json::Value> {
    let ticket = handle.prepared_binding();
    let actual = ticket.intent();
    if handle.is_committed()
        || handle.cancellation().is_cancelled()
        || actual != &intent(plan)?
        || ticket.appearance().role()
            != &crate::canonical_runtime::participant_role(&text(plan, "source_role")?)
    {
        return Err(Error::forbidden());
    }
    Ok(
        serde_json::json!({"operation_id":actual.operation_id,"session":ticket.appearance().session().as_str(),"participant":actual.participant.as_str(),"membership_epoch":ticket.appearance().membership_epoch(),"source":actual.source.as_str(),"retained":actual.retained.as_str(),"candidate":ticket.binding().connection().as_str(),"endpoint":ticket.binding().endpoint().as_str(),"device":ticket.binding().device().as_str(),"generation":ticket.generation(),"participant_revision":actual.expected_revision}),
    )
}
impl Runtime {
    pub(crate) fn queue_replacement(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) {
        let Ok(mut jobs) = self.consultation_jobs.lock() else {
            return;
        };
        if jobs.len() >= self.media.store.limits().connections || !jobs.insert(record.clone()) {
            return;
        }
        let Ok(mut work) = self.work.lock() else {
            jobs.remove(&record);
            return;
        };
        while work.try_join_next().is_some() {}
        let runtime = self.clone();
        let job = Job {
            runtime: Arc::downgrade(self),
            record: record.clone(),
        };
        work.spawn(async move {
            let _job=job;
            let result=runtime.prepare_replacement(credential.clone(),command.clone(),record.clone()).await;
            if let Err(error)=result {
                if matches!(error.code.as_str(),"unsupported-media-profile"|"invalid-request"|"forbidden"|"conflict") {
                    let _=retry_metadata(&runtime.stop,||{let store=runtime.media.store.clone();let key=credential.clone();let command=command.clone();let code=error.code.clone();store_call(move||store.fail_native_replacement_preflight(&key,command,&code))}).await;
                }
                tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,"Native replacement preparation deferred or refused");
            }
            let saved=runtime.replacements.lock().ok().and_then(|m|m.get(&record).cloned());
            if let Some(saved)=saved {runtime.watch_replacement(record,saved).await;}
        });
    }
    async fn prepare_replacement(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) -> Result<()> {
        let _roles = self.role_projection.lock().await;
        let saved = {
            self.replacements
                .lock()
                .map_err(|_| Error::storage())?
                .get(&record)
                .cloned()
        };
        if let Some(saved) = saved {
            return self.record_replacement(&saved).await;
        }
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let key = credential.clone();
            let c = command.clone();
            store_call(move || store.native_replacement_origin(&key, c))
        })
        .await?;
        let ctx = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let origin = origin.clone();
            store_call(move || store.native_replacement_context(&origin))
        })
        .await?;
        let telephone_route = match ctx.target_transport {
            connect_protocol::provisioning::EndpointTransport::Quic
            | connect_protocol::provisioning::EndpointTransport::Webtransport => None,
            connect_protocol::provisioning::EndpointTransport::Telephone => {
                let reference = ctx
                    .target_secret_ref
                    .as_deref()
                    .ok_or_else(Error::forbidden)?;
                Some(self.telephone_route(reference)?.1)
            }
            _ => {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "Replacement transport is not configured",
                ))
            }
        };
        let sid = SessionId::from_string(command.session_id.clone());
        let candidate = match &command.intent {
            TransitionIntent::Replace {
                candidate_connid, ..
            } => candidate_connid
                .clone()
                .unwrap_or_else(|| format!("replacement_{}", record)),
            _ => return Err(Error::forbidden()),
        };
        let native_request = ReplacementPreparationRequest {
            candidate_connection_id: candidate,
            participant_revision: self
                .core
                .participant_revision(&sid)
                .map_err(|_| Error::forbidden())?,
            timeout_seconds: 30,
            telephone_policy: None,
            confirmation: ReplacementConfirmation::MediaOnly,
        };
        let request = if let Some(route) = &telephone_route {
            route.policy.request(
                native_request.candidate_connection_id,
                native_request.participant_revision,
            )
        } else {
            native_request
        };
        let read = retry_metadata(&self.stop, || async { self.replacement_read(&origin) }).await?;
        let store = self.media.store.clone();
        let lease = store_call(move || store.prepare_native_replacement(read, request)).await?;
        let Some(lease) = lease else {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement preparation was already attempted",
            ));
        };
        let lease = Arc::new(lease);
        let plan = lease.plan()?;
        let resolved: Result<_> = async {
            let read = retry_metadata(&self.stop, || async {
                self.media
                    .store
                    .native_replacement_preparation_attempt_read(&lease)
            })
            .await?;
            let actor = self.media.store.revalidate(&credential)?;
            let principal = StoreValidator::canonical(self.media.store.clone())
                .principal(actor)
                .map_err(|_| Error::forbidden())?;
            if principal.subject != read.caller().actor.identity_id {
                return Err(Error::forbidden());
            }
            let telephone_target = if let Some(route) = &telephone_route {
                let old = self
                    .media
                    .replacement_source_owner(&read)
                    .map_err(|_| Error::forbidden())?;
                Some(Arc::new(crate::telephone_replacement::authority::target(
                    self,
                    &read,
                    route,
                    &text(&plan["telephone_route"], "destination")?,
                    &old.member,
                )?))
            } else {
                None
            };
            Ok((read, principal, telephone_target))
        }
        .await;
        let (read, principal, telephone_target) = match resolved {
            Ok(value) => value,
            Err(error) => {
                self.abort_unstarted_replacement(lease.clone(), &error.code)
                    .await;
                return Err(error);
            }
        };
        let expires = lease.expires_at();
        let store = self.media.store.clone();
        let begun = lease.clone();
        let result =
            store_call(move || store.begin_native_replacement_preparation(&begun, read)).await;
        let (attempt, read) = match result {
            Ok(value) => value,
            Err(error) => {
                self.abort_unstarted_replacement(lease, &error.code).await;
                return Err(error);
            }
        };
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs((expires - chrono::Utc::now().timestamp()).max(0) as u64);
        let invocation = ReplacementPreparationInvocation {
            policy: self.media.clone(),
            read: Arc::new(read),
            principal: principal.clone(),
            intent: intent(&plan)?,
            deadline,
            cancel: self.stop.child_token(),
            telephone: telephone_target,
        };
        // Begin is never retried. Core owns only a capacity/binding ticket here;
        // all allocation/target IO phases remain independently default-denied.
        let actual = self.core.prepare_staged_device_replacement_with_held(
            &sid,
            principal.clone(),
            invocation.intent.clone(),
            deadline,
            invocation.cancel.clone(),
            &invocation,
        );
        drop(invocation);
        let handle = match actual {
            Ok(handle) => handle,
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_replacement",?error,"Actual managed Core preparation refused");
                let store = self.media.store.clone();
                store_call(move || store.native_replacement_preparation_uncertain(attempt)).await?;
                return Err(Error::new(
                    "reconciliation-required",
                    "Managed Core preparation is uncertain",
                ));
            }
        };
        let observation = match observed(&handle, &plan) {
            Ok(v) => v,
            Err(error) => {
                handle.cancel();
                let store = self.media.store.clone();
                store_call(move || store.native_replacement_preparation_uncertain(attempt)).await?;
                return Err(error);
            }
        };
        let receipt = Arc::new(attempt.prepared(observation)?);
        let saved = Arc::new(PreparedReplacement {
            operator: principal,
            handle,
            receipt,
            origin,
            plan,
            recorded: AtomicBool::new(false),
            deadline,
            telephone: std::sync::Mutex::new(None),
            allocated: std::sync::Mutex::new(None),
        });
        self.replacements
            .lock()
            .map_err(|_| Error::storage())?
            .insert(record, saved.clone());
        self.record_replacement(&saved).await?;
        drop(_roles);
        if let Some(route) = telephone_route {
            let leg = self.allocate_telephone_replacement(&saved, route).await?;
            let runtime = self.clone();
            let pending = saved.clone();
            self.work.lock().map_err(|_| Error::storage())?.spawn(async move {
                if let Err(error) = runtime.submit_telephone_replacement(&pending, &leg).await {
                    tracing::debug!(target:"connect_service::telephone_replacement",code=%error.code,"Telephone replacement submission stopped");
                }
                // Answer/readiness/confirmation remain independently fenced;
                // submission itself can never commit a speaking route.
            });
        }
        Ok(())
    }
    async fn abort_unstarted_replacement(
        &self,
        lease: Arc<connect_store::NativeReplacementPreparationLease>,
        code: &str,
    ) {
        if !matches!(
            code,
            "forbidden" | "conflict" | "invalid-request" | "unsupported-media-profile"
        ) {
            return;
        }
        let _ = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let lease = lease.clone();
            let code = code.to_owned();
            store_call(move || store.abort_native_replacement_preparation(&lease, &code))
        })
        .await;
    }
    fn replacement_read(
        &self,
        origin: &NativeReplacementOrigin,
    ) -> Result<connect_store::NativeReplacementPreparationRead> {
        let request = origin.request();
        let source_id = request
            .connection_id
            .as_deref()
            .ok_or_else(Error::forbidden)?;
        let retained_id = request.intent.retained();
        let sources = self
            .media
            .scene_origins(&request.session_id)
            .map_err(|_| Error::forbidden())?;
        let remote = sources
            .iter()
            .find(|s| s.connection_id() == retained_id)
            .ok_or_else(|| Error::new("not-ready", "Retained replacement route is not live"))?;
        let source = if let Some(native) = sources.iter().find(|s| s.connection_id() == source_id) {
            connect_store::NativeReplacementSourceOrigin::Native(native.clone())
        } else {
            let retained = self
                .media
                .entry(&ConnectionId::from_string(
                    remote.runtime_connection_id().to_owned(),
                ))
                .map_err(|_| Error::forbidden())?;
            let phone = self
                .media
                .telephone_sources(&retained)?
                .into_iter()
                .filter_map(|p| p.established_origin())
                .find(|o| o.snapshot().connection_id == source_id)
                .ok_or_else(|| {
                    Error::new("not-ready", "Original replacement source is not live")
                })?;
            connect_store::NativeReplacementSourceOrigin::Telephone(phone)
        };
        self.media
            .store
            .try_native_replacement_preparation_read(origin, &source, remote)
    }
    async fn record_replacement(&self, saved: &PreparedReplacement) -> Result<()> {
        if saved.recorded.load(Ordering::Acquire) {
            return Ok(());
        }
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = saved.receipt.clone();
            store_call(move || store.record_native_replacement_prepared(&receipt))
        })
        .await?;
        saved.recorded.store(true, Ordering::Release);
        Ok(())
    }
    async fn watch_replacement(self: &Arc<Self>, record: String, saved: Arc<PreparedReplacement>) {
        loop {
            let captured = saved
                .allocated
                .lock()
                .ok()
                .and_then(|a| a.clone())
                .and_then(|a| a.started.lock().ok().and_then(|s| s.clone()))
                .and_then(|s| s.commit.lock().ok().and_then(|c| c.clone()));
            if let Some(captured) = captured {
                let _ = retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let captured = captured.clone();
                    store_call(move || match captured {
                        movement::Capture::Known(c) => {
                            store.record_native_replacement_committed(&c)
                        }
                        movement::Capture::Unknown(c) => {
                            store.native_replacement_commit_uncertain(&c)
                        }
                    })
                })
                .await;
            }
            let allocated = saved.allocated.lock().ok().and_then(|a| a.clone());
            let telephone = saved.telephone.lock().ok().and_then(|a| a.clone());
            let telephone_capture = telephone
                .as_ref()
                .and_then(|leg| leg.resources.lock().ok().and_then(|s| s.clone()))
                .and_then(|started| started.commit.lock().ok().and_then(|c| c.clone()));
            if let Some(captured) = telephone_capture {
                let _ = retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let captured = captured.clone();
                    store_call(move || match captured {
                        crate::telephone_replacement::movement::Capture::Known(c) => {
                            store.record_telephone_replacement_committed(&c)
                        }
                        crate::telephone_replacement::movement::Capture::Unknown(c) => {
                            store.record_telephone_replacement_commit_uncertain(&c)
                        }
                    })
                })
                .await;
            }
            let target_lost = if let Some(telephone) = &telephone {
                let absent = self
                    .telephone_route(&telephone.route.policy.route_reference)
                    .map_or(true, |(adapter, _)| {
                        !adapter.is_connection_live(&saved.handle.intent().replacement)
                    });
                let current = self
                    .media
                    .store
                    .telephone_replacement_submission_read(telephone.receipt.clone());
                if absent
                    || current.as_ref().is_err_and(|e| {
                        !matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                    })
                {
                    tracing::debug!(target:"connect_service::telephone_replacement",record=%record,absent,code=?current.as_ref().err().map(|e|e.code.as_str()),"Telephone preparation watcher lost target");
                }
                absent
                    || current.is_err_and(|e| {
                        !matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                    })
            } else if let Some(allocated) = allocated {
                let resources_lost = allocated
                    .started
                    .lock()
                    .ok()
                    .and_then(|s| s.clone())
                    .is_some_and(|s| {
                        s.controller.active_bindings().is_empty()
                            || s.policy
                                .activation
                                .get()
                                .and_then(|a| a.confirmed_generation())
                                != Some(saved.handle.prepared_binding().generation())
                    });
                let physical_lost = resources_lost
                    || allocated
                        .physical
                        .upgrade()
                        .is_none_or(|p| p.lifetime.is_cancelled())
                    || !self.native_connection_live(&saved.handle.intent().replacement);
                let store = self.media.store.clone();
                let receipt = allocated.receipt.clone();
                let current =
                    store_call(move || store.check_native_replacement_allocated_target(&receipt))
                        .await;
                physical_lost
                    || current.is_err_and(|e| {
                        !matches!(e.code.as_str(), "storage-unavailable" | "authority-busy")
                    })
            } else {
                false
            };
            let valid = if target_lost
                || saved.handle.cancellation().is_cancelled()
                || self.stop.is_cancelled()
            {
                false
            } else {
                match self.replacement_read(&saved.origin) {
                    Ok(read) => {
                        read.current()
                            && read.context().role
                                == saved.plan["source_role"].as_str().unwrap_or("")
                    }
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "authority-busy" | "storage-unavailable"
                        ) =>
                    {
                        true
                    }
                    Err(error) => {
                        tracing::debug!(target:"connect_service::canonical_replacement",record=%record,code=%error.code,"Replacement preparation watcher lost source");
                        false
                    }
                }
            };
            if !valid {
                let newly_cancelled = saved.handle.cancel();
                if let Some(telephone) = &telephone {
                    let _ = self.abandon_telephone_submission(telephone).await;
                }
                if newly_cancelled {
                    let _ = self.record_replacement(&saved).await;
                    let allocated = saved.allocated.lock().ok().and_then(|a| a.clone());
                    if let Some(allocated) = allocated {
                        let _ = self.record_allocated_replacement(&allocated).await;
                        let until = tokio::time::Instant::now() + Duration::from_secs(5);
                        while self.native_connection_live(&saved.handle.intent().replacement)
                            && !self.stop.is_cancelled()
                            && tokio::time::Instant::now() < until
                        {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        let started = allocated.started.lock().ok().and_then(|s| s.clone());
                        let uncertain = allocated.failed_start.lock().ok().and_then(|s| s.clone());
                        if let Some(started) = started {
                            let _ = self.record_started_replacement(&started).await;
                            let phases =
                                started.phases.lock().map(|p| p.clone()).unwrap_or_default();
                            for capture in phases {
                                let _ = retry_metadata(&self.stop, || {
                                    let store = self.media.store.clone();
                                    let capture = capture.clone();
                                    store_call(move || {
                                        store.record_native_replacement_media_phase(&capture)
                                    })
                                })
                                .await;
                            }
                            let _ = retry_metadata(&self.stop, || {
                                let store = self.media.store.clone();
                                let receipt = started.receipt.clone();
                                let absent = crate::canonical_native::Absent {
                                    natives: self.native_adapters(),
                                    id: saved.handle.intent().replacement.clone(),
                                    generation: saved.handle.prepared_binding().generation(),
                                };
                                store_call(move || {
                                    store.record_native_replacement_started_candidate_stopped(
                                        &receipt, &absent,
                                    )
                                })
                            })
                            .await;
                        } else if let Some(uncertain) = uncertain {
                            let _ = retry_metadata(&self.stop, || {
                                let store = self.media.store.clone();
                                let uncertain = uncertain.clone();
                                store_call(move || {
                                    store.native_replacement_start_uncertain(&uncertain)
                                })
                            })
                            .await;
                            let _ = retry_metadata(&self.stop, || {
                                let store = self.media.store.clone();
                                let uncertain = uncertain.clone();
                                let absent = crate::canonical_native::Absent {
                                    natives: self.native_adapters(),
                                    id: saved.handle.intent().replacement.clone(),
                                    generation: saved.handle.prepared_binding().generation(),
                                };
                                store_call(move || {
                                    store.record_native_replacement_uncertain_start_stopped(
                                        &uncertain, &absent,
                                    )
                                })
                            })
                            .await;
                        } else {
                            let _ = retry_metadata(&self.stop, || {
                                let store = self.media.store.clone();
                                let receipt = allocated.receipt.clone();
                                let absent = crate::canonical_native::Absent {
                                    natives: self.native_adapters(),
                                    id: saved.handle.intent().replacement.clone(),
                                    generation: saved.handle.prepared_binding().generation(),
                                };
                                store_call(move || {
                                    store.record_native_replacement_allocation_stopped(
                                        &receipt, &absent,
                                    )
                                })
                            })
                            .await;
                        }
                    } else if let Some(telephone) = &telephone {
                        if let Some(attempt) = telephone.submission.get() {
                            // Historical custody remains independent of current
                            // authority and cannot create another SIP submission.
                            let capture = attempt.uncertain();
                            let store = self.media.store.clone();
                            let _ = store_call(move || {
                                store.record_telephone_replacement_uncertain(&capture)
                            })
                            .await;
                        }
                    } else {
                        let _ = retry_metadata(&self.stop, || {
                            let store = self.media.store.clone();
                            let receipt = saved.receipt.clone();
                            store_call(move || {
                                store.record_native_replacement_preparation_stopped(&receipt)
                            })
                        })
                        .await;
                    }
                }
                self.replacements
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&record);
                return;
            }
            let _ = self.record_replacement(&saved).await;
            // No reader, role fence or Core control lease survives this scope.
            tokio::select! {biased;_=self.stop.cancelled()=>{},_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
    }
}
