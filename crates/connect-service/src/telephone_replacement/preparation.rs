//! Private SIP decoder/output and exact captured adapter confirmation. No movement.
use super::continuation::{Invocation, Phase};
use super::*;
use crate::{
    canonical_consultation::{retry_metadata, store_call},
    canonical_replacement::PreparedReplacement,
};
use connect_store::{TelephoneReplacementPhase as Step, TelephoneReplacementPhaseCapture};
use rvoip_core::{ids::SessionId, staged_streams::local_audio::LocalAudioPurpose};
use std::time::Duration;
impl Runtime {
    pub(super) async fn telephone_scene(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<media::Started>,
        phase: Phase,
    ) -> Result<Invocation> {
        let admission =
            admission::Admission::new(root.deadline, root.handle.cancellation(), i64::MAX);
        let media = self.media.clone();
        let session = root
            .handle
            .prepared_binding()
            .appearance()
            .session()
            .as_str()
            .to_owned();
        let capture = started.capture.clone();
        let read = admission
            .read(self, move |store, until, wait| {
                let sources = media
                    .scene_origins(&session)
                    .map_err(|_| Error::forbidden())?;
                store.telephone_replacement_scene_read_with_wait(
                    capture.clone(),
                    &sources,
                    until,
                    wait,
                )
            })
            .await?;
        let expiry = tokio::time::Instant::now()
            + Duration::from_secs(
                (read.media().expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
            );
        Ok(Invocation {
            runtime: self.clone(),
            root: root.clone(),
            started: started.clone(),
            read: Arc::new(read),
            phase,
            deadline: root.deadline.min(expiry),
        })
    }
    // Reserve metadata under a short Writer, then obtain a fresh current IO
    // read for that same original attempt. Neither transport work nor a second
    // begin is performed while waiting for this post-reservation admission.
    async fn begin_telephone_phase(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<media::Started>,
        phase: Phase,
        step: Step,
    ) -> Result<(
        Arc<connect_store::TelephoneReplacementPhaseAttempt>,
        Invocation,
    )> {
        let admission =
            admission::Admission::new(root.deadline, root.handle.cancellation(), i64::MAX);
        let media = self.media.clone();
        let session = root
            .handle
            .prepared_binding()
            .appearance()
            .session()
            .as_str()
            .to_owned();
        let capture = started.capture.clone();
        let attempt = admission
            .run(self, move |store, until, wait| {
                let sources = media
                    .scene_origins(&session)
                    .map_err(|_| Error::forbidden())?;
                let read = match store.telephone_replacement_scene_read_with_wait(
                    capture.clone(),
                    &sources,
                    until,
                    wait,
                ) {
                    Ok(read) => read,
                    Err(error) if error.code == "authority-busy" => return Ok(None),
                    Err(error) => return Err(error),
                };
                store.try_begin_telephone_replacement_phase_with_wait(
                    read,
                    step.clone(),
                    *until,
                    wait,
                )
            })
            .await?;
        let original_attempt = attempt.clone();
        let admitted = admission
            .read(self, move |store, _, wait| {
                store.telephone_replacement_phase_attempt_read_with_wait(
                    original_attempt.clone(),
                    wait,
                )
            })
            .await
            .and_then(|read| {
                if self.stop.is_cancelled()
                    || root.handle.cancellation().is_cancelled()
                    || tokio::time::Instant::now() >= root.deadline
                    || !read.current()
                {
                    return Err(Error::forbidden());
                }
                let expiry = tokio::time::Instant::now()
                    + Duration::from_secs(
                        (read.media().expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
                    );
                Ok(Invocation {
                    runtime: self.clone(),
                    root: root.clone(),
                    started: started.clone(),
                    read: Arc::new(read),
                    phase,
                    deadline: root.deadline.min(expiry),
                })
            });
        match admitted {
            Ok(invocation) => Ok((attempt, invocation)),
            Err(error) => {
                root.handle.cancel();
                self.save_telephone_phase(started, attempt.uncertain())
                    .await?;
                Err(error)
            }
        }
    }
    async fn save_telephone_phase(
        &self,
        started: &media::Started,
        capture: TelephoneReplacementPhaseCapture,
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
            store_call(move || store.record_telephone_replacement_phase(&capture))
        })
        .await?;
        Ok(())
    }
    pub(super) async fn prepare_telephone_private(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        leg: &Arc<allocation::Allocated>,
        started: &Arc<media::Started>,
    ) -> Result<()> {
        {
            let _roles = self.role_projection.lock().await;
            let (attempt, i) = self
                .begin_telephone_phase(root, started, Phase::Attach, Step::Attach)
                .await?;
            let result = self
                .core
                .attach_staged_device_replacement_with_held(&root.handle, None, &i)
                .await;
            let capture = if result.is_ok() {
                attempt.applied(phase_observation(&i, "attach", false, false))?
            } else {
                attempt.uncertain()
            };
            drop(i);
            self.save_telephone_phase(started, capture).await?;
            result.map_err(|e| { tracing::debug!(target:"connect_service::telephone_replacement",?e,"Private SIP attach refused"); Error::forbidden() })?;
        }
        let context = {
            let _roles = self.role_projection.lock().await;
            let plan = self
                .core
                .plan_staged_device_replacement_muted_activation(&root.handle)
                .map_err(|_| Error::forbidden())?;
            let (attempt, i) = self
                .begin_telephone_phase(root, started, Phase::Activate(plan.clone()), Step::Activate)
                .await?;
            let result = self
                .core
                .activate_staged_device_replacement_with_held(&root.handle, plan, &i)
                .await;
            let capture = match &result {
                Ok(context) => {
                    let mut revisions = std::collections::BTreeMap::new();
                    for id in context.commit().connections() {
                        let controller = if id == &root.handle.intent().replacement {
                            started.controller.clone()
                        } else {
                            self.connection_native_adapter(id)?
                                .media_controller(id)
                                .map_err(|_| Error::forbidden())?
                        };
                        let (_, revision) = context
                            .commit()
                            .revisions_for(&controller)
                            .map_err(|_| Error::forbidden())?;
                        revisions.insert(id.as_str().to_owned(), revision);
                    }
                    attempt
                        .applied(phase_observation(&i, "activate", true, false))?
                        .with_audience(i.read.capture_activation_revisions(revisions)?)?
                }
                Err(_) => attempt.uncertain(),
            };
            drop(i);
            self.save_telephone_phase(started, capture).await?;
            result.map_err(|e| { tracing::debug!(target:"connect_service::telephone_replacement",?e,"Private SIP activation refused"); Error::forbidden() })?
        };
        for id in context.commit().connections() {
            if id != &root.handle.intent().replacement {
                self.observe_native_audio_audience_commit(id, context.commit())
                    .map_err(|_| Error::forbidden())?;
            }
        }
        started
            .policy
            .preparation_operation
            .set(context.operation_id().to_owned())
            .map_err(|_| Error::forbidden())?;
        let session = SessionId::from_string(
            root.handle
                .prepared_binding()
                .appearance()
                .session()
                .as_str(),
        );
        let preparation = retry_metadata(&root.handle.cancellation(), || async {
            self.core
                .prepare_staged_audio_endpoint(
                    &session,
                    &root.handle.intent().replacement,
                    context.operation_id(),
                )
                .map_err(|_| {
                    Error::new("authority-busy", "Private SIP decoder is not yet available")
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
            .next()
            .ok_or_else(Error::forbidden)?;
        let rate = match &binding.negotiated.format {
            rvoip_core::stream_lifecycle::MediaFormat::Encoded { clock_rate_hz, .. } => {
                *clock_rate_hz
            }
            _ => return Err(Error::forbidden()),
        };
        let start = tokio::time::Instant::now();
        let cancellation = root.handle.cancellation();
        let mut signals = leg.signals.lock().await;
        let mut confirmed = false; // Only the sealed adapter event below may set this.
        let mut prompt = tokio::time::interval(Duration::from_millis(50));
        prompt.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { biased;
                _=self.stop.cancelled()=>return Ok(()),
                _=cancellation.cancelled()=>return Ok(()),
                _=tokio::time::sleep_until(root.deadline)=>return Err(Error::new("expired","Telephone confirmation deadline elapsed")),
                _=prompt.tick()=>{
                    if !preparation.has_duplex_io() { if let Err(error)=started.controller.try_queue_local_silence(&binding.negotiated.requested.logical_id,LocalAudioPurpose::EndpointPreparation {operation_id:context.operation_id().into()},(start.elapsed().as_micros()*u128::from(rate)/1_000_000) as u32) {
                        tracing::trace!(target:"connect_service::telephone_replacement",?error,"Private SIP silence deferred");
                    } }
                    if self.capture_telephone_ready(root,started,&preparation,confirmed).await? { return self.move_telephone_replacement(root,leg,started).await; }
                },
                signal=signals.recv()=>{
                    let signal=signal.ok_or_else(Error::forbidden)?;
                    if signal.connection()!=&root.handle.intent().replacement { return Err(Error::forbidden()); }
                    let i=self.telephone_scene(root,started,Phase::Signal(signal.event().clone())).await?;
                    let observed=signal.into_event().observe_with_held(&i).map_err(|e| { tracing::debug!(target:"connect_service::telephone_replacement",?e,"Captured SIP signal refused"); Error::forbidden() })?;
                    tracing::debug!(target:"connect_service::telephone_replacement",?observed,"Captured SIP confirmation observed");
                    confirmed = matches!(observed, rvoip_core::participant_transitions::EndpointConfirmationObservation::Confirmed | rvoip_core::participant_transitions::EndpointConfirmationObservation::AlreadyConfirmed);
                    drop(i);
                    if self.capture_telephone_ready(root,started,&preparation,confirmed).await? { return self.move_telephone_replacement(root,leg,started).await; }
                },
            }
        }
    }
    async fn capture_telephone_ready(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<media::Started>,
        preparation: &rvoip_core::staged_streams::endpoint_preparation::AudioEndpointPreparation,
        confirmed: bool,
    ) -> Result<bool> {
        if !preparation.has_duplex_io() {
            return Ok(false);
        }
        // This phase captures actual private IO and attributed confirmation.
        // Planning the final audience move additionally requires established
        // ownership/retargeting, which this phase does not claim.
        if !confirmed {
            return Ok(false);
        }
        let (attempt, i) = self
            .begin_telephone_phase(root, started, Phase::Readiness, Step::Readiness)
            .await?;
        let ready = preparation.has_duplex_io() && confirmed;
        let capture = if ready {
            attempt.applied(phase_observation(&i, "readiness", true, true))?
        } else {
            attempt.uncertain()
        };
        drop(i);
        self.save_telephone_phase(started, capture).await?;
        if !ready {
            root.handle.cancel();
            return Err(Error::forbidden());
        }
        Ok(true)
    }
}
fn phase_observation(
    i: &Invocation,
    phase: &str,
    activated: bool,
    ready: bool,
) -> serde_json::Value {
    serde_json::json!({"candidate":i.root.handle.intent().replacement.as_str(),"generation":i.root.handle.prepared_binding().generation(),"phase":phase,"attached":true,"private":true,"activated_muted":activated,"media_ready":ready})
}
