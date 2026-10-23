//! Captured provider custody and complete current source intervals.
use super::*;
use crate::{
    AssistantProviderApplied, AssistantProviderContext, AssistantProviderRead, AuthorityGuardSet,
    AuthorityMode, AuthorityScope, PrivateProviderApplied, PrivateProviderContext,
    PrivateProviderMediaRead,
};
use std::sync::Arc;
#[derive(Clone)]
pub(super) enum Memory {
    Public {
        context: AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
    },
    Private {
        context: PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
    },
}
pub(crate) struct Facts {
    pub cid: String,
    pub sid: String,
    pub connection: String,
    pub pid: String,
    pub epoch: u64,
    pub generation: u64,
    pub audience: Vec<(String, u64)>,
    pub scopes: Vec<AuthorityScope>,
    pub expires_at: i64,
}
impl Memory {
    pub(super) fn identity(&self) -> (&str, &str, &str) {
        match self {
            Self::Public { context, applied } => {
                ("public", context.delivery_id(), applied.provider_ref())
            }
            Self::Private { context, applied } => {
                ("private", context.delivery_id(), applied.provider_ref())
            }
        }
    }
    pub(super) fn contains_question(&self, c: &Connection, question: &str) -> Result<bool> {
        let mut statement=c.prepare("SELECT selection_json FROM context_deliveries WHERE id=?1 UNION ALL SELECT d.selection_json FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.instance_delivery_id=?1 AND u.delivered_at IS NOT NULL AND d.state='delivered'").map_err(|_|Error::storage())?;
        let rows = statement
            .query_map([self.identity().1], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?;
        for encoded in rows {
            let view: connect_protocol::ContextView =
                serde_json::from_str(&encoded.map_err(|_| Error::storage())?)
                    .map_err(|_| Error::storage())?;
            if view
                .items
                .iter()
                .any(|i| i.kind == "message" && i.source_id == question)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn facts(&self, store: &Store, c: &Connection) -> Result<Facts> {
        match self {
            Self::Public { context, applied } => {
                store.native_assistant_observation_facts(c, context, applied)
            }
            Self::Private { context, applied } => {
                store.private_provider_observation_facts(c, context, applied)
            }
        }
    }
    pub(super) fn check(
        &self,
        store: &Store,
        c: &Connection,
        route: &Route,
    ) -> Result<Vec<AuthorityScope>> {
        let facts = self.facts(store, c)?;
        let mut audience = facts.audience;
        audience.sort();
        if facts.cid != route.cid
            || facts.sid != route.sid
            || facts.connection != route.connection
            || facts.pid != route.pid
            || facts.epoch != route.epoch
            || facts.generation != route.generation
            || audience != route.audience
        {
            return Err(Error::forbidden());
        }
        Ok(facts.scopes)
    }
}
fn unavailable(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "unauthorized"
            | "forbidden"
            | "not-ready"
            | "reconciliation-required"
            | "private-context-required"
            | "unsupported-media-profile"
            | "conflict"
            | "gone"
    )
}
pub(super) fn current(store: &Store, c: &Connection, route: &Route) -> Result<bool> {
    match &route.memory {
        None => Ok(true),
        Some(memory) => match memory.check(store, c, route) {
            Ok(_) => Ok(true),
            Err(error) if unavailable(&error) => Ok(false),
            Err(error) => Err(error),
        },
    }
}
pub(super) fn scopes(store: &Store, c: &Connection, route: &Route) -> Result<Vec<AuthorityScope>> {
    let mut scopes =
        crate::authority_admission::conversation_scopes(c, &route.cid, AuthorityMode::Read)?;
    scopes.push(AuthorityScope::identity(&route.actor, AuthorityMode::Read)?);
    if let Some(memory) = &route.memory {
        scopes.extend(memory.check(store, c, route)?);
    }
    AuthorityScope::merged(&scopes)
}
impl Store {
    /// Protected composition only. The destination, audience and correlation
    /// derive from an acknowledged captured instance, never provider/model IDs.
    pub fn bind_native_assistant_observation_source(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
    ) -> Result<SessionObservationSource> {
        self.bind_native_assistant_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            None,
            None,
        )
    }
    pub fn bind_native_assistant_observation_source_with_activity(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Arc<dyn SessionObservationActivity>,
    ) -> Result<SessionObservationSource> {
        self.bind_native_assistant_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            Some(activity),
            None,
        )
    }
    pub fn bind_native_assistant_observation_source_with_work(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Arc<dyn SessionObservationActivity>,
        work: Option<crate::TaskObservationBinding>,
    ) -> Result<SessionObservationSource> {
        self.bind_native_assistant_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            Some(activity),
            work,
        )
    }
    fn bind_native_assistant_observation_source_restricted(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Option<Arc<dyn SessionObservationActivity>>,
        work: Option<crate::TaskObservationBinding>,
    ) -> Result<SessionObservationSource> {
        let memory = Memory::Public {
            context: context.clone(),
            applied,
        };
        let facts = {
            let c = self.connection()?;
            memory.facts(self, &c)?
        };
        let participants = facts
            .audience
            .into_iter()
            .map(|(pid, _)| pid)
            .collect::<Vec<_>>();
        self.bind_observation_source_inner(
            publisher,
            &facts.connection,
            "vapi",
            context.delivery_id(),
            &participants,
            capabilities,
            Some(memory),
            activity,
            work,
        )
    }
    /// Reuse the caller's original complete model-memory interval. This cannot
    /// enter a second reader behind a queued writer or upgrade its authority.
    pub fn publish_native_assistant_observation(
        &self,
        source: &SessionObservationSource,
        read: &AssistantProviderRead,
        event: &str,
        data: ObservationDraft,
    ) -> Result<String> {
        let route = self.observation_route(source)?;
        let Some(Memory::Public { context, .. }) = &route.memory else {
            return Err(Error::forbidden());
        };
        let interval = read.observation_interval(context)?;
        self.publish_observation_inner(source, event, data, Some(interval))
    }
    /// The private purpose and original pair come from its captured context.
    /// This API cannot convert a public instance into a private observation source.
    pub fn bind_private_provider_observation_source(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
    ) -> Result<SessionObservationSource> {
        self.bind_private_provider_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            None,
            None,
        )
    }
    pub fn bind_private_provider_observation_source_with_activity(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Arc<dyn SessionObservationActivity>,
    ) -> Result<SessionObservationSource> {
        self.bind_private_provider_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            Some(activity),
            None,
        )
    }
    pub fn bind_private_provider_observation_source_with_work(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Arc<dyn SessionObservationActivity>,
        work: Option<crate::TaskObservationBinding>,
    ) -> Result<SessionObservationSource> {
        self.bind_private_provider_observation_source_restricted(
            publisher,
            context,
            applied,
            capabilities,
            Some(activity),
            work,
        )
    }
    fn bind_private_provider_observation_source_restricted(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
        activity: Option<Arc<dyn SessionObservationActivity>>,
        work: Option<crate::TaskObservationBinding>,
    ) -> Result<SessionObservationSource> {
        let memory = Memory::Private {
            context: context.clone(),
            applied,
        };
        let facts = {
            let c = self.connection()?;
            memory.facts(self, &c)?
        };
        let participants = facts
            .audience
            .into_iter()
            .map(|(pid, _)| pid)
            .collect::<Vec<_>>();
        self.bind_observation_source_inner(
            publisher,
            &facts.connection,
            "vapi",
            context.delivery_id(),
            &participants,
            capabilities,
            Some(memory),
            activity,
            work,
        )
    }
    pub fn publish_private_provider_observation(
        &self,
        source: &SessionObservationSource,
        read: &PrivateProviderMediaRead,
        event: &str,
        data: ObservationDraft,
    ) -> Result<String> {
        let route = self.observation_route(source)?;
        let Some(Memory::Private { context, applied }) = &route.memory else {
            return Err(Error::forbidden());
        };
        let interval = read.observation_interval(context, applied)?;
        self.publish_observation_inner(source, event, data, Some(interval))
    }
    pub(super) fn observation_route(&self, source: &SessionObservationSource) -> Result<Route> {
        if source.runtime != self.link_epoch {
            return Err(Error::forbidden());
        }
        self.events
            .observations
            .lock()
            .map_err(|_| Error::storage())?
            .routes
            .get(&source.key)
            .cloned()
            .ok_or_else(Error::forbidden)
    }
    pub(super) fn observation_publication_authority(
        &self,
        source: &SessionObservationSource,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Arc<AuthorityGuardSet>> {
        let route = self.observation_route(source)?;
        let needed = {
            let c = self.connection()?;
            scopes(self, &c, &route)?
        };
        let guard = if let Some(held) = held {
            if !self.authority_covers(held, &needed) {
                return Err(Error::forbidden());
            }
            held.clone()
        } else {
            // Publication is nonblocking when a caller already owns a media
            // reader. A queued authority writer must not wait on that caller.
            self.try_acquire_authority(&needed)?
        };
        Ok(guard)
    }
    pub(super) fn observation_page_scopes(
        &self,
        c: &Connection,
        credential: &str,
        cid: &str,
        sid: &str,
    ) -> Result<Vec<AuthorityScope>> {
        let actor = actor_from_connection(c, credential)?;
        let epoch = authorize(c, &actor, cid, "session.observation")?;
        actor.require("session_get")?;
        let mut selected =
            crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
        selected.push(AuthorityScope::identity(
            &actor.identity_id,
            AuthorityMode::Read,
        )?);
        let routes = self
            .events
            .observations
            .lock()
            .map_err(|_| Error::storage())?
            .routes
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for route in routes {
            if reader(c, &actor, &route, cid, sid, epoch)? && route_current(c, &route, self)? {
                selected.extend(scopes(self, c, &route)?);
            }
        }
        AuthorityScope::merged(&selected)
    }
}
