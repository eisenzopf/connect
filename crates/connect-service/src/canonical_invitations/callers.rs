//! Transient caller correlations. Durable decisions live in Store; a process
//! restart cannot preserve a physical caller, so replay registers a new one.
use super::*;
use rvoip_uctp::v2::{auth_dispatch::AuthorizedEnvelope, reachability::RouteHandle, Scope};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub(crate) struct Caller {
    peer: Arc<rvoip_core::peer_admission::PeerAdmission>,
    owner: rvoip_auth_core::PrincipalOwnershipKey,
    request: String,
    operation: String,
    conversation: String,
    session: String,
}
impl Caller {
    pub(crate) fn capture(a: &AuthorizedEnvelope) -> Result<Self> {
        let e = a.envelope();
        if e.kind() != "session.invite" || e.scope(Scope::Reply).is_some() {
            return Err(Error::forbidden());
        }
        Ok(Self {
            peer: a.peer_admission().ok_or_else(Error::forbidden)?.clone(),
            owner: a.principal().ownership_key(),
            request: e.id().into(),
            operation: e
                .scope(Scope::Operation)
                .ok_or_else(Error::forbidden)?
                .into(),
            conversation: e
                .scope(Scope::Conversation)
                .ok_or_else(Error::forbidden)?
                .into(),
            session: e.scope(Scope::Session).ok_or_else(Error::forbidden)?.into(),
        })
    }
}
struct Registration {
    route: RouteHandle,
    request: String,
    operation: String,
    conversation: String,
    session: String,
    expires: tokio::time::Instant,
    attempted: BTreeSet<String>,
}
#[derive(Default)]
pub(crate) struct Callers {
    sources: BTreeMap<String, Vec<Registration>>,
}
impl Callers {
    fn prune(&mut self) {
        let now = tokio::time::Instant::now();
        self.sources.retain(|_, entries| {
            entries.retain(|e| e.expires > now && e.route.physical_is_live());
            !entries.is_empty()
        });
    }
    pub(crate) fn register(
        &mut self,
        registry: &ReachabilityRegistry,
        source: &SessionInvitationSource,
        caller: &Caller,
        capacity: usize,
    ) -> Result<()> {
        self.prune();
        let identity =
            Identifier::try_from(caller.owner.subject.clone()).map_err(|_| Error::forbidden())?;
        let route = registry
            .routes_for(&caller.owner, &identity)
            .map_err(|_| Error::forbidden())?
            .into_iter()
            .find(|route| route.matches_peer(&caller.peer))
            .ok_or_else(Error::forbidden)?;
        let count: usize = self.sources.values().map(Vec::len).sum();
        let entries = self
            .sources
            .entry(source.invitation_id().into())
            .or_default();
        if let Some(old) = entries
            .iter_mut()
            .find(|e| e.route.same_physical_peer(&route))
        {
            if old.request == caller.request {
                return Ok(());
            }
            *old = Registration {
                route,
                request: caller.request.clone(),
                operation: caller.operation.clone(),
                conversation: caller.conversation.clone(),
                session: caller.session.clone(),
                expires: tokio::time::Instant::now() + Duration::from_secs(60),
                attempted: Default::default(),
            };
        } else {
            if count >= capacity.saturating_mul(16) || entries.len() >= 64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Invitation caller capacity exceeded",
                ));
            }
            entries.push(Registration {
                route,
                request: caller.request.clone(),
                operation: caller.operation.clone(),
                conversation: caller.conversation.clone(),
                session: caller.session.clone(),
                expires: tokio::time::Instant::now() + Duration::from_secs(60),
                attempted: Default::default(),
            });
        }
        Ok(())
    }
    pub(crate) fn owns_cancellation(&mut self, a: &AuthorizedEnvelope) -> bool {
        self.prune();
        let e = a.envelope();
        let Some(peer) = a.peer_admission() else {
            return false;
        };
        self.sources.values().flatten().any(|entry| {
            e.scope(Scope::Reply) == Some(entry.request.as_str())
                && e.scope(Scope::Conversation) == Some(entry.conversation.as_str())
                && e.scope(Scope::Session) == Some(entry.session.as_str())
                && entry.route.same_physical_admission(peer)
        })
    }
    fn claim(&mut self, source: &SessionInvitationSource) -> Vec<(RouteHandle, String, String)> {
        self.prune();
        let Some(entries) = self.sources.get_mut(source.invitation_id()) else {
            return vec![];
        };
        entries
            .iter_mut()
            .filter(|entry| entry.route.is_current())
            .filter_map(|entry| {
                entry
                    .attempted
                    .insert(source.target_participant_id().into())
                    .then(|| {
                        (
                            entry.route.clone(),
                            entry.request.clone(),
                            entry.operation.clone(),
                        )
                    })
            })
            .collect()
    }
}
impl Service {
    /// Remember the verified command before its actual ACK can reach the caller.
    /// Enumeration is protected metadata only, under the existing mutation fence.
    pub(crate) async fn remember_session_invitation_caller(
        &self,
        record: &str,
        caller: &Caller,
    ) -> Result<()> {
        let Some(runtime) = self.canonical_runtime.get() else {
            return Ok(());
        };
        let Some(registry) = runtime.routes.get() else {
            return Ok(());
        };
        let store = self.store.clone();
        let record = record.to_owned();
        let sources =
            tokio::task::spawn_blocking(move || store.session_invitation_sources(&record))
                .await
                .map_err(|_| Error::storage())??;
        let mut callers = runtime
            .invitation_callers
            .lock()
            .map_err(|_| Error::storage())?;
        for source in sources {
            callers.register(registry, &source, caller, self.store.limits().connections)?;
        }
        Ok(())
    }
    pub(super) async fn relay_session_decision(
        &self,
        source: &SessionInvitationSource,
        outcome: &connect_store::SessionInvitationDecisionOutcome,
    ) -> Result<()> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let attempts = runtime
            .invitation_callers
            .lock()
            .map_err(|_| Error::storage())?
            .claim(source);
        for (route, reply_to, operation) in attempts {
            if runtime.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            let (kind, payload) = match &outcome.decision {
                SessionInvitationDecision::Accept { capabilities, .. } => (
                    "session.accept",
                    json!({"by":outcome.participant_id,"capabilities_answer":capabilities}),
                ),
                SessionInvitationDecision::Reject {
                    reason_code,
                    reason,
                    ..
                } => (
                    "session.reject",
                    json!({"by":outcome.participant_id,"reason_code":reason_code,"reason":reason}),
                ),
            };
            let envelope=Envelope::from_value(json!({"v":2,"type":kind,"id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":outcome.session.conversation_id,"sid":outcome.session.id,"operation_id":operation,"in_reply_to":reply_to,"payload":payload})).map_err(|_|Error::storage())?;
            // One attempt per caller request/target. Uncertain notification IO
            // never retries a mutation; logical replay gets a new correlation.
            let _ = route
                .submit(
                    envelope,
                    tokio::time::Instant::now() + Duration::from_secs(2),
                )
                .await;
        }
        Ok(())
    }
}
