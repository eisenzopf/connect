//! Same started attempt; only the opaque Core pre-effect return permits retry.
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
        StreamControlError,
    },
};

pub(super) enum Outcome {
    NeverInvoked {
        attempt: connect_store::NativeReplacementCommitAttempt,
        error: Error,
    },
    Uncertain {
        attempt: connect_store::NativeReplacementCommitAttempt,
        error: Error,
    },
    Published {
        attempt: connect_store::NativeReplacementCommitAttempt,
        outcome: AudioDeviceReplacementOutcome,
        handle: AudioDeviceReplacementHandle,
        revisions: Result<connect_store::NativeReplacementAudienceCapture>,
        adoption: Arc<crate::canonical_media::replacement::ReplacementAdoption>,
        observed: ObservationCustody,
        cleanup: Result<()>,
    },
}
enum Turn {
    JournalBegun,
    BeforeEntryBusy,
    Still(rvoip_core::orchestrator::participant_control::PreparedManagedCut),
    Published {
        outcome: AudioDeviceReplacementOutcome,
        handle: AudioDeviceReplacementHandle,
        revisions: Result<connect_store::NativeReplacementAudienceCapture>,
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
    pub(super) async fn commit_prepared_native_cut(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        until: tokio::time::Instant,
    ) -> Result<Outcome> {
        let cold = self
            .prepare_native_managed_cut(root, started, until)
            .await?;
        let cold::ColdCommit {
            plan,
            metadata,
            prepared: cut,
            adoption,
            source_phone,
            route_resource,
            #[cfg(feature = "test-support")]
            control_probe,
        } = cold;
        let cancel = root.handle.cancellation();
        // OUTER post owner predates every final frame. The original admitted
        // operation entry is reserved before durable begin and cannot disappear.
        let mut turn_resources: Option<FinalTurnResources> = None;
        let mut post = PostCustody::new(self, metadata.attempt_binding())?;
        #[cfg(feature = "test-support")]
        post.observe_for_test(control_probe.clone());
        let mut prepared = Some(cut);
        let mut after_unpublished = false;
        let mut attempt: Option<connect_store::NativeReplacementCommitAttempt> = None;
        let mut invoked = false;
        loop {
            let mut deferred = None;
            let mut cut_failure = None;
            // Per-turn controller/resource owners are captured BEFORE any
            // purpose Writer. They drop after its lexical frame, before pause.
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
                            Error::new("authority-busy", "Current selected resource metadata busy")
                        } else {
                            Error::forbidden()
                        }
                    })?;
                let entries = self
                    .media
                    .scene_entries(&root.origin.request().session_id)
                    .map_err(|_| Error::forbidden())?;
                let bank = FinalResourceBank::capture(requirements, |id| {
                    if id == started.policy.pending.binding().connection() {
                        return Ok(FinalResourceOwner::Canonical {
                            activation: started
                                .policy
                                .activation
                                .get()
                                .cloned()
                                .ok_or(StreamControlError::NotReady)?,
                            generation: started.policy.pending.generation(),
                        });
                    }
                    if let Some(phone) = source_phone
                        .as_ref()
                        .filter(|phone| id == &phone.pending.intent().replacement)
                    {
                        return Ok(FinalResourceOwner::Sip {
                            generation: phone.pending.generation(),
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
                        Error::new("authority-busy", "Exact resource witness busy")
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
                #[cfg(feature = "test-support")]
                if let Some(probe) = &control_probe {
                    probe.note_final_writer_call();
                }
                let store = self.media.store.clone();
                let wait = connect_store::AuthorityWait::new(
                    until.saturating_duration_since(tokio::time::Instant::now()),
                );
                let _cancel_wait = wait.cancel_on_drop();
                let read = if let Some(original) = attempt.as_ref() {
                    let admission = original.admission();
                    let read = store_call(move || {
                        store.try_native_replacement_commit_attempt_scene_write(
                            &admission, &sources, &wait,
                        )
                    })
                    .await?;
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = &control_probe {
                        probe.note_final_writer_returned();
                    }
                    read
                } else {
                    let facts = metadata.clone();
                    let dispatch = post.begin_dispatch();
                    #[cfg(feature = "test-support")]
                    let store_probe = control_probe.clone();
                    // Reserve primary SQL before taking any Writer. Ordinary
                    // native moves write the journal Record while the current
                    // media owners keep their original packet snapshots.
                    let result = store_call(move || {
                        let begin = || {
                            store.try_admit_and_begin_prepared_native_replacement_commit(
                                &facts,
                                &sources,
                                &wait,
                                || dispatch.issue(),
                            )
                        };
                        #[cfg(feature = "test-support")]
                        let begun = if let Some(observation) = store_probe
                            .as_ref()
                            .and_then(|probe| probe.native_begin_observation())
                        {
                            store.with_native_prepared_admission_observation_for_test(
                                observation,
                                &wait,
                                begin,
                            )?
                        } else {
                            begin()?
                        };
                        #[cfg(not(feature = "test-support"))]
                        let begun = begin()?;
                        Ok(begun)
                    })
                    .await?;
                    post.begin_returned_definite();
                    let Some((read, original)) = result else {
                        drop(roles);
                        return Ok(Turn::BeforeEntryBusy);
                    };
                    attempt = Some(original);
                    let Some(read) = read else {
                        drop(roles);
                        return Ok(Turn::JournalBegun);
                    };
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = &control_probe {
                        probe.note_final_writer_returned();
                    }
                    read
                };
                let mut i = Invocation {
                    entries: entries.clone(),
                    policy: started.policy.clone(),
                    source_phone: source_phone.clone(),
                    read: Arc::new(read),
                    root: root.clone(),
                    phase: Phase::Commit(plan.clone()),
                    deadline: until,
                    final_turn: true,
                    managed_capture: false,
                    final_sip_route: None,
                };
                // Exact original fault boundary: SAME durable Started, no
                // Core invocation. Try-only test registry; no retry permission
                // or different denial is manufactured from its result.
                #[cfg(feature = "test-support")]
                if !invoked {
                    self.media
                        .check_replacement_adoption_fault_for_test(root.origin.record_id())
                        .map_err(|error| {
                            if matches!(error, StreamControlError::MetadataBusy) {
                                Error::new(
                                    "authority-busy",
                                    "Original pre-Core fault metadata busy",
                                )
                            } else {
                                Error::forbidden()
                            }
                        })?;
                }
                #[cfg(feature = "test-support")]
                if let Some(probe) = &control_probe {
                    probe.record_final_read(
                        crate::canonical_media::preparation_custody_fixture::FinalRead::Native(
                            Arc::downgrade(&i.read),
                        ),
                    );
                }
                let original = attempt.as_ref().expect("same actual started attempt");
                let binding = original.attempt_binding();
                let conversation =
                    ConversationId::from_string(root.origin.request().conversation_id.clone());
                let session = SessionId::from_string(root.origin.request().session_id.clone());
                let mut enter = |i: &Invocation,
                                 sip: Option<&rvoip_sip::adapter::SipCurrentMediaRoute<'_>>|
                 -> Result<Turn> {
                    let scope = ParticipantReplacementScope {
                        conversation: &conversation,
                        session: &session,
                        principal: &root.operator,
                        intent: root.handle.intent(),
                        phase: ReplacementPhase::Commit,
                        plan: Some(plan.media()),
                    };
                    let current = final_current::Current::capture(i, binding, bank, sip).map_err(
                        |error| {
                            if matches!(
                                error,
                                StreamControlError::MetadataBusy | StreamControlError::Backpressure
                            ) {
                                Error::new("authority-busy", "Current final owner metadata busy")
                            } else {
                                Error::forbidden()
                            }
                        },
                    )?;
                    let authority = i
                        .current(&scope)
                        .map_err(super::super::projection::core_plan_error)?;
                    if !invoked {
                        started.lifecycle.begin_movement();
                    }
                    invoked = true;
                    // Set invocation truth BEFORE Core entry. No public error
                    // may later reconstruct a NeverInvoked disposition.
                    match self.core.try_commit_prepared_managed_cut(
                        prepared.take().expect("one linear cut"),
                        FinalControlGrant::Replacement {
                            authority: &authority,
                            attempt: ManagedAttemptBinding {
                                record: binding.0,
                                attempt_id: binding.1,
                                effect_id: binding.2,
                            },
                            held: i,
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
                            let _observed = post.observe(ObservedCut::Handoff { outcome, handle });
                            Ok(Turn::Terminal(Error::storage()))
                        }
                        ManagedCutAttempt::Refused {
                            error,
                            post: obligation,
                        } => {
                            post.install(obligation);
                            cut_failure = Some(("core-cut-refused", error));
                            Ok(Turn::Terminal(Error::forbidden()))
                        }
                        ManagedCutAttempt::EffectPossible {
                            error,
                            post: obligation,
                        } => {
                            post.install(obligation);
                            cut_failure = Some(("core-cut-effect-possible", error));
                            Ok(Turn::Terminal(Error::forbidden()))
                        }
                    }
                };
                let result = if let Some(phone) = source_phone.as_ref() {
                    let context = phone
                        .final_route_context()
                        .map_err(|_| Error::forbidden())?;
                    let scope = rvoip_sip::adapter::SipManagedRouteScope::new(
                        binding,
                        phone.pending.intent().replacement.clone(),
                        phone.pending.generation(),
                        rvoip_sip::adapter::SipManagedRouteRole::ReplacementSource,
                    )
                    .map_err(|_| Error::forbidden())?;
                    match context.try_with_managed_final_route(
                        route_resource.as_ref().ok_or_else(Error::forbidden)?,
                        &scope,
                        |proof| {
                            i.final_sip_route = Some(proof.view());
                            enter(&i, Some(&proof))
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
                    }
                } else {
                    enter(&i, None)
                };
                // Drops ALL returned callback leases through sync Core return,
                // then actual Invocation/Writer/role before bank/controller Arcs.
                drop(i);
                drop(roles);
                result
            }
            .await;
            if let Some(failure) = deferred.take() {
                failure.finish();
            }
            if let Some((stage, error)) = cut_failure {
                tracing::debug!(target: "connect_service::canonical_replacement",
                    operation_record = root.origin.record_id(), stage, ?error,
                    "Original managed replacement cut failed");
            }
            let begin_unknown = post.begin_is_unconfirmed();
            let terminal = begin_unknown
                || matches!(&step, Ok(Turn::Published { .. }) | Ok(Turn::Terminal(_)))
                || step.as_ref().is_err_and(|error| !temporary(error));
            if !terminal {
                drop(turn_resources.take());
            }
            match step {
                Ok(Turn::JournalBegun) => continue,
                Ok(Turn::Still(same)) => {
                    prepared = Some(same);
                    after_unpublished = true;
                }
                Ok(Turn::BeforeEntryBusy) => {}
                Ok(Turn::Published {
                    outcome,
                    handle,
                    revisions,
                    observed,
                }) => {
                    let observer = post.release();
                    drop(turn_resources.take());
                    drop(route_resource);
                    drop(source_phone);
                    // Publication already happened. All original transient
                    // resource owners are gone. Start metadata activation on
                    // the SAME turn alongside, not after, the existing cleanup
                    // observation; never await it before the actual Core cut.
                    let ((), cleanup) = tokio::join! {biased;
                        self.activate_dormant_replacement_routes(&adoption,until),
                        observer.wait(),
                    };
                    return Ok(Outcome::Published {
                        attempt: attempt.take().expect("published actual attempt"),
                        outcome,
                        handle,
                        revisions,
                        adoption,
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
                    if !begin_unknown && temporary(&error) && prepared.is_some() {
                        if self
                            .managed_cut_metadata_pause(until, &cancel)
                            .await
                            .is_ok()
                        {
                            continue;
                        }
                    }
                    let observer = post.release();
                    drop(turn_resources.take());
                    drop(route_resource);
                    drop(source_phone);
                    let _cleanup = observer.wait().await;
                    let Some(attempt) = attempt.take() else {
                        return Err(error);
                    };
                    return Ok(if invoked {
                        Outcome::Uncertain { attempt, error }
                    } else {
                        Outcome::NeverInvoked { attempt, error }
                    });
                }
            }
            if let Err(error) = self.managed_cut_metadata_pause(until, &cancel).await {
                let observer = post.release();
                drop(turn_resources.take());
                drop(route_resource);
                drop(source_phone);
                let _cleanup = observer.wait().await;
                let Some(attempt) = attempt.take() else {
                    return Err(error);
                };
                return Ok(if invoked {
                    Outcome::Uncertain { attempt, error }
                } else {
                    Outcome::NeverInvoked { attempt, error }
                });
            }
        }
    }
}
