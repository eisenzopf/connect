//! Existing native-to-telephone path: cold prep, one begin, exact same-attempt final turns.
use super::*;
use crate::canonical_media::{
    final_post::{ObservationCustody, ObservedCut, PostCustody},
    FinalResourceBank, FinalResourceOwner, FinalTurnResources,
};
use rvoip_core::{
    ids::{ConversationId, SessionId},
    orchestrator::participant_control::{
        FinalControlGrant, ManagedAttemptBinding, ManagedCutAttempt, ManagedCutPublished,
        ParticipantReplacementLease, ParticipantReplacementScope, ReplacementPhase,
    },
    staged_streams::{
        device_replacement::{AudioDeviceReplacementHandle, AudioDeviceReplacementOutcome},
        MediaOwner, StreamControlError,
    },
};
pub(super) enum Outcome {
    Uncertain {
        attempt: Arc<connect_store::TelephoneReplacementCommitAttempt>,
        error: Error,
    },
    Published {
        attempt: Arc<connect_store::TelephoneReplacementCommitAttempt>,
        outcome: AudioDeviceReplacementOutcome,
        handle: AudioDeviceReplacementHandle,
        owner: MediaOwner,
        revisions: Result<connect_store::TelephoneReplacementAudienceCapture>,
        observed: ObservationCustody,
        cleanup: Result<()>,
    },
}
enum Turn {
    BeforeEntryBusy,
    Still(rvoip_core::orchestrator::participant_control::PreparedManagedCut),
    Published {
        outcome: AudioDeviceReplacementOutcome,
        handle: AudioDeviceReplacementHandle,
        owner: MediaOwner,
        revisions: Result<connect_store::TelephoneReplacementAudienceCapture>,
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
    pub(super) async fn commit_prepared_telephone_cut(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<media::Started>,
        until: tokio::time::Instant,
    ) -> Result<Outcome> {
        let cold::ColdCommit {
            plan,
            metadata,
            prepared: cut,
            route_resource,
            #[cfg(feature = "test-support")]
            control_probe,
        } = self
            .prepare_telephone_managed_cut(root, started, until)
            .await?;
        let context = started
            .policy
            .final_route_context()
            .map_err(|_| Error::forbidden())?;
        let cancel = root.handle.cancellation();
        let mut turn_resources: Option<FinalTurnResources> = None;
        let mut post = PostCustody::new(self, metadata.attempt_binding())?;
        #[cfg(feature = "test-support")]
        post.observe_for_test(control_probe.clone());
        let mut prepared = Some(cut);
        let mut after_unpublished = false;
        let mut attempt: Option<Arc<connect_store::TelephoneReplacementCommitAttempt>> = None;
        loop {
            let mut deferred = None;
            let step: Result<Turn> = async {
                if self.stop.is_cancelled()
                    || cancel.is_cancelled()
                    || tokio::time::Instant::now() >= until
                {
                    return Err(Error::forbidden());
                }
                if after_unpublished {
                    // Availability only, AFTER a genuine same-token Still return.
                    // No grant/role/Writer/bank exists here; full final fences
                    // still decide permission and commit after this hint.
                    let observed = self.observe_managed_retry_model(
                        root.handle.prepared_binding().appearance().session(),
                    );
                    #[cfg(feature = "test-support")]
                    if observed.as_ref().is_err_and(|e| e.code == "authority-busy") {
                        if let Some(probe) = &control_probe {
                            probe.note_model_retry_busy();
                        }
                    }
                    observed?;
                }
                let requirements = prepared
                    .as_ref()
                    .expect("same linear cut")
                    .try_final_resource_requirements()
                    .map_err(|e| {
                        if matches!(
                            e,
                            StreamControlError::MetadataBusy | StreamControlError::Backpressure
                        ) {
                            Error::new("authority-busy", "Current selected metadata busy")
                        } else {
                            Error::forbidden()
                        }
                    })?;
                let entries = self
                    .media
                    .scene_entries(
                        &root
                            .handle
                            .prepared_binding()
                            .appearance()
                            .session()
                            .as_str(),
                    )
                    .map_err(|_| Error::forbidden())?;
                let bank = FinalResourceBank::capture(requirements, |id| {
                    if id == &started.policy.pending.intent().replacement {
                        return Ok(FinalResourceOwner::Sip {
                            generation: started.policy.pending.generation(),
                        });
                    }
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
                        Error::new("authority-busy", "Exact selected resource busy")
                    } else {
                        Error::forbidden()
                    }
                })?;
                let sources = entries
                    .values()
                    .map(|entry| entry.origin.clone())
                    .collect::<Vec<_>>();
                turn_resources = Some(FinalTurnResources::new(bank, entries));
                let resources = turn_resources.as_ref().expect("outer exact turn owners");
                #[cfg(feature = "test-support")]
                resources.observe_for_test(&control_probe);
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
                #[cfg(feature = "test-support")]
                if let Some(probe) = &control_probe {
                    probe.note_final_writer_call();
                }
                let read = if let Some(original) = attempt.as_ref() {
                    let original = original.clone();
                    let store = self.media.store.clone();
                    store_call(move || {
                        store.try_telephone_replacement_commit_attempt_scene_write(&original, &wait)
                    })
                    .await?
                } else {
                    let store = self.media.store.clone();
                    let capture = started.capture.clone();
                    let facts = metadata.clone();
                    let read_wait = wait.clone();
                    let cutoff = facts.expires_at();
                    let read = store_call(move || {
                        let mut bound = cutoff;
                        store.telephone_replacement_scene_read_with_wait(
                            capture, &sources, &mut bound, &read_wait,
                        )
                    })
                    .await?;
                    let store = self.media.store.clone();
                    post.begin_issued();
                    let result = store_call(move || {
                        store.try_begin_prepared_telephone_replacement_commit_with_wait(
                            &facts, read, &wait,
                        )
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
                #[cfg(feature = "test-support")]
                if let Some(probe) = &control_probe {
                    probe.note_final_writer_returned();
                }
                let mut i = Invocation {
                    runtime: self.clone(),
                    root: root.clone(),
                    started: started.clone(),
                    read: Arc::new(read),
                    phase: Phase::Commit(plan.clone()),
                    deadline: until,
                    final_turn: true,
                    final_sip_route: None,
                };
                #[cfg(feature = "test-support")]
                if let Some(probe) = &control_probe {
                    probe.record_final_read(
                        crate::canonical_media::preparation_custody_fixture::FinalRead::Telephone(
                            Arc::downgrade(&i.read),
                        ),
                    );
                }
                let binding = attempt
                    .as_ref()
                    .expect("same started telephone attempt")
                    .attempt_binding();
                let conversation = ConversationId::from_string(
                    i.read.media().source().context().conversation_id.clone(),
                );
                let session =
                    SessionId::from_string(i.read.media().source().context().session_id.clone());
                let scope = rvoip_sip::adapter::SipManagedRouteScope::new(
                    binding,
                    started.policy.pending.intent().replacement.clone(),
                    started.policy.pending.generation(),
                    rvoip_sip::adapter::SipManagedRouteRole::ReplacementTarget,
                )
                .map_err(|_| Error::forbidden())?;
                let result = match context.try_with_managed_final_route(
                    &route_resource,
                    &scope,
                    |proof| -> Result<Turn> {
                        i.final_sip_route = Some(proof.view());
                        let current =
                            final_current::Current::capture(&i, binding, bank, Some(&proof))
                                .map_err(|error| {
                                    if matches!(
                                        error,
                                        StreamControlError::MetadataBusy
                                            | StreamControlError::Backpressure
                                    ) {
                                        Error::new(
                                            "authority-busy",
                                            "Current telephone owner metadata busy",
                                        )
                                    } else {
                                        Error::forbidden()
                                    }
                                })?;
                        let role_scope = ParticipantReplacementScope {
                            conversation: &conversation,
                            session: &session,
                            principal: &root.operator,
                            intent: root.handle.intent(),
                            phase: ReplacementPhase::Commit,
                            plan: Some(plan.media()),
                        };
                        let authority = i.current(&role_scope).map_err(|_| Error::forbidden())?;
                        let owner = i
                            .owner(&started.policy.pending.intent().replacement)
                            .map_err(|_| Error::forbidden())?;
                        match self.core.try_commit_prepared_managed_cut(
                            prepared.take().expect("one linear cut"),
                            FinalControlGrant::Replacement {
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
                            ManagedCutAttempt::StillUnpublished(same) => {
                                #[cfg(feature = "test-support")]
                                if let Some(probe) = &control_probe {
                                    probe.note_actual_unpublished();
                                }
                                Ok(Turn::Still(same.into_prepared()))
                            }
                            ManagedCutAttempt::Published(ManagedCutPublished::Replacement {
                                outcome,
                                handle,
                                post: obligation,
                            }) => {
                                post.install(obligation);
                                let observed = post.observe(ObservedCut::Replacement {
                                    outcome: outcome.clone(),
                                    handle: handle.clone(),
                                });
                                #[cfg(feature = "test-support")]
                                if let Some(probe) = &control_probe {
                                    probe.panic_after_publication();
                                }
                                let revisions = i.read.capture_commit_revisions(
                                    outcome
                                        .media
                                        .receiver_revisions
                                        .iter()
                                        .map(|(id, r)| (id.as_str().to_owned(), *r))
                                        .collect(),
                                );
                                Ok(Turn::Published {
                                    outcome,
                                    handle,
                                    owner,
                                    revisions,
                                    observed,
                                })
                            }
                            ManagedCutAttempt::Published(ManagedCutPublished::Handoff {
                                outcome,
                                handle,
                                post: obligation,
                            }) => {
                                post.install(obligation);
                                let _observed =
                                    post.observe(ObservedCut::Handoff { outcome, handle });
                                Ok(Turn::Terminal(Error::storage()))
                            }
                            ManagedCutAttempt::Refused {
                                post: obligation, ..
                            }
                            | ManagedCutAttempt::EffectPossible {
                                post: obligation, ..
                            } => {
                                post.install(obligation);
                                Ok(Turn::Terminal(Error::forbidden()))
                            }
                        }
                    },
                ) {
                    Ok(result) => result,
                    Err(failure) => {
                        let retry = failure.is_busy();
                        deferred = Some(failure);
                        if retry {
                            Ok(Turn::BeforeEntryBusy)
                        } else {
                            Ok(Turn::Terminal(Error::forbidden()))
                        }
                    }
                };
                drop(i);
                drop(roles);
                result
            }
            .await;
            if let Some(failure) = deferred.take() {
                failure.finish();
            }
            let begin_unknown = post.begin_is_unconfirmed();
            let terminal = begin_unknown
                || matches!(&step, Ok(Turn::Published { .. }) | Ok(Turn::Terminal(_)))
                || step.as_ref().is_err_and(|error| !temporary(error));
            if !terminal {
                drop(turn_resources.take());
            }
            match step {
                Ok(Turn::Still(same)) => {
                    prepared = Some(same);
                    after_unpublished = true;
                }
                Ok(Turn::BeforeEntryBusy) => {}
                Ok(Turn::Published {
                    outcome,
                    handle,
                    owner,
                    revisions,
                    observed,
                }) => {
                    let observer = post.release();
                    drop(turn_resources.take());
                    drop(route_resource);
                    let cleanup = observer.wait().await;
                    return Ok(Outcome::Published {
                        attempt: attempt.take().expect("actual published attempt"),
                        outcome,
                        handle,
                        owner,
                        revisions,
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
                    drop(route_resource);
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
                drop(route_resource);
                let _cleanup = observer.wait().await;
                let Some(attempt) = attempt.take() else {
                    return Err(error);
                };
                return Ok(Outcome::Uncertain { attempt, error });
            }
        }
    }
}
