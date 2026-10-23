//! Owned native handoff: actual opaque Core plan, one attempt and captured facts.
use crate::{
    authentication::StoreValidator,
    canonical_consultation::{descriptor, retry_metadata, store_call},
    canonical_media::handoff::HandoffInvocation,
    canonical_runtime::Runtime,
};
use connect_protocol::{
    transitions::{OutgoingDisposition, TransitionCommand},
    Error, Result,
};
use rvoip_core::{
    ids::{ConnectionId, SessionId},
    orchestrator::participant_control::ParticipantControlError,
    participant::ParticipantRole,
    staged_streams::StreamControlError,
    staged_streams::{
        audience::AudioAudienceHandle,
        handoff::{AudioHandoffHandle, AudioHandoffRequest, OldHandoffDisposition},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
pub(crate) struct AppliedHandoff {
    receipt: Arc<connect_store::NativeHandoffApplied>,
    handle: AudioHandoffHandle,
    session: String,
    audience_revision: u64,
    members: BTreeSet<ConnectionId>,
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
fn denied<T>(_: T) -> Error {
    Error::forbidden()
}
fn plan_error(error: ParticipantControlError) -> Error {
    match error {
        ParticipantControlError::Media(StreamControlError::NotReady) => Error::new(
            "not-ready",
            "Incoming native audio is not ready for handoff",
        ),
        ParticipantControlError::Unsupported
        | ParticipantControlError::Media(StreamControlError::Unsupported) => Error::new(
            "unsupported-media-profile",
            "Native handoff is unavailable for this media profile",
        ),
        other => denied(other),
    }
}
fn role(value: &ParticipantRole) -> String {
    match value {
        ParticipantRole::Custom(s) => s.clone(),
        ParticipantRole::Observer => "observer".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}
impl Runtime {
    pub(crate) fn queue_handoff(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) {
        let Ok(mut queued) = self.consultation_jobs.lock() else {
            return;
        };
        if queued.len() >= self.media.store.limits().connections || !queued.insert(record.clone()) {
            return;
        }
        let Ok(mut work) = self.work.lock() else {
            queued.remove(&record);
            return;
        };
        let runtime = self.clone();
        let job = Job {
            runtime: Arc::downgrade(self),
            record: record.clone(),
        };
        work.spawn(async move {
            let _job = job;
            let until=tokio::time::Instant::now()+Duration::from_secs(30);
            let outcome=loop {
                // A planning-only readiness miss owns no reader or role fence
                // while waiting. Recheck the original intent and independent
                // current physical grants before every bounded retry.
                let outcome=runtime.apply_handoff(credential.clone(),command.clone(),record.clone()).await;
                if outcome.as_ref().is_err_and(|e|e.code=="not-ready") && tokio::time::Instant::now()<until {
                    tokio::select! {biased; _=runtime.stop.cancelled()=>return, _=tokio::time::sleep(Duration::from_millis(100))=>{}}
                    continue;
                }
                break outcome;
            };
            if let Err(error) = outcome {
                if matches!(
                    error.code.as_str(),
                    "unsupported-media-profile"
                        | "private-context-required"
                        | "invalid-request"
                        | "forbidden"
                        | "conflict"
                ) {
                    let recorded = retry_metadata(&runtime.stop, || {
                        let store = runtime.media.store.clone();
                        let credential = credential.clone();
                        let command = command.clone();
                        let code = error.code.clone();
                        store_call(move || {
                            store.fail_native_handoff_preflight(&credential, command, &code)
                        })
                    })
                    .await;
                    if let Err(refusal) = recorded {
                        tracing::debug!(target:"connect_service::canonical_handoff",code=%refusal.code,"Native handoff preflight refusal recording deferred");
                    }
                }
                tracing::debug!(target:"connect_service::canonical_handoff",code=%error.code,"Native handoff did not complete");
            }
        });
    }
    async fn apply_handoff(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) -> Result<()> {
        let _roles = self.role_projection.lock().await;
        let saved = self
            .handoffs
            .lock()
            .map_err(|_| Error::storage())?
            .get(&record)
            .map(|a| a.receipt.clone());
        if let Some(receipt) = saved {
            return self.settle_handoff(receipt).await;
        }
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let key = credential.clone();
            let c = command.clone();
            store_call(move || store.native_handoff_origin(&key, c))
        })
        .await?;
        let context = self.media.store.native_handoff_context(&origin)?;
        let previous = if let Some(parent) = context.consultation_record_id.as_ref() {
            let audiences = self
                .consultation_audiences
                .lock()
                .map_err(|_| Error::storage())?;
            let a = audiences.get(parent).ok_or_else(Error::forbidden)?;
            if a.session != command.session_id || a.handle.audience().operation_id() != parent {
                return Err(Error::forbidden());
            }
            Some(a.handle.audience().clone())
        } else {
            None
        };
        let sid = SessionId::from_string(command.session_id.clone());
        let (read, plan, principal) = retry_metadata(&self.stop, || async {
            let sources = self
                .media
                .scene_origins(&command.session_id)
                .map_err(denied)?;
            let read = self
                .media
                .store
                .try_native_handoff_read(&origin, &sources)?;
            let actor = self.media.store.revalidate(&credential)?;
            let principal = StoreValidator::canonical(self.media.store.clone())
                .principal(actor)
                .map_err(denied)?;
            if principal.subject != read.caller().actor.identity_id {
                return Err(Error::forbidden());
            }
            let ctx = read.context();
            let source = sources
                .iter()
                .find(|s| Some(s.connection_id()) == ctx.source_connection_id.as_deref())
                .ok_or_else(Error::forbidden)?;
            let incoming = sources
                .iter()
                .find(|s| Some(s.connection_id()) == ctx.target_connection_id.as_deref())
                .ok_or_else(Error::forbidden)?;
            let remote = sources
                .iter()
                .find(|s| s.connection_id() == ctx.retained_connection_id)
                .ok_or_else(Error::forbidden)?;
            // Select only the actual committed audience containing this source.
            // Receiver pins include quarantined ports and are not membership.
            // Core rechecks the opaque handle's scene, group and incarnations;
            // this metadata never supplies authority or restores an old route.
            let previous: Option<AudioAudienceHandle> = if previous.is_some() {
                previous.clone()
            } else {
                self.handoffs
                    .lock()
                    .map_err(|_| Error::storage())?
                    .values()
                    .filter(|a| {
                        a.session == command.session_id
                            && a.members.contains(&ConnectionId::from_string(
                                source.runtime_connection_id().to_owned(),
                            ))
                    })
                    .max_by_key(|a| a.audience_revision)
                    .map(|a| a.handle.audience().clone())
            };
            let desired = read
                .media(source.connection_id(), None)
                .ok_or_else(Error::forbidden)?
                .target()
                .presence_role
                .clone();
            let request = AudioHandoffRequest {
                operation_id: record.clone(),
                expected_revision: self.core.audio_audience_revision(&sid).map_err(denied)?,
                old: ConnectionId::from_string(source.runtime_connection_id().to_owned()),
                retained: ConnectionId::from_string(remote.runtime_connection_id().to_owned()),
                admitted: ConnectionId::from_string(incoming.runtime_connection_id().to_owned()),
                old_disposition: if ctx.outgoing == OutgoingDisposition::Observe {
                    OldHandoffDisposition::Observer
                } else {
                    OldHandoffDisposition::Leave
                },
            };
            let revision = self.core.participant_revision(&sid).map_err(denied)?;
            let role = crate::canonical_runtime::participant_role(&desired);
            let independent = read
                .media(incoming.connection_id(), None)
                .ok_or_else(Error::forbidden)?
                .target()
                .handoff_preparation;
            let plan = if independent {
                let preparation = self
                    .media
                    .entry(&request.admitted)
                    .map_err(denied)?
                    .preparation()
                    .map_err(|_| {
                        Error::new("not-ready", "Incoming native preparation is not ready")
                    })?;
                self.core.plan_staged_prepared_audio_handoff(
                    &sid,
                    revision,
                    record.clone(),
                    role,
                    preparation.audience(),
                    previous.as_ref(),
                    request,
                )
            } else {
                let previous = previous.as_ref().ok_or_else(|| {
                    Error::new(
                        "unsupported-media-profile",
                        "Native handoff requires its own target preparation",
                    )
                })?;
                self.core.plan_staged_audio_handoff(
                    &sid,
                    revision,
                    record.clone(),
                    role,
                    previous,
                    request,
                )
            }
            .map_err(plan_error)?;
            Ok((read, plan, principal))
        })
        .await?;
        let mut intent = descriptor(plan.media().audience())?;
        let members = plan
            .media()
            .audience()
            .request()
            .private_connections
            .clone();
        let request = plan.media().request();
        intent["handoff"] = serde_json::json!({"outgoing":request.old.as_str(),"incoming":request.admitted.as_str(),"retained":request.retained.as_str(),"participant_revision":plan.expected_participant_revision(),"incoming_role":role(plan.incoming_role()),"disposition":if request.old_disposition==OldHandoffDisposition::Observer{"observe"}else{"leave"}});
        let store = self.media.store.clone();
        let Some(lease) = store_call(move || store.prepare_native_handoff(read, intent)).await?
        else {
            return Ok(());
        };
        let read = retry_metadata(&self.stop, || async {
            self.media.store.native_handoff_attempt_read(&lease)
        })
        .await?;
        let store = self.media.store.clone();
        let (attempt, read) = store_call(move || store.begin_native_handoff(lease, read)).await?;
        let remaining = (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let invocation = HandoffInvocation {
            policy: self.media.clone(),
            plan: plan.clone(),
            read: Arc::new(read),
            principal: principal.clone(),
            cancel: self.stop.child_token(),
        };
        let actual = self
            .core
            .commit_staged_audio_handoff_with_held(
                &sid,
                principal,
                plan,
                tokio::time::Instant::now() + Duration::from_secs(remaining),
                invocation.cancel.clone(),
                &invocation,
            )
            .await;
        let (outcome, handle) = match actual {
            Ok(v) => v,
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_handoff",?error,"Core handoff refused or became uncertain");
                drop(invocation);
                let store = self.media.store.clone();
                store_call(move || store.native_handoff_uncertain(attempt)).await?;
                return Err(Error::new(
                    "reconciliation-required",
                    "Native handoff attempt is uncertain",
                ));
            }
        };
        let p = outcome
            .media
            .participant_handoff
            .as_ref()
            .ok_or_else(Error::storage)?;
        let observed = serde_json::json!({"operation_id":outcome.media.operation_id,"audience_revision":outcome.media.audience_revision,"receiver_revisions":outcome.media.receiver_revisions.iter().map(|(id,r)|(id.as_str(),*r)).collect::<BTreeMap<_,_>>(),"restored":outcome.media.restored,"departed_connections":outcome.media.departed_connections.iter().map(|id|id.as_str()).collect::<Vec<_>>(),"participant_replacement":null,"participant_handoff":{"session":p.session.as_str(),"operation_id":p.operation_id,"correlation_id":p.correlation_id,"revision":p.revision,"outgoing":p.outgoing.as_str(),"outgoing_epoch":p.outgoing_epoch,"incoming":p.incoming.as_str(),"incoming_epoch":p.incoming_epoch,"outgoing_role":p.outgoing_role.as_ref().map(role),"incoming_role":role(&p.incoming_role),"retained":p.retained.as_str(),"admitted":p.admitted.as_str(),"retired":p.retired.iter().map(|id|id.as_str()).collect::<Vec<_>>()}});
        let receipt = Arc::new(attempt.applied(observed)?);
        let mut carry_failed = false;
        for id in handle.audience().commit().connections() {
            if !outcome.retiring.contains(id)
                && self
                    .native
                    .observe_audio_audience_commit(id, handle.audience().commit())
                    .is_err()
            {
                carry_failed = true;
            }
        }
        self.handoffs.lock().map_err(|_| Error::storage())?.insert(
            record,
            AppliedHandoff {
                receipt: receipt.clone(),
                handle,
                session: command.session_id.clone(),
                audience_revision: outcome.media.audience_revision,
                members,
            },
        );
        drop(invocation);
        self.settle_handoff(receipt).await?;
        if carry_failed {
            return Err(Error::new(
                "native-confirmation-lost",
                "Handoff applied but native confirmation could not follow",
            ));
        }
        Ok(())
    }
    async fn settle_handoff(
        &self,
        receipt: Arc<connect_store::NativeHandoffApplied>,
    ) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let r = receipt.clone();
            store_call(move || store.record_native_handoff_applied(&r))
        })
        .await?;
        let complete = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let r = receipt.clone();
            store_call(move || store.complete_native_handoff(&r))
        })
        .await?;
        if complete.is_none() {
            return Err(Error::new(
                "reconciliation-required",
                "Known handoff awaits current role projection",
            ));
        }
        let handle = self
            .handoffs
            .lock()
            .map_err(|_| Error::storage())?
            .get(receipt.record_id())
            .map(|a| a.handle.clone());
        if let Some(handle) = handle {
            let _cleanup = handle
                .finish_local_media_cleanup(tokio::time::Instant::now() + Duration::from_secs(3))
                .await;
        }
        Ok(())
    }
}
