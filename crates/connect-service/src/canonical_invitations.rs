//! Bounded metadata-only invitation delivery to authenticated live physical
//! peers. A verified response resolves its original source through fresh policy.
use crate::{Service, StoreCredentialBinding};
use connect_protocol::{Error, Result};
use connect_store::{
    AuthorityWait, PeerAuthorityRequest, PeerCommand, PeerDeviceClaim, SessionInvitationDecision,
    SessionInvitationDecisionClaim, SessionInvitationSource,
};
use rvoip_uctp::v2::{
    reachability::{ReachabilityRegistry, VerifiedRouteReply},
    Envelope, Identifier, Payload,
};
use serde_json::json;
use std::{
    sync::{Arc, Weak},
    time::Duration,
};
mod callers;
pub(crate) use callers::{Caller, Callers};
struct Job {
    runtime: Weak<crate::canonical_runtime::Runtime>,
    id: String,
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            if let Ok(mut jobs) = runtime.invitation_jobs.lock() {
                jobs.remove(&self.id);
            }
        }
    }
}
impl Service {
    pub fn install_canonical_invitation_routes(
        &self,
        routes: Arc<ReachabilityRegistry>,
    ) -> Result<()> {
        tokio::runtime::Handle::try_current().map_err(|_| Error::storage())?;
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        runtime
            .routes
            .set(routes)
            .map_err(|_| Error::new("conflict", "Canonical invitation routes already installed"))?;
        runtime.install_session_reconnect_worker()?;
        runtime.install_session_lifecycle_worker()
    }
    pub(crate) fn queue_session_invitations(self: &Arc<Self>, record_id: String) {
        let Some(runtime) = self.canonical_runtime.get().cloned() else {
            return;
        };
        let Some(registry) = runtime.routes.get().cloned() else {
            return;
        };
        let service = Arc::downgrade(self);
        let stop = runtime.stop.clone();
        let store = self.store.clone();
        let owner = Arc::downgrade(&runtime);
        let Ok(mut work) = runtime.work.lock() else {
            return;
        };
        while work.try_join_next().is_some() {}
        if stop.is_cancelled() || work.len() >= self.store.limits().connections {
            return;
        }
        work.spawn(async move {
            let loaded = tokio::task::spawn_blocking(move || {
                Ok::<_, Error>((
                    store.installation_id()?,
                    store.session_invitation_sources(&record_id)?,
                ))
            })
            .await;
            let Ok(Ok((installation, sources))) = loaded else {
                return;
            };
            for source in sources {
                let Some(service) = service.upgrade() else {
                    return;
                };
                let Some(runtime) = owner.upgrade() else {
                    return;
                };
                // Registration precedes the decision lookup. If a decision is
                // already committed, a separate relay does not depend on the
                // original receiver job's lifetime/deduplication slot.
                let store = service.store.clone();
                let saved_source = source.clone();
                let saved = tokio::task::spawn_blocking(move || {
                    store.session_invitation_decision_template(&saved_source)
                })
                .await;
                if let Ok(Ok(Some(outcome))) = saved {
                    let _ = service.relay_session_decision(&source, &outcome).await;
                    continue;
                }
                let id = format!(
                    "{}:{}",
                    source.invitation_id(),
                    source.target_participant_id()
                );
                let Ok(mut jobs) = runtime.invitation_jobs.lock() else {
                    return;
                };
                if jobs.len() >= service.store.limits().connections || !jobs.insert(id.clone()) {
                    continue;
                }
                drop(jobs);
                let job = Job {
                    runtime: Arc::downgrade(&runtime),
                    id,
                };
                let registry = registry.clone();
                let installation = installation.clone();
                let stop = stop.clone();
                let Ok(mut work) = runtime.work.lock() else {
                    return;
                };
                while work.try_join_next().is_some() {}
                if stop.is_cancelled() || work.len() >= service.store.limits().connections {
                    continue;
                }
                work.spawn(async move {
                    let _job = job;
                    let _ = service
                        .run_session_invitation(registry, installation, source, stop)
                        .await;
                });
            }
        });
    }
    async fn run_session_invitation(
        self: &Arc<Self>,
        registry: Arc<ReachabilityRegistry>,
        installation: String,
        source: SessionInvitationSource,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        let identity = Identifier::try_from(source.target_identity_id().to_owned())
            .map_err(|_| Error::forbidden())?;
        let owner = rvoip_auth_core::PrincipalOwnershipKey {
            issuer: Some("connect-local".into()),
            tenant: Some(installation),
            subject: identity.as_str().into(),
        };
        let routes = registry
            .routes_for(&owner, &identity)
            .map_err(|_| Error::forbidden())?;
        let store = self.store.clone();
        let original = source.clone();
        // Reconstruct original semantic metadata using a current live route's
        // actual credential only inside its delivery policy, never here under
        // a long-lived reader. The template is made from immutable stored data.
        let metadata =
            tokio::task::spawn_blocking(move || store.session_invitation_template(&original))
                .await
                .map_err(|_| Error::storage())??;
        let template=Envelope::from_value(json!({"v":2,"type":"session.invite","id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":metadata.conversation_id,"sid":metadata.session_id,"operation_id":metadata.invitation_id,"payload":{"from":metadata.from_participant_id,"to":[metadata.target_participant_id],"medium":metadata.medium,"intent":metadata.intent,"capabilities_offer":metadata.capabilities_offer}})).map_err(|_|Error::storage())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut pending = tokio::task::JoinSet::new();
        for route in routes {
            let request = template.clone();
            let service = self.clone();
            let source = source.clone();
            let stop = stop.clone();
            pending.spawn(async move {
                let result = service
                    .wait_session_invitation(&route, &source, request, deadline, stop)
                    .await;
                (route, result)
            });
        }
        while let Some(result) = tokio::select! {biased;_=stop.cancelled()=>return Err(Error::forbidden()), result=pending.join_next()=>result}
        {
            if let Ok((route, Ok(observed))) = result {
                let request_id = observed.request_id().as_str().to_owned();
                if let Ok(outcome) = self
                    .commit_session_invitation(source.clone(), observed)
                    .await
                {
                    self.notify_changed();
                    self.relay_session_decision(&source, &outcome).await?;
                    return Ok(());
                }
                // A cancellation can win between response verification and
                // decision commit. Its durable source still governs the hint.
                let _ = self
                    .send_invitation_cancellation(&route, &source, &request_id)
                    .await;
            }
        }
        Err(Error::new(
            "unavailable",
            "No current invitation decision observed",
        ))
    }
    async fn wait_session_invitation(
        &self,
        route: &rvoip_uctp::v2::reachability::RouteHandle,
        source: &SessionInvitationSource,
        request: Envelope,
        deadline: tokio::time::Instant,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<VerifiedRouteReply> {
        let submitted = route
            .begin_request(request, deadline)
            .await
            .map_err(|_| Error::forbidden())?;
        let request_id = submitted.envelope_id().as_str().to_owned();
        let observed = submitted.wait_reply();
        tokio::pin!(observed);
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Err(Error::forbidden()),
                reply = &mut observed => return reply.map_err(|_| Error::forbidden()),
                _ = self.changed.notified() => (),
                _ = tick.tick() => (),
            }
            if self
                .send_invitation_cancellation(route, source, &request_id)
                .await?
            {
                return Err(Error::new("conflict", "Original invitation cancelled"));
            }
        }
    }
    async fn send_invitation_cancellation(
        &self,
        route: &rvoip_uctp::v2::reachability::RouteHandle,
        source: &SessionInvitationSource,
        request_id: &str,
    ) -> Result<bool> {
        let store = self.store.clone();
        let original = source.clone();
        let cancelled = tokio::task::spawn_blocking(move || {
            store.session_invitation_cancellation_template(&original)
        })
        .await
        .map_err(|_| Error::storage())??;
        let Some(cancelled) = cancelled else {
            return Ok(false);
        };
        let envelope = Envelope::from_value(json!({"v":2,"type":"session.cancel","id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":cancelled.conversation_id,"sid":cancelled.session_id,"operation_id":source.invitation_id(),"in_reply_to":request_id,"payload":{"by":cancelled.by_participant_id,"reason_code":cancelled.reason_code,"reason":cancelled.reason}})).map_err(|_|Error::storage())?;
        // One bounded hint attempt; uncertainty never retries the mutation.
        // The route reacquires exact recipient read authority for actual IO.
        let _ = route
            .submit(
                envelope,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await;
        Ok(true)
    }
    async fn commit_session_invitation(
        &self,
        source: SessionInvitationSource,
        observed: VerifiedRouteReply,
    ) -> Result<connect_store::SessionInvitationDecisionOutcome> {
        if !observed.current() || observed.identity().as_str() != source.target_identity_id() {
            return Err(Error::forbidden());
        }
        // A correlated observation is not an ordinary command, but committing
        // its decision still requires the same immutable host assurance floor.
        // This never upgrades a credential or fabricates a blocked command.
        if self.assurance_policy.get().is_some_and(|policy| {
            !policy.permits(observed.envelope(), &observed.credential().principal)
        }) {
            return Err(Error::forbidden());
        }
        let decision = match observed.envelope().payload() {
            Payload::SessionAccept(v) => SessionInvitationDecision::Accept {
                by_participant: v.by.as_ref().map(|id| id.as_str().into()),
                capabilities: v.capabilities_answer.clone(),
            },
            Payload::SessionReject(v) => SessionInvitationDecision::Reject {
                by_participant: v.by.as_ref().map(|id| id.as_str().into()),
                reason_code: v.reason_code,
                reason: v.reason.clone(),
            },
            _ => return Err(Error::forbidden()),
        };
        let claim = SessionInvitationDecisionClaim { source, decision };
        let signature = &observed.envelope().original()["signature"];
        let credential = observed.credential().clone();
        let device = observed.device();
        let request = PeerAuthorityRequest {
            credentials: vec![credential.token_id.clone().ok_or_else(Error::forbidden)?],
            device: PeerDeviceClaim {
                id: device.id.as_str().into(),
                kind: device.kind.clone(),
                platform: device.platform.clone(),
            },
            key_id: signature["keyid"]
                .as_str()
                .ok_or_else(Error::forbidden)?
                .into(),
            algorithm: signature["alg"]
                .as_str()
                .ok_or_else(Error::forbidden)?
                .into(),
            command: PeerCommand::SessionInvitationDecision(claim.clone()),
        };
        let store = self.store.clone();
        let wait = AuthorityWait::new(Duration::from_secs(2));
        let _cancel = wait.cancel_on_drop();
        tokio::task::spawn_blocking(move || {
            if !observed.current() {
                return Err(Error::forbidden());
            }
            let proof = store.admit_peer_authority(&request, &wait)?;
            let binding = StoreCredentialBinding::from_validated(&store, &credential)?;
            store.apply_session_invitation_decision(binding.credential_id(), claim, &proof, &|| {
                observed.current()
            })
        })
        .await
        .map_err(|_| Error::storage())?
    }
}
