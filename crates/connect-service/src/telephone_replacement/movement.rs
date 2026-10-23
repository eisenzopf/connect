//! Actual managed telephone movement, retained capture and independent lifetime.
use super::continuation::Phase;
use super::*;
use crate::{
    canonical_consultation::{retry_metadata, store_call},
    canonical_replacement::PreparedReplacement,
};
use connect_store::{
    TelephoneMediaOrigin, TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
};
use rvoip_core::{
    adapter::EndReason, ids::ConnectionId, participant_transitions::EndpointConfirmationPolicy,
};
#[derive(Clone)]
pub(crate) enum Capture {
    Known(Arc<TelephoneReplacementCommitted>),
    Unknown(Arc<TelephoneReplacementCommitUncertain>),
}
impl Runtime {
    pub(super) async fn move_telephone_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        leg: &Arc<allocation::Allocated>,
        started: &Arc<media::Started>,
    ) -> Result<()> {
        let roles = self.role_projection.lock().await;
        let plan=self.core.plan_staged_device_replacement(&root.handle).map_err(|error|{tracing::debug!(target:"connect_service::telephone_replacement",?error,"Telephone compound plan refused");Error::forbidden()})?;
        let i = self
            .telephone_scene(root, started, Phase::Commit(plan.clone()))
            .await?;
        let owner = started
            .policy
            .owner(i.read.media())
            .map_err(|_| Error::forbidden())?;
        let store = self.media.store.clone();
        let read = i.read.clone();
        let attempt = store_call(move || store.begin_telephone_replacement_commit(&read)).await?;
        let result = self
            .core
            .commit_staged_device_replacement_with_held(plan, &i)
            .await;
        let (outcome, handle) = match result {
            Ok(actual) => actual,
            Err(error) => {
                drop(i);
                let capture = Arc::new(attempt.uncertain());
                *started.commit.lock().map_err(|_| Error::storage())? =
                    Some(Capture::Unknown(capture.clone()));
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    store_call(move || {
                        store.record_telephone_replacement_commit_uncertain(&capture)
                    })
                })
                .await?;
                tracing::debug!(target:"connect_service::telephone_replacement",?error,"Telephone movement uncertain");
                return Err(Error::forbidden());
            }
        };
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
            rvoip_core::participant::ParticipantRole::Custom(role) => role.clone(),
            _ => return Err(Error::forbidden()),
        };
        let confirmation = match &actual.confirmation {
            EndpointConfirmationPolicy::MediaOnly => serde_json::json!({"type":"media-only"}),
            EndpointConfirmationPolicy::AnsweredDtmf { digits } => {
                serde_json::json!({"type":"answered-dtmf","digits":digits})
            }
        };
        let movement = serde_json::json!({"operation_id":actual.operation_id,"session":actual.session.as_str(),"participant":actual.participant.as_str(),"membership_epoch":actual.membership_epoch,"source":actual.replaced.as_str(),"candidate":actual.admitted.as_str(),"retained":actual.retained.as_str(),"endpoint":actual.endpoint.as_str(),"device":actual.device.as_str(),"generation":actual.generation,"participant_revision":actual.revision,"role":role,"confirmation":confirmation});
        let stream_revision = *outcome
            .media
            .receiver_revisions
            .get(&actual.admitted)
            .ok_or_else(Error::forbidden)?;
        let capture = Arc::new(attempt.committed(
            movement,
            outcome.new_stream.logical.as_str(),
            stream_revision,
            revisions,
        )?);
        *started.commit.lock().map_err(|_| Error::storage())? =
            Some(Capture::Known(capture.clone()));
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.record_telephone_replacement_committed(&capture))
        })
        .await?;
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.project_telephone_replacement_committed(&capture))
        })
        .await?;
        let mut confirmed = true;
        for id in handle.audience().commit().connections() {
            if id != &outcome.replaced
                && id != &outcome.admitted
                && self.native.is_connection_live(id)
                && self
                    .native
                    .observe_audio_audience_commit(id, handle.audience().commit())
                    .is_err()
            {
                confirmed = false;
            }
        }
        // Public status polls and the retained native packet readers can briefly
        // own SQL here. A busy read is not evidence that the new route lost its
        // authority; retry only metadata, never the already committed switch.
        let current = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let origin = origin.clone();
            store_call(move || store.try_telephone_media_read(origin))
        })
        .await;
        if !confirmed || current.is_err() || !started.policy.physical_current() {
            tracing::debug!(target:"connect_service::telephone_replacement",confirmed,code=?current.as_ref().err().map(|e|e.code.as_str()),physical=started.policy.physical_current(),"Known telephone target adoption refused");
            drop(current);
            drop(roles);
            self.retire_telephone_connection(&origin, leg).await;
            return Err(Error::new(
                "reconciliation-required",
                "Known telephone movement lost target authority",
            ));
        }
        drop(current);
        started
            .policy
            .establish(origin.clone(), owner)
            .map_err(|_| Error::forbidden())?;
        leg.writer
            .established
            .set(origin.clone())
            .map_err(|_| Error::forbidden())?;
        self.media.remove(&outcome.replaced);
        drop(roles);
        tracing::debug!(target:"connect_service::telephone_replacement",generation=origin.snapshot().generation,"Telephone actual managed movement committed");
        self.watch_telephone_connection(origin, leg, started).await;
        Ok(())
    }
    async fn watch_telephone_connection(
        self: &Arc<Self>,
        origin: Arc<TelephoneMediaOrigin>,
        leg: &Arc<allocation::Allocated>,
        started: &Arc<media::Started>,
    ) {
        let id = ConnectionId::from_string(origin.snapshot().runtime_connection_id.clone());
        let signal_leg = crate::canonical_sip_signal::SipLeg::Established(started.policy.clone());
        let Ok((adapter, _)) = self.telephone_route(&leg.route.policy.route_reference) else {
            return;
        };
        let mut received = None;
        let mut next_check = tokio::time::Instant::now();
        loop {
            if self.stop.is_cancelled() || signal_leg.cancelled() {
                break;
            }
            if tokio::time::Instant::now() >= next_check {
                // Existing RTP and each named signal direction are independent.
                // All reads are released before metadata waits or queued inputs.
                let media_current = match self.media.store.try_telephone_media_read(origin.clone())
                {
                    Ok(read) => read.current(),
                    Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => {
                        false
                    }
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        continue;
                    }
                };
                if !media_current {
                    let mut signal_current = false;
                    let mut busy = false;
                    for direction in [
                        connect_store::TelephoneSignalDirection::FromTelephone,
                        connect_store::TelephoneSignalDirection::ToTelephone,
                    ] {
                        match self
                            .media
                            .store
                            .try_telephone_signal_read(origin.clone(), direction)
                        {
                            Ok(read) => signal_current |= read.current(),
                            Err(error)
                                if matches!(error.code.as_str(), "forbidden" | "unauthorized") =>
                            {
                                ()
                            }
                            Err(_) => busy = true,
                        }
                    }
                    if !signal_current && !busy {
                        break;
                    }
                    if !signal_current {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        continue;
                    }
                }
                self.reconcile_connection_role(&id).await;
                next_check = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
            }
            if received.is_none() {
                if let (Ok(controller), Ok(principal)) =
                    (signal_leg.controller(), signal_leg.principal())
                {
                    if let Ok(subscription) = adapter.subscribe_managed_dtmf_source(
                        &self.core,
                        id.clone(),
                        &controller,
                        principal,
                    ) {
                        received = Some(subscription);
                    }
                }
            }
            if let Some(subscription) = received.as_mut() {
                tokio::select! {biased;
                    _=self.stop.cancelled()=>break,
                    input=subscription.recv()=>match input {
                        Some(observed)=>if let Err(error)=self.enqueue_received_dtmf(signal_leg.clone(),observed) {
                            tracing::debug!(code=%error.code,"Established SIP received signal refused before journal admission");
                        },
                        None=>received=None,
                    },
                    _=tokio::time::sleep(std::time::Duration::from_millis(25))=>{},
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
        drop(received);
        self.retire_telephone_connection(&origin, leg).await;
    }
    async fn retire_telephone_connection(
        &self,
        origin: &Arc<TelephoneMediaOrigin>,
        leg: &allocation::Allocated,
    ) {
        let id = ConnectionId::from_string(origin.snapshot().runtime_connection_id.clone());
        let Ok((adapter, _)) = self.telephone_route(&leg.route.policy.route_reference) else {
            return;
        };
        loop {
            if adapter.is_connection_live(&id) {
                let _ = self
                    .core
                    .end_connection(id.clone(), EndReason::Normal)
                    .await;
            }
            let absent = Absent {
                adapter: adapter.clone(),
                id: id.clone(),
                generation: origin.snapshot().generation,
            };
            let store = self.media.store.clone();
            let origin = origin.clone();
            if store_call(move || store.settle_telephone_connection_cleanup(&origin, &absent))
                .await
                .is_ok()
            {
                break;
            }
            if self.stop.is_cancelled() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        self.media.remove_telephone_target(&id);
    }
}
struct Absent {
    adapter: Arc<SipAdapter>,
    id: ConnectionId,
    generation: u64,
}
impl connect_store::TelephoneConnectionRuntimeObservation for Absent {
    fn observe_absent(&self, id: &str, generation: u64) -> Result<()> {
        if id != self.id.as_str()
            || generation != self.generation
            || self.adapter.is_connection_live(&self.id)
        {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone route absence is not confirmed",
            ));
        }
        Ok(())
    }
}
