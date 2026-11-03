//! One actual managed switch followed only by captured metadata projection.
use super::*;
use super::{continuation::Phase, start::StartedReplacement};
use connect_store::{NativeReplacementCommitUncertain, NativeReplacementCommitted};
use rvoip_core::{adapter::EndReason, participant_transitions::EndpointConfirmationPolicy};
#[derive(Clone)]
pub(super) enum Capture {
    Known(Arc<NativeReplacementCommitted>),
    Unknown(Arc<NativeReplacementCommitUncertain>),
}
impl Runtime {
    pub(super) async fn move_native_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
    ) -> Result<()> {
        let (roles, i) = super::projection::read(&self.role_projection, &self.stop,
            root.deadline.min(tokio::time::Instant::now() + Duration::from_secs(5)), || async {
                let plan = self.core.plan_staged_device_replacement(&root.handle).map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="plan",?error,"Replacement pre-commit refused");
                    Error::forbidden()
                })?;
                self.continuation_read_once(root, started, Phase::Commit(plan)).await
            }).await.map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",stage="scene-read",code=%error.code,"Replacement pre-commit refused");
                error
            })?;
        let Phase::Commit(plan) = i.phase.clone() else {
            return Err(Error::forbidden());
        };
        let owner = started
            .policy
            .current_owner(i.read.media())
            .map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",stage="current-owner",?error,"Replacement pre-commit refused");
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
                tracing::debug!(target:"connect_service::canonical_replacement",stage="commit-reservation",code=%error.code,"Replacement pre-commit refused");
                error
            })?
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
        super::projection::record(
            roles,
            retry_metadata(&self.stop, || {
                let store = self.media.store.clone();
                let capture = capture.clone();
                store_call(move || store.record_native_replacement_committed(&capture))
            }),
        )
        .await?;
        tracing::debug!(target:"connect_service::canonical_replacement",
            connection=%outcome.admitted, stage="capture-recorded",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.project_native_replacement_committed(&capture))
        })
        .await?;
        tracing::debug!(target:"connect_service::canonical_replacement",
            connection=%outcome.admitted, stage="established-projected",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        // Create historical write custody before acquiring the adoption reader
        // and before making the ordinary Data route visible. This is metadata
        // only; no publication, delivery, or original effect is repeated.
        let data_writer = if let Some(observer) = &started.policy.data_observer {
            let scope = observer.scope().clone();
            let result = retry_metadata(&self.stop, || {
                let store = self.media.store.clone();
                let origin = origin.clone();
                let scope = scope.clone();
                store_call(move || {
                    crate::canonical_media::data_writes::Observer::new(store, &origin, scope)
                })
            })
            .await;
            match result {
                Ok(writer) => Some(writer),
                Err(error) => {
                    self.retire_moved_replacement(origin).await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        tracing::debug!(target:"connect_service::canonical_replacement",
            connection=%outcome.admitted, stage="data-observer-recorded",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        let read = super::projection::read(
            &self.role_projection,
            &self.stop,
            root.deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(5)),
            || {
                let store = self.media.store.clone();
                let origin = origin.clone();
                store_call(move || store.try_native_media_read(&origin, None))
            },
        )
        .await;
        let (_roles, read) = match read {
            Ok(value) => value,
            Err(error) => {
                self.retire_moved_replacement(origin).await;
                return Err(error);
            }
        };
        tracing::debug!(target:"connect_service::canonical_replacement",
            connection=%outcome.admitted, stage="adoption-authority-read",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement commit phase observed");
        // Preserve the known movement before any later confirmation failure.
        // Failure here ends only this new route; it cannot roll back the commit.
        let mut confirmed = true;
        for id in handle.audience().commit().connections() {
            if id != &outcome.replaced
                && self.native_connection_live(id)
                && self
                    .observe_native_audio_audience_commit(id, handle.audience().commit())
                    .is_err()
            {
                confirmed = false;
            }
        }
        if !confirmed || !self.native_connection_live(&outcome.admitted) {
            drop(read);
            drop(_roles);
            self.retire_moved_replacement(origin).await;
            return Err(Error::new(
                "reconciliation-required",
                "Known movement lost native confirmation",
            ));
        }
        let adopted = self.media.adopt_replacement(
            &started.policy,
            origin.clone(),
            owner.connection,
            &read,
            data_writer,
        );
        drop(read);
        if adopted.is_err() {
            drop(_roles);
            self.retire_moved_replacement(origin).await;
            return Err(Error::forbidden());
        }
        tracing::debug!(target:"connect_service::canonical_replacement",generation=origin.generation(),
            connection=%outcome.admitted, stage="adopted",
            elapsed_us=commit_began.elapsed().as_micros() as u64,
            "Replacement actual managed movement committed");
        drop(_roles);
        self.watch_moved_replacement(origin).await;
        Ok(())
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
                natives: self.native_adapters(),
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
