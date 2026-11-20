//! One original Handoff lease/token; final grants never span cold preparation.
use super::*;
use crate::canonical_media::{
    final_post::{ObservationCustody, ObservedCut, PostCustody},
    FinalResourceBank, FinalResourceOwner, FinalTurnResources,
};
use rvoip_core::staged_streams::MediaActivation;
use rvoip_core::{
    ids::ConversationId,
    orchestrator::participant_control::{
        FinalControlGrant, HandoffFinalCurrent, ManagedAttemptBinding, ManagedAudioHandoffPlan,
        ManagedCutAttempt, ManagedCutPublished, ParticipantHandoffLease, ParticipantHandoffScope,
    },
    staged_streams::{
        handoff::{AudioHandoffHandle, AudioHandoffOutcome},
        MediaControlIntent, MediaOwner, MediaParticipantSource, MediaPurpose, MediaScope,
        MediaSource, StreamController,
    },
    AuthenticatedPrincipal,
};
struct Current<'a> {
    i: &'a HandoffInvocation,
    binding: (&'a str, &'a str, &'a str),
    bank: &'a FinalResourceBank,
    owners: BTreeMap<ConnectionId, MediaOwner>,
    principals: BTreeMap<ConnectionId, AuthenticatedPrincipal>,
}
impl Current<'_> {
    fn current(&self, attempt: ManagedAttemptBinding<'_>) -> bool {
        self.binding == (attempt.record, attempt.attempt_id, attempt.effect_id)
            && attempt.record == self.i.read.context().operation_record_id
            && self.i.read.current()
            && !self.i.cancel.is_cancelled()
    }
    fn intent(&self, intent: &MediaControlIntent<'_>) -> bool {
        matches!(intent,MediaControlIntent::Handoff {plan}if self.i.plan.media().audience().same_intent(plan))
    }
    fn owner_matches(&self, source: &MediaSource) -> bool {
        self.owners.get(&source.connection).is_some_and(|o| {
            o.member.conversation() == &source.conversation
                && o.presence.session() == &source.session
                && o.member.participant() == &source.participant
                && o.member.epoch() == source.membership_epoch
                && o.connection.connection() == &source.connection
                && o.connection.generation() == source.connection_generation
        })
    }
}
impl HandoffFinalCurrent for Current<'_> {
    fn is_current(
        &self,
        scope: &ParticipantHandoffScope<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt) && self.i.final_role_scope_current(scope).is_ok()
    }
    fn media_is_current(
        &self,
        controller: &Arc<StreamController>,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && matches!(scope.purpose, MediaPurpose::Control)
            && self.owners.get(scope.connection).is_some_and(|o| {
                scope.conversation == o.member.conversation()
                    && scope.session == o.presence.session()
            })
            && self.principals.get(scope.connection).is_some_and(|p| {
                rvoip_core::staged_streams::reliable_data::same_credential(scope.principal, p)
            })
            && self
                .bank
                .control_current(scope.connection, controller, None)
    }
    fn source_is_current(
        &self,
        controller: &Arc<StreamController>,
        source: &MediaSource,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && self.owner_matches(source)
            && self.bank.source_current(controller, source, None)
    }
    fn participant_source_is_current(
        &self,
        controller: &Arc<StreamController>,
        source: &MediaParticipantSource,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && self.owners.get(&source.connection).is_some_and(|o| {
                o.member.conversation() == &source.conversation
                    && o.presence.session() == &source.session
                    && o.member.participant() == &source.participant
                    && o.member.identity() == &source.identity
                    && o.member.epoch() == source.membership_epoch
                    && o.member
                        .revalidate_principal(&source.principal, chrono::Utc::now())
                        .is_ok()
            })
            && source.medium == rvoip_core::stream_lifecycle::Medium::Audio
            && self.bank.participant_current(controller, source, None)
    }
}
pub(super) enum Outcome {
    Uncertain {
        attempt: connect_store::NativeHandoffAttempt,
        error: Error,
    },
    Published {
        attempt: connect_store::NativeHandoffAttempt,
        outcome: AudioHandoffOutcome,
        handle: AudioHandoffHandle,
        observed: ObservationCustody,
        cleanup: Result<()>,
    },
}
enum Turn {
    BeforeEntryBusy,
    Still(rvoip_core::orchestrator::participant_control::PreparedManagedCut),
    Published {
        outcome: AudioHandoffOutcome,
        handle: AudioHandoffHandle,
        observed: ObservationCustody,
    },
    Terminal(Error),
}
fn temporary(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "authority-busy" | "storage-unavailable"
    )
}
impl Runtime {
    pub(super) async fn commit_prepared_handoff(
        self: &Arc<Self>,
        sid: &SessionId,
        principal: AuthenticatedPrincipal,
        plan: ManagedAudioHandoffPlan,
        lease: Arc<connect_store::NativeHandoffLease>,
        until: tokio::time::Instant,
    ) -> Result<Outcome> {
        let until_epoch = lease.expires_at().min(
            chrono::Utc::now().timestamp()
                + until
                    .saturating_duration_since(tokio::time::Instant::now())
                    .as_secs() as i64,
        );
        let cancel = self.stop.child_token();
        // Same original plan and lease survive every effect-free planning retry.
        let facts = loop {
            let result: Result<_> = async {
                let roles = self.role_projection.lock().await;
                let store = self.media.store.clone();
                let lease = lease.clone();
                let read = store_call(move || {
                    store.native_handoff_attempt_read_until(&lease, until_epoch)
                })
                .await?;
                let i = HandoffInvocation {
                    policy: self.media.clone(),
                    plan: plan.clone(),
                    read: Arc::new(read),
                    principal: principal.clone(),
                    cancel: cancel.clone(),
                };
                let facts = self
                    .core
                    .capture_staged_handoff_cut_facts(
                        sid,
                        principal.clone(),
                        &plan,
                        until,
                        cancel.clone(),
                        &i,
                    )
                    .map_err(|e| {
                        if matches!(
                            e,
                            ParticipantControlError::Media(
                                StreamControlError::MetadataBusy | StreamControlError::Backpressure
                            )
                        ) {
                            Error::new("authority-busy", "Handoff planning metadata busy")
                        } else {
                            plan_error(e)
                        }
                    });
                drop(i);
                drop(roles);
                facts
            }
            .await;
            match result {
                Ok(facts) => break facts,
                Err(error) if temporary(&error) => {
                    self.managed_cut_metadata_pause(until, &cancel).await?
                }
                Err(error) => return Err(error),
            }
        };
        let cut = self
            .core
            .prepare_staged_managed_cut(facts)
            .await
            .map_err(plan_error)?;
        let mut turn_resources: Option<FinalTurnResources> = None;
        let mut post = PostCustody::new(self, lease.attempt_binding())?;
        let mut prepared = Some(cut);
        let mut attempt: Option<connect_store::NativeHandoffAttempt> = None;
        loop {
            let step: Result<Turn> = async {
                if self.stop.is_cancelled()
                    || cancel.is_cancelled()
                    || tokio::time::Instant::now() >= until
                {
                    return Err(Error::forbidden());
                }
                let requirements = prepared
                    .as_ref()
                    .expect("same linear handoff cut")
                    .try_final_resource_requirements()
                    .map_err(|e| {
                        if matches!(
                            e,
                            StreamControlError::MetadataBusy | StreamControlError::Backpressure
                        ) {
                            Error::new("authority-busy", "Handoff resource metadata busy")
                        } else {
                            Error::forbidden()
                        }
                    })?;
                // Exact outer owners survive all callback and Writer aliases.
                let entries = self.media.scene_entries(sid.as_str()).map_err(denied)?;
                let bank = FinalResourceBank::capture(requirements, |id| {
                    let entry = entries.get(id).ok_or(StreamControlError::Authority)?;
                    Ok(FinalResourceOwner::Canonical {
                        activation: entry.activation()?.ok_or(StreamControlError::NotReady)?,
                        generation: entry.origin.generation(),
                    })
                })
                .map_err(|e| {
                    if matches!(
                        e,
                        StreamControlError::MetadataBusy | StreamControlError::Backpressure
                    ) {
                        Error::new("authority-busy", "Handoff exact resource witness busy")
                    } else {
                        Error::forbidden()
                    }
                })?;
                turn_resources = Some(FinalTurnResources::new(bank, entries));
                let resources = turn_resources.as_ref().expect("outer exact turn owners");
                let bank = &resources.bank;
                let entries = &resources.entries;
                let roles = tokio::select! {biased;
                    _=self.stop.cancelled()=>return Err(Error::forbidden()),
                    _=cancel.cancelled()=>return Err(Error::forbidden()),
                    _=tokio::time::sleep_until(until)=>return Err(Error::forbidden()),
                    roles=self.role_projection.lock()=>roles,
                };
                let wait = connect_store::AuthorityWait::new(
                    until.saturating_duration_since(tokio::time::Instant::now()),
                );
                let _cancel_wait = wait.cancel_on_drop();
                let read = if let Some(original) = attempt.as_ref() {
                    let admission = original.admission();
                    let store = self.media.store.clone();
                    store_call(move || {
                        store.try_native_handoff_started_scene_write(&admission, &wait)
                    })
                    .await?
                } else {
                    let store = self.media.store.clone();
                    let source_lease = lease.clone();
                    let read = store_call(move || {
                        store.native_handoff_attempt_read_until(&source_lease, until_epoch)
                    })
                    .await?;
                    let store = self.media.store.clone();
                    let lease = lease.clone();
                    post.begin_issued();
                    let result = store_call(move || {
                        store.try_begin_native_handoff_with_wait(&lease, read, until_epoch, &wait)
                    })
                    .await?;
                    post.begin_returned_definite();
                    let Some((original, read)) = result else {
                        drop(roles);
                        return Ok(Turn::BeforeEntryBusy);
                    };
                    attempt = Some(original);
                    read
                };
                let i = HandoffInvocation {
                    policy: self.media.clone(),
                    plan: plan.clone(),
                    read: Arc::new(read),
                    principal: principal.clone(),
                    cancel: cancel.clone(),
                };
                let binding = attempt
                    .as_ref()
                    .expect("same actual handoff attempt")
                    .attempt_binding();
                let mut owners = BTreeMap::new();
                let mut principals = BTreeMap::new();
                for id in bank.connections() {
                    let entry = entries.get(id).ok_or_else(Error::forbidden)?;
                    let read = i
                        .read
                        .media(entry.origin.connection_id(), None)
                        .ok_or_else(Error::forbidden)?;
                    let owner = entry
                        .owner(read.target(), &entry.principal, false)
                        .map_err(denied)?;
                    owners.insert(
                        id.clone(),
                        entry
                            .activation()
                            .map_err(denied)?
                            .ok_or_else(Error::forbidden)?
                            .current(owner)
                            .map_err(denied)?,
                    );
                    principals.insert(id.clone(), entry.principal.clone());
                }
                let current = Current {
                    i: &i,
                    binding,
                    bank,
                    owners,
                    principals,
                };
                let conversation =
                    ConversationId::from_string(i.read.context().conversation_id.clone());
                let scope = ParticipantHandoffScope {
                    conversation: &conversation,
                    session: sid,
                    principal: &principal,
                    plan: &plan,
                };
                let authority = i.current(&scope).map_err(plan_error)?;
                let result = match self.core.try_commit_prepared_managed_cut(
                    prepared.take().expect("one linear handoff cut"),
                    FinalControlGrant::Handoff {
                        authority: &authority,
                        attempt: ManagedAttemptBinding {
                            record: binding.0,
                            attempt_id: binding.1,
                            effect_id: binding.2,
                        },
                        held: &i,
                        current: &current,
                        cancellation: &cancel,
                        deadline: until,
                    },
                ) {
                    ManagedCutAttempt::StillUnpublished(same) => Turn::Still(same.into_prepared()),
                    ManagedCutAttempt::Published(ManagedCutPublished::Handoff {
                        outcome,
                        handle,
                        post: obligation,
                    }) => {
                        post.install(obligation);
                        let observed = post.observe(ObservedCut::Handoff {
                            outcome: outcome.clone(),
                            handle: handle.clone(),
                        });
                        Turn::Published {
                            outcome,
                            handle,
                            observed,
                        }
                    }
                    ManagedCutAttempt::Published(ManagedCutPublished::Replacement {
                        outcome,
                        handle,
                        post: obligation,
                    }) => {
                        post.install(obligation);
                        let _observed = post.observe(ObservedCut::Replacement { outcome, handle });
                        Turn::Terminal(Error::storage())
                    }
                    ManagedCutAttempt::Refused {
                        post: obligation, ..
                    }
                    | ManagedCutAttempt::EffectPossible {
                        post: obligation, ..
                    } => {
                        post.install(obligation);
                        Turn::Terminal(Error::forbidden())
                    }
                };
                drop(current);
                drop(authority);
                drop(i);
                drop(roles);
                Ok(result)
            }
            .await;
            let begin_unknown = post.begin_is_unconfirmed();
            let terminal = begin_unknown
                || matches!(&step, Ok(Turn::Published { .. }) | Ok(Turn::Terminal(_)))
                || step.as_ref().is_err_and(|error| !temporary(error));
            if !terminal {
                drop(turn_resources.take());
            }
            match step {
                Ok(Turn::Still(same)) => prepared = Some(same),
                Ok(Turn::BeforeEntryBusy) => {}
                Ok(Turn::Published {
                    outcome,
                    handle,
                    observed,
                }) => {
                    let observer = post.release();
                    drop(turn_resources.take());
                    let cleanup = observer.wait().await;
                    return Ok(Outcome::Published {
                        attempt: attempt.take().expect("actual published handoff"),
                        outcome,
                        handle,
                        observed,
                        cleanup,
                    });
                }
                result => {
                    let error = match result {
                        Ok(Turn::Terminal(error)) | Err(error) => error,
                        _ => unreachable!(),
                    };
                    let error = if begin_unknown {
                        Error::new("reconciliation-required","Original Store begin outcome is uncertain; same IDs retained without retry")
                    } else {
                        error
                    };
                    if !begin_unknown
                        && temporary(&error)
                        && prepared.is_some()
                        && self
                            .managed_cut_metadata_pause(until, &cancel)
                            .await
                            .is_ok()
                    {
                        continue;
                    }
                    let observer = post.release();
                    drop(turn_resources.take());
                    let _cleanup = observer.wait().await;
                    let Some(attempt) = attempt.take() else {
                        return Err(error);
                    };
                    return Ok(Outcome::Uncertain { attempt, error });
                }
            }
            if let Err(error) = self.managed_cut_metadata_pause(until, &cancel).await {
                let observer = post.release();
                drop(turn_resources.take());
                let _cleanup = observer.wait().await;
                let Some(attempt) = attempt.take() else {
                    return Err(error);
                };
                return Ok(Outcome::Uncertain { attempt, error });
            }
        }
    }
}
