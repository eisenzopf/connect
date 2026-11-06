//! Owned local continuation and discard-only duplex readiness before movement.
use super::*;
use super::{
    continuation::{Invocation, Phase},
    start::StartedReplacement,
};
mod audio_clock;
mod video;
use audio_clock::PreparationAudioClock;
use connect_store::{NativeReplacementMediaCapture, NativeReplacementMediaPhase as Step};
use rvoip_core::{
    orchestrator::participant_control::{
        DeviceReplacementEndpointEvent, DeviceReplacementSignalSource,
    },
    peer_admission::PeerAdmission,
    staged_streams::local_audio::LocalAudioPurpose,
};
impl Runtime {
    pub(super) fn queue_replacement_media(
        self: &Arc<Self>,
        root: Arc<PreparedReplacement>,
        started: Arc<StartedReplacement>,
        peer: Arc<PeerAdmission>,
    ) {
        let Ok(mut work) = self.work.lock() else {
            root.handle.cancel();
            return;
        };
        while work.try_join_next().is_some() {}
        let runtime = self.clone();
        work.spawn(async move {
            if let Err(error)=runtime.prepare_replacement_media(&root,&started,&peer).await {
                tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,code=%error.code,"Private replacement preparation stopped");
                root.handle.cancel();
            }
        });
    }
    pub(super) async fn continuation_read(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        phase: Phase,
    ) -> Result<Invocation> {
        retry_metadata(&self.stop, || {
            self.continuation_read_once(root, started, phase.clone())
        })
        .await
    }
    pub(super) async fn continuation_read_once(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        phase: Phase,
    ) -> Result<Invocation> {
        let entries = self
            .media
            .scene_entries(&root.origin.request().session_id)
            .map_err(|_| Error::forbidden())?;
        let sources = entries
            .values()
            .map(|entry| entry.origin.clone())
            .collect::<Vec<_>>();
        let store = self.media.store.clone();
        let origin = started
            .policy
            .origin
            .get()
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let read = store_call(move || store.try_native_replacement_scene_read(&origin, &sources)).await.map_err(|error| {tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,detail=%error.message,"Replacement scene read deferred");error})?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(
                (read.media().source().expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
            );
        let source_phone = self
            .media
            .replacement_telephone_source(read.media().source())
            .map_err(|_| Error::forbidden())?;
        Ok(Invocation {
            entries,
            source_phone,
            policy: started.policy.clone(),
            root: root.clone(),
            read: Arc::new(read),
            phase,
            deadline,
        })
    }
    async fn begin_continuation(
        &self,
        started: &StartedReplacement,
        i: &Invocation,
        phase: Step,
    ) -> Result<connect_store::NativeReplacementMediaAttempt> {
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %i.root.origin.request().operation_id, ?phase, "Replacement phase begin requested");
        let store = self.media.store.clone();
        let origin = started
            .policy
            .origin
            .get()
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let read = i.read.media().clone();
        let result =
            store_call(move || store.begin_native_replacement_media_phase(origin, phase, &read))
                .await;
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %i.root.origin.request().operation_id, ?phase,
            code = ?result.as_ref().err().map(|error| error.code.as_str()), "Replacement phase begin returned");
        result
    }
    async fn save_continuation(
        &self,
        root: &PreparedReplacement,
        started: &StartedReplacement,
        phase: Step,
        capture: NativeReplacementMediaCapture,
    ) -> Result<()> {
        let capture = Arc::new(capture);
        started
            .phases
            .lock()
            .map_err(|_| Error::storage())?
            .push(capture.clone());
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %root.origin.request().operation_id, ?phase, "Replacement phase recording requested");
        let result = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.record_native_replacement_media_phase(&capture))
        })
        .await;
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %root.origin.request().operation_id, ?phase,
            code = ?result.as_ref().err().map(|error| error.code.as_str()), "Replacement phase recording returned");
        result?;
        Ok(())
    }
    async fn prepare_replacement_media(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        peer: &Arc<PeerAdmission>,
    ) -> Result<()> {
        {
            tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="attach-fence","Replacement continuation");
            let (roles, i) = super::projection::read(
                &self.role_projection,
                &self.stop,
                root.deadline
                    .min(tokio::time::Instant::now() + Duration::from_secs(5)),
                || self.continuation_read_once(root, started, Phase::Attach),
            )
            .await?;
            tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="attach-begin","Replacement continuation");
            let attempt = self.begin_continuation(started, &i, Step::Attach).await?;
            let expected = attempt.expected()?;
            let result = self
                .core
                .attach_staged_device_replacement_with_held(&root.handle, Some(peer.clone()), &i)
                .await;
            drop(i);
            let capture = if result.is_ok() {
                attempt.applied(expected)?
            } else {
                attempt.uncertain()
            };
            super::projection::record(
                roles,
                self.save_continuation(root, started, Step::Attach, capture),
            )
            .await?;
            result.map_err(|e|{tracing::debug!(target:"connect_service::canonical_replacement",?e,"Replacement attach refused");Error::forbidden()})?;
        }
        let (context, source_phone) = {
            let (roles, i) = super::projection::read(
                &self.role_projection,
                &self.stop,
                root.deadline
                    .min(tokio::time::Instant::now() + Duration::from_secs(5)),
                || async {
                    let plan = self
                        .core
                        .plan_staged_device_replacement_muted_activation(&root.handle)
                        .map_err(|_| Error::forbidden())?;
                    self.continuation_read_once(root, started, Phase::Activate(plan))
                        .await
                },
            )
            .await?;
            let Phase::Activate(plan) = i.phase.clone() else {
                return Err(Error::forbidden());
            };
            let attempt = self.begin_continuation(started, &i, Step::Activate).await?;
            let expected = attempt.expected()?;
            let result = self
                .core
                .activate_staged_device_replacement_with_held(&root.handle, plan, &i)
                .await;
            let revisions = match &result {
                Ok(context) => {
                    let mut observed = std::collections::BTreeMap::new();
                    for id in context.commit().connections() {
                        let controller = if id.as_str()
                            == i.read
                                .media()
                                .source()
                                .context()
                                .source_runtime_connection_id
                            && i.source_phone.is_some()
                        {
                            i.source_phone
                                .as_ref()
                                .ok_or_else(Error::forbidden)?
                                .controller()
                                .map_err(|_| Error::forbidden())?
                        } else {
                            self.connection_media_controller(id)?
                        };
                        let (_, revision) = context
                            .commit()
                            .revisions_for(&controller)
                            .map_err(|_| Error::forbidden())?;
                        observed.insert(id.as_str().to_owned(), revision);
                    }
                    Some(i.read.capture_activation_revisions(observed)?)
                }
                Err(_) => None,
            };
            let source_phone = i.source_phone.clone();
            drop(i);
            let capture = if let Some(revisions) = revisions {
                attempt
                    .applied(expected)?
                    .with_audience_revisions(revisions)?
            } else {
                attempt.uncertain()
            };
            super::projection::record(
                roles,
                self.save_continuation(root, started, Step::Activate, capture),
            )
            .await?;
            let context=result.map_err(|e|{tracing::debug!(target:"connect_service::canonical_replacement",?e,"Replacement private activation refused");Error::forbidden()})?;
            (context, source_phone)
        };
        for id in context.commit().connections() {
            if let Some(phone) = source_phone
                .as_ref()
                .filter(|p| p.connection().is_ok_and(|connection| &connection == id))
            {
                phone
                    .observe_audience(context.commit())
                    .map_err(|_| Error::forbidden())?;
            } else {
                self.observe_native_audio_audience_commit(id, context.commit())
                    .map_err(|_| Error::forbidden())?;
            }
        }
        if let Some(gate) = &started.policy.readiness {
            if let Some(request) = super::movement::replacement_video(&root.plan)? {
                if !request.receivers.is_empty() {
                    let preparation = self.prepare_private_video(root, started, &request).await?;
                    gate.preroll
                        .set(Arc::new(preparation))
                        .map_err(|_| Error::forbidden())?;
                }
            }
        }
        started
            .policy
            .preparation_operation
            .set(context.operation_id().to_owned())
            .map_err(|_| Error::forbidden())?;
        started.lifecycle.start_private(&started.policy)?;
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %root.origin.request().operation_id, "Replacement decoder preparation requested");
        let preparation = retry_metadata(&self.stop, || async {
            self.core
                .prepare_staged_audio_endpoint(
                    &SessionId::from_string(root.origin.request().session_id.clone()),
                    root.handle.prepared_binding().binding().connection(),
                    context.operation_id(),
                )
                .map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",?error,"Private replacement decoder deferred");
                    Error::new("authority-busy","Private endpoint decoder is not yet available")
                })
        })
        .await?;
        tracing::debug!(target: "connect_service::replacement_phase_trace",
            operation = %root.origin.request().operation_id, "Replacement decoder preparation returned");
        let preparation = Arc::new(preparation);
        started
            .preparation
            .set(preparation.clone())
            .map_err(|_| Error::forbidden())?;
        let binding = started
            .controller
            .active_bindings()
            .into_iter()
            .find(|b| b.negotiated.requested.medium == rvoip_core::stream_lifecycle::Medium::Audio)
            .ok_or_else(Error::forbidden)?;
        let rvoip_core::stream_lifecycle::MediaFormat::Encoded { clock_rate_hz, .. } =
            &binding.negotiated.format
        else {
            return Err(Error::forbidden());
        };
        let rate = *clock_rate_hz;
        let mut audio_clock = PreparationAudioClock::new(rate, tokio::time::Instant::now())?;

        let cancellation = root.handle.cancellation();
        let mut waiting_reported = false;
        let mut duplex_reported = false;
        loop {
            tokio::select! { biased;
                _ = self.stop.cancelled() => return Ok(()),
                _ = cancellation.cancelled() => return Ok(()),
                _ = tokio::time::sleep_until(root.deadline) => {
                    return Err(Error::new("expired", "Original replacement preparation deadline elapsed"));
                },
                _ = tokio::time::sleep_until(audio_clock.next_due()) => {},
            }
            let Some(timestamp_rtp) = audio_clock.take_due(tokio::time::Instant::now()) else {
                continue;
            };
            // Core's first successful writer completion can precede the new
            // peer's decoded-media observation. Keep the same purpose-scoped
            // preparation audio available while that original report is pending;
            // otherwise the peer can wait for packets that we stopped sending.
            let waiting_for_decoder = started
                .policy
                .readiness
                .as_ref()
                .is_some_and(|gate| !gate.observed());
            if !preparation.has_duplex_io() || waiting_for_decoder {
                if !waiting_reported {
                    waiting_reported = true;
                    tracing::debug!(target: "connect_service::replacement_phase_trace",
                        operation = %root.origin.request().operation_id, "Replacement endpoint waiting for duplex activity");
                }
                let _ = started.controller.try_queue_local_silence(
                    &binding.negotiated.requested.logical_id,
                    LocalAudioPurpose::EndpointPreparation {
                        operation_id: context.operation_id().into(),
                    },
                    timestamp_rtp,
                );
                continue;
            }
            // The original endpoint remains public until its new peer has
            // actually decoded this challenge's audio and first video frame.
            // The signed report alone cannot bypass Core private route pins.
            if !duplex_reported {
                duplex_reported = true;
                tracing::debug!(target: "connect_service::replacement_phase_trace",
                    operation = %root.origin.request().operation_id, "Replacement endpoint duplex activity observed");
            }
            let i = self
                .continuation_read(
                    root,
                    started,
                    Phase::Signal(DeviceReplacementEndpointEvent::MediaObserved),
                )
                .await?;
            let observed = self
                .core
                .capture_device_replacement_event(
                    &root.handle,
                    DeviceReplacementSignalSource::NativePeer(peer.clone()),
                    DeviceReplacementEndpointEvent::MediaObserved,
                )
                .and_then(|event| event.observe_with_held(&i));
            let observed = match observed {
                Ok(observed) => observed,
                Err(error) if super::projection::is_core_metadata_busy(&error) => {
                    // Readiness uses try-locks while holding connection fences.
                    // Release this invocation before the next bounded poll and
                    // capture a fresh event with fresh current authority. The
                    // original root/deadline remain unchanged; no media phase
                    // or movement has been recorded by this observation.
                    drop(i);
                    tracing::debug!(target:"connect_service::canonical_replacement","Replacement readiness observation deferred by media contention");
                    continue;
                }
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_replacement",?error,"Replacement actual media observation refused");
                    return Err(Error::forbidden());
                }
            };
            if !matches!(
                observed,
                rvoip_core::participant_transitions::EndpointConfirmationObservation::NotRequired
            ) {
                drop(i);
                continue;
            }
            let attempt = self
                .begin_continuation(started, &i, Step::Readiness)
                .await?;
            let expected = attempt.expected()?;
            let capture = attempt.applied(expected)?;
            drop(i);
            self.save_continuation(root, started, Step::Readiness, capture)
                .await?;
            tracing::debug!(target:"connect_service::canonical_replacement",?observed,"Replacement actual private duplex readiness captured");
            return self.move_native_replacement(root, started).await;
        }
    }
}
