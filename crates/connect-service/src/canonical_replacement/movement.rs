//! One actual managed switch followed only by captured metadata projection.
use super::*;
use super::{continuation::Phase, start::StartedReplacement};
use connect_store::{
    NativeReplacementCommitNotSubmitted, NativeReplacementCommitUncertain,
    NativeReplacementCommitted,
};
use rvoip_core::{adapter::EndReason, participant_transitions::EndpointConfirmationPolicy};
#[derive(Clone)]
pub(super) enum Capture {
    Known(Arc<NativeReplacementCommitted>),
    Unknown(Arc<NativeReplacementCommitUncertain>),
    NotSubmitted(Arc<NativeReplacementCommitNotSubmitted>),
}
impl Runtime {
    pub(super) async fn move_native_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
    ) -> Result<()> {
        let (roles, i) = super::projection::read(&self.role_projection, &self.stop,
            root.deadline.min(tokio::time::Instant::now() + Duration::from_secs(5)), || async {
                let plan = self.core.plan_staged_device_replacement(&root.handle, replacement_video(&root.plan)?).map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="plan",?error,"Replacement pre-commit refused");
                    super::projection::core_plan_error(error)
                })?;
                self.continuation_read_once(root, started, Phase::Commit(plan)).await
            }).await.map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="scene-read",code=%error.code,"Replacement pre-commit refused");
                error
            })?;
        let Phase::Commit(plan) = i.phase.clone() else {
            return Err(Error::forbidden());
        };
        let owner = i
            .owner(started.policy.pending.binding().connection())
            .map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="current-owner",?error,"Replacement pre-commit refused");
                Error::forbidden()
            })?;
        let attempt = {
            let store = self.media.store.clone();
            let origin = started
                .policy
                .origin
                .get()
                .cloned()
                .ok_or_else(Error::forbidden)?;
            let read = i.read.clone();
            store_call(move || store.begin_native_replacement_commit(origin, &read)).await.map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",operation=%root.origin.request().operation_id,stage="commit-reservation",code=%error.code,"Replacement pre-commit refused");
                error
            })?
        };
        let adoption = match self.media.prepare_replacement_adoption(
            &started.policy,
            attempt.activation().clone(),
            owner.connection.clone(),
        ) {
            Ok(adoption) => adoption,
            Err(_) => {
                drop(i);
                let capture = Arc::new(attempt.not_submitted());
                *started.commit.lock().map_err(|_| Error::storage())? =
                    Some(Capture::NotSubmitted(capture.clone()));
                // Release the role/read interval before acquiring any writer.
                // The cleanup loop retains the definite outcome even if its
                // first metadata write is transiently unavailable.
                drop(roles);
                self.stop_unsubmitted_replacement(root, started, capture)
                    .await;
                return Err(Error::forbidden());
            }
        };
        let commit_began = std::time::Instant::now();
        let result = self
            .core
            .commit_staged_device_replacement_with_held(plan, &i)
            .await;
        let (outcome, handle) = match result {
            Ok(value) => value,
            Err(error) => {
                drop(i);
                let capture = Arc::new(attempt.uncertain());
                *started.commit.lock().map_err(|_| Error::storage())? =
                    Some(Capture::Unknown(capture.clone()));
                super::projection::record(
                    roles,
                    retry_metadata(&self.stop, || {
                        let store = self.media.store.clone();
                        let capture = capture.clone();
                        store_call(move || store.native_replacement_commit_uncertain(&capture))
                    }),
                )
                .await?;
                tracing::debug!(target:"connect_service::canonical_replacement",?error,"Replacement movement outcome uncertain");
                return Err(Error::forbidden());
            }
        };
        tracing::debug!(target:"connect_service::canonical_replacement",
            connection=%outcome.admitted, stage="physical-complete",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        let observed = outcome
            .media
            .receiver_revisions
            .iter()
            .map(|(id, revision)| (id.as_str().to_owned(), *revision))
            .collect();
        let revisions = i.read.capture_commit_revisions(observed)?;
        drop(i);
        let actual = outcome
            .media
            .participant_replacement
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        if !root.handle.is_committed() {
            return Err(Error::forbidden());
        }
        let role = match &actual.role {
            rvoip_core::participant::ParticipantRole::Observer => "observer".into(),
            rvoip_core::participant::ParticipantRole::Custom(s) => s.clone(),
            _ => return Err(Error::forbidden()),
        };
        let confirmation = match &actual.confirmation {
            EndpointConfirmationPolicy::MediaOnly => serde_json::json!({"type":"media-only"}),
            EndpointConfirmationPolicy::AnsweredDtmf { digits } => {
                serde_json::json!({"type":"answered-dtmf","digits":digits})
            }
        };
        let movement = serde_json::json!({"operation_id":actual.operation_id,"session":actual.session.as_str(),"participant":actual.participant.as_str(),"membership_epoch":actual.membership_epoch,"source":actual.replaced.as_str(),"candidate":actual.admitted.as_str(),"retained":actual.retained.as_str(),"endpoint":actual.endpoint.as_str(),"device":actual.device.as_str(),"generation":actual.generation,"participant_revision":actual.revision,"role":role,"confirmation":confirmation});
        let ready = connect_store::NativeReadyObservation {
            private_consultation: None,
            handoff_preparation: false,
            conversation_id: root.origin.request().conversation_id.clone(),
            session_id: actual.session.as_str().into(),
            runtime_connection_id: actual.admitted.as_str().into(),
            participant_id: actual.participant.as_str().into(),
            membership_epoch: actual.membership_epoch,
            endpoint_id: actual.endpoint.as_str().into(),
            device_id: actual.device.as_str().into(),
            generation: actual.generation,
            stream_revision: *outcome
                .media
                .receiver_revisions
                .get(&actual.admitted)
                .ok_or_else(Error::forbidden)?,
            participant_revision: actual.revision,
            // Immutable identities from actual resource negotiation. Video
            // remains muted; this does not assert decoded video IO readiness.
            streams: started.resource_streams.clone(),
        };
        let capture = Arc::new(
            attempt
                .committed(movement, &ready)?
                .with_audience_revisions(revisions)?,
        );
        *started.commit.lock().map_err(|_| Error::storage())? =
            Some(Capture::Known(capture.clone()));
        let recorded = super::projection::record(
            roles,
            super::diagnostics::metadata(
                &self.stop,
                &capture,
                super::diagnostics::Phase::Capture,
                || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    move || store.record_native_replacement_committed(&capture)
                },
            ),
        )
        .await;
        if let Err(error) = recorded {
            self.stop_unactivated_replacement(&capture).await;
            return Err(error);
        }
        tracing::debug!(target:"connect_service::canonical_replacement", connection=%outcome.admitted,
            stage="capture-recorded", elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        // Install metadata under the role coordinator, with no Store reader or
        // writer. The candidate's ordinary origin remains unusable until the
        // following exact completion transaction commits.
        let installed = super::projection::read(
            &self.role_projection,
            &self.stop,
            root.deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(5)),
            || async {
                for id in handle.audience().commit().connections() {
                    if id != &outcome.replaced
                        && self.native_connection_live(id)
                        && self
                            .observe_native_audio_audience_commit(id, handle.audience().commit())
                            .is_err()
                    {
                        return Err(Error::new(
                            "reconciliation-required",
                            "Known movement lost native audience confirmation",
                        ));
                    }
                }
                if !self.native_connection_live(&outcome.admitted) {
                    return Err(Error::forbidden());
                }
                adoption.install().map_err(|error| match error {
                    rvoip_core::staged_streams::StreamControlError::NotReady => Error::new(
                        "authority-busy",
                        "Replacement installation metadata is busy",
                    ),
                    _ => Error::forbidden(),
                })
            },
        )
        .await;
        match installed {
            Ok((roles, ())) => drop(roles),
            Err(error) => {
                self.stop_unactivated_replacement(&capture).await;
                return Err(error);
            }
        }
        tracing::debug!(target:"connect_service::canonical_replacement", connection=%outcome.admitted,
            stage="installed-inactive", elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        let installed = Arc::new(Installed {
            runtime: self.clone(),
            adoption,
        });
        let projected = super::diagnostics::metadata(
            &self.stop,
            &capture,
            super::diagnostics::Phase::Activation,
            || {
                let store = self.media.store.clone();
                let capture = capture.clone();
                let installed = installed.clone();
                move || store.project_native_replacement_committed(&capture, installed.as_ref())
            },
        )
        .await;
        let origin = match projected {
            Ok(origin) => origin,
            Err(error) => {
                self.stop_unactivated_replacement(&capture).await;
                return Err(error);
            }
        };
        // Publication establishes the ordinary origin used by each fresh
        // lifecycle authorization. The original captured writer can now emit
        // packet epochs; cached metadata alone never publishes private media.
        if let Err(error) = started.lifecycle.activate(self) {
            self.stop_unactivated_replacement(&capture).await;
            return Err(error);
        }
        tracing::debug!(target:"connect_service::canonical_replacement", generation=origin.generation(),
            connection=%outcome.admitted, stage="adopted", elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement actual managed movement activated");
        self.watch_moved_replacement(origin).await;
        Ok(())
    }
    /// Retain and record a live pre-Core refusal through metadata contention.
    /// Cancel only its prepared candidate; source and retained never moved.
    async fn stop_unsubmitted_replacement(
        &self,
        root: &PreparedReplacement,
        started: &StartedReplacement,
        capture: Arc<NativeReplacementCommitNotSubmitted>,
    ) {
        root.handle.cancel();
        let id = root.handle.intent().replacement.clone();
        loop {
            if self.native_connection_live(&id) {
                let _ = self
                    .core
                    .end_connection(id.clone(), EndReason::Normal)
                    .await;
            }
            let store = self.media.store.clone();
            let receipt = started.receipt.clone();
            let capture = capture.clone();
            let absent = crate::canonical_native::Absent {
                natives: self.media_adapters(),
                id: id.clone(),
                generation: root.handle.prepared_binding().generation(),
            };
            let settled = store_call(move || {
                store.record_native_replacement_commit_not_submitted(&capture)?;
                store.record_native_replacement_started_candidate_stopped(&receipt, &absent)
            })
            .await;
            if settled.is_ok() || self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.media.remove(&id);
    }

    async fn stop_unactivated_replacement(&self, capture: &Arc<NativeReplacementCommitted>) {
        let origin = capture.activation().origin().clone();
        let id = ConnectionId::from_string(origin.runtime_connection_id());
        loop {
            if self.native_connection_live(&id) {
                let _ = self
                    .core
                    .end_connection(id.clone(), EndReason::Normal)
                    .await;
            }
            let store = self.media.store.clone();
            let capture = capture.clone();
            let origin = origin.clone();
            let absent = crate::canonical_native::Absent {
                natives: self.media_adapters(),
                id: id.clone(),
                generation: origin.generation(),
            };
            let settled = store_call(move || {
                store.record_native_replacement_committed(&capture)?;
                // A final transaction may have committed despite an uncertain
                // return. Settle that exact lifetime first; otherwise project
                // the known move directly as stopped, without a Ready event.
                match store.settle_native_replacement_cleanup(&origin, &absent) {
                    Ok(()) => Ok(()),
                    Err(error) if error.code == "forbidden" => store
                        .project_native_replacement_stopped(&capture, &absent)
                        .map(|_| ()),
                    Err(error) => Err(error),
                }
            })
            .await;
            if settled.is_ok() || self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.media.remove(&id);
    }

    async fn watch_moved_replacement(&self, origin: Arc<connect_store::NativeMediaOrigin>) {
        let id = ConnectionId::from_string(origin.runtime_connection_id());
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.stop.is_cancelled() || !self.native_connection_live(&id) {
                break;
            }
            let Some(current) = self.media.origin(&id) else {
                break;
            };
            match self.media.store.try_native_connection_current(&current) {
                Ok(true) => (),
                Ok(_) => break,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => break,
                Err(_) => continue,
            }
            self.reconcile_connection_role(&id).await;
        }
        self.retire_moved_replacement(origin).await;
    }
    async fn retire_moved_replacement(&self, origin: Arc<connect_store::NativeMediaOrigin>) {
        let id = ConnectionId::from_string(origin.runtime_connection_id());
        loop {
            if self.native_connection_live(&id) {
                let _ = self
                    .core
                    .end_connection(id.clone(), EndReason::Normal)
                    .await;
            }
            let store = self.media.store.clone();
            let origin = origin.clone();
            let absent = crate::canonical_native::Absent {
                natives: self.media_adapters(),
                id: id.clone(),
                generation: origin.generation(),
            };
            if store_call(move || store.settle_native_replacement_cleanup(&origin, &absent))
                .await
                .is_ok()
            {
                break;
            }
            if self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.media.remove(&id);
    }
}

struct Installed {
    runtime: Arc<Runtime>,
    adoption: Arc<crate::canonical_media::replacement::ReplacementAdoption>,
}
impl connect_store::NativeReplacementInstallationObservation for Installed {
    fn hold_installed(
        &self,
        expected: &connect_store::NativeReplacementActivation,
    ) -> Result<Option<Box<dyn connect_store::NativeReplacementInstallationGuard + '_>>> {
        let installed = self.adoption.hold_installed(expected)?;
        if !self
            .runtime
            .native_connection_live(&ConnectionId::from_string(
                expected.origin().runtime_connection_id(),
            ))
        {
            return Err(Error::forbidden());
        }
        Ok(installed)
    }
}

pub(super) fn replacement_video(
    plan: &serde_json::Value,
) -> Result<Option<rvoip_core::staged_streams::device_replacement::ReplacementVideoRequest>> {
    use rvoip_core::staged_streams::device_replacement::{
        ReplacementVideoRequest, ReplacementVideoStream,
    };
    let Some(value) = plan.get("video") else {
        return Ok(None);
    };
    let video: connect_protocol::replacement_video::ReplacementVideo =
        serde_json::from_value(value.clone()).map_err(|_| Error::storage())?;
    video.validate()?;
    let mapping =
        |v: connect_protocol::replacement_video::ReplacementVideoStream| ReplacementVideoStream {
            previous: rvoip_core::ids::StreamId::from_string(v.previous),
            replacement: rvoip_core::ids::StreamId::from_string(v.replacement),
        };
    Ok(Some(ReplacementVideoRequest {
        publications: video.publications.into_iter().map(mapping).collect(),
        receivers: video.receivers.into_iter().map(mapping).collect(),
    }))
}
