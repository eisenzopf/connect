//! Owned local continuation and discard-only duplex readiness before movement.
use super::*;
use super::{
    continuation::{Invocation, Phase},
    start::StartedReplacement,
};
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
                tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,"Private replacement preparation stopped");
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
        let read = retry_metadata(&self.stop, || {
            let sources = self
                .media
                .scene_origins(&root.origin.request().session_id)
                .map_err(|_| Error::forbidden());
            let store = self.media.store.clone();
            let origin = started.policy.origin.get().cloned();
            async move {
                let sources = sources?;
                let origin = origin.ok_or_else(Error::forbidden)?;
                store_call(move || store.try_native_replacement_scene_read(&origin, &sources)).await.map_err(|error| {tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,detail=%error.message,"Replacement scene read deferred");error})
            }
        })
        .await?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(
                (read.media().source().expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
            );
        let source_phone = self
            .media
            .replacement_telephone_source(read.media().source())
            .map_err(|_| Error::forbidden())?;
        Ok(Invocation {
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
        let store = self.media.store.clone();
        let origin = started
            .policy
            .origin
            .get()
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let read = i.read.media().clone();
        store_call(move || store.begin_native_replacement_media_phase(origin, phase, &read)).await
    }
    async fn save_continuation(
        &self,
        started: &StartedReplacement,
        capture: NativeReplacementMediaCapture,
    ) -> Result<()> {
        let capture = Arc::new(capture);
        started
            .phases
            .lock()
            .map_err(|_| Error::storage())?
            .push(capture.clone());
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.record_native_replacement_media_phase(&capture))
        })
        .await?;
        Ok(())
    }
    async fn prepare_replacement_media(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        peer: &Arc<PeerAdmission>,
    ) -> Result<()> {
        {
            tracing::debug!(target:"connect_service::canonical_replacement",stage="attach-fence","Replacement continuation");
            let _roles = self.role_projection.lock().await;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="attach-read","Replacement continuation");
            let i = self.continuation_read(root, started, Phase::Attach).await?;
            tracing::debug!(target:"connect_service::canonical_replacement",stage="attach-begin","Replacement continuation");
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
            self.save_continuation(started, capture).await?;
            result.map_err(|e|{tracing::debug!(target:"connect_service::canonical_replacement",?e,"Replacement attach refused");Error::forbidden()})?;
        }
        let (context, source_phone) = {
            let _roles = self.role_projection.lock().await;
            let plan = self
                .core
                .plan_staged_device_replacement_muted_activation(&root.handle)
                .map_err(|_| Error::forbidden())?;
            let i = self
                .continuation_read(root, started, Phase::Activate(plan.clone()))
                .await?;
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
                            self.native
                                .media_controller(id)
                                .map_err(|_| Error::forbidden())?
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
            self.save_continuation(started, capture).await?;
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
                self.native
                    .observe_audio_audience_commit(id, context.commit())
                    .map_err(|_| Error::forbidden())?;
            }
        }
        started
            .policy
            .preparation_operation
            .set(context.operation_id().to_owned())
            .map_err(|_| Error::forbidden())?;
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
        let start = tokio::time::Instant::now();

        let cancellation = root.handle.cancellation();
        loop {
            tokio::select! {biased;_=self.stop.cancelled()=>return Ok(()),_=cancellation.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
            if !preparation.has_duplex_io() {
                let _ = started.controller.try_queue_local_silence(
                    &binding.negotiated.requested.logical_id,
                    LocalAudioPurpose::EndpointPreparation {
                        operation_id: context.operation_id().into(),
                    },
                    (start.elapsed().as_micros() * u128::from(rate) / 1_000_000) as u32,
                );
                continue;
            }
            let i = self
                .continuation_read(
                    root,
                    started,
                    Phase::Signal(DeviceReplacementEndpointEvent::MediaObserved),
                )
                .await?;
            let event = self
                .core
                .capture_device_replacement_event(
                    &root.handle,
                    DeviceReplacementSignalSource::NativePeer(peer.clone()),
                    DeviceReplacementEndpointEvent::MediaObserved,
                )
                .map_err(|_| Error::forbidden())?;
            let observed=event.observe_with_held(&i).map_err(|e|{tracing::debug!(target:"connect_service::canonical_replacement",?e,"Replacement actual media observation refused");Error::forbidden()})?;
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
            self.save_continuation(started, capture).await?;
            tracing::debug!(target:"connect_service::canonical_replacement",?observed,"Replacement actual private duplex readiness captured");
            return self.move_native_replacement(root, started).await;
        }
    }
}
