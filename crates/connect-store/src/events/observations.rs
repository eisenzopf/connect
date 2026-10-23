//! Ephemeral route observations. Only a trusted adapter can obtain a source
//! handle; neither request bodies nor provider/model payloads choose its scope.
use super::*;
use std::collections::VecDeque;
pub(crate) mod memory;
mod work;
use crate::AuthorityGuardSet;
use std::sync::Arc;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROUTES: usize = 128;
const TTL: i64 = 30;
/// Protected host restriction checked on publication and buffered reads. It
/// must not access Store/SQLite, wait for authority or perform IO: checks run
/// inside the current snapshot. Busy checks return authority-busy without
/// advancing a cursor; permanent closure returns forbidden. Never a grant.
pub trait SessionObservationActivity: Send + Sync {
    fn check(&self) -> Result<()>;
}
#[derive(Clone)]
pub struct SessionObservationSource {
    key: String,
    runtime: String,
}
impl std::fmt::Debug for SessionObservationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionObservationSource(redacted)")
    }
}
#[derive(Clone)]
struct Route {
    credential: String,
    actor: String,
    cid: String,
    sid: String,
    pid: String,
    epoch: u64,
    connection: String,
    generation: u64,
    provider: String,
    correlation: String,
    audience: Vec<(String, u64)>,
    capabilities: ObservationCapabilities,
    memory: Option<memory::Memory>,
    activity: Option<Arc<dyn SessionObservationActivity>>,
    work: Option<crate::TaskObservationBinding>,
}
struct Stored {
    sequence: i64,
    source: String,
    event: String,
    digest: String,
    value: SessionObservation,
    bytes: usize,
}
#[derive(Default)]
pub(super) struct Buffer {
    routes: BTreeMap<String, Route>,
    entries: VecDeque<Stored>,
    sequence: i64,
    floor: i64,
    source_floors: BTreeMap<String, i64>,
    bytes: usize,
}
impl Buffer {
    #[cfg(test)]
    pub(super) fn occupancy(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
    fn expire(&mut self) {
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.value.observed_at + TTL <= crate::now())
        {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
                self.floor = self.floor.max(entry.sequence);
                if self.routes.contains_key(&entry.source) {
                    self.source_floors
                        .entry(entry.source.clone())
                        .and_modify(|floor| *floor = (*floor).max(entry.sequence))
                        .or_insert(entry.sequence);
                }
            }
        }
    }
    fn bound(&mut self) {
        while self.entries.len() > EVENT_QUEUE_LIMIT || self.bytes > MAX_BYTES {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
                self.floor = self.floor.max(entry.sequence);
                if self.routes.contains_key(&entry.source) {
                    self.source_floors
                        .entry(entry.source.clone())
                        .and_modify(|floor| *floor = (*floor).max(entry.sequence))
                        .or_insert(entry.sequence);
                }
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    purpose: String,
    identity: String,
    cid: String,
    sid: String,
    epoch: u64,
    runtime: String,
    sequence: i64,
    expires_at: i64,
}
fn route_current(c: &Connection, route: &Route, store: &Store) -> Result<bool> {
    if !memory::current(store, c, route)? || !work::current(store, c, route)? {
        return Ok(false);
    }
    if let Some(activity) = &route.activity {
        match activity.check() {
            Ok(()) => (),
            Err(error) if error.code == "forbidden" => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    if !crate::connection_control::private_consultation::observation_source_current(
        store,
        c,
        &route.connection,
    )? {
        return Ok(false);
    }
    if !crate::connection_control::private_consultation::observation_allowed(
        c,
        &route.connection,
        &route.audience,
    )? {
        return Ok(false);
    }
    let Some(actor) = c
        .query_row(
            "SELECT id FROM credentials WHERE id=?1 AND revoked=0 AND expires_at>?2",
            params![route.credential, crate::now()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
    else {
        return Ok(false);
    };
    let actor = match actor_from_connection(c, &actor) {
        Ok(a) => a,
        Err(e) if matches!(e.code.as_str(), "unauthorized" | "forbidden") => return Ok(false),
        Err(e) => return Err(e),
    };
    if actor.identity_id != route.actor || !actor.permits("session.observation.publish") {
        return Ok(false);
    }
    match audience::require_original(
        c,
        &actor,
        &route.cid,
        connect_protocol::membership::ContentKind::Session,
        &route.sid,
    ) {
        Ok(()) => (),
        Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => return Ok(false),
        Err(e) => return Err(e),
    }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 JOIN endpoints e ON e.id=v.endpoint_id AND e.verification_state='verified' JOIN devices d ON d.id=e.device_id AND d.revoked=0 JOIN sessions s ON s.id=v.session_id JOIN conversations room ON room.id=s.conversation_id AND room.closed=0 WHERE v.id=?1 AND v.session_id=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND v.generation=?5 AND v.observed_runtime_id=?6 AND v.state IN ('ready','active','held') AND s.state IN ('active','accepted','ready'))",params![route.connection,route.sid,route.pid,route.epoch,route.generation,store.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !valid {
        return Ok(false);
    }
    for (pid, epoch) in &route.audience {
        let identity:Option<String>=c.query_row("SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE p.participant_id=?1 AND p.conversation_id=?2 AND p.membership_epoch=?3 AND p.active=1",params![pid,route.cid,epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        // Removed recipients stop receiving; they do not prevent safe delivery to
        // remaining originals. A narrowed publisher ceiling does stop publishing.
        if let Some(identity) = identity {
            if !actor.permits_recipient(&identity) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn reader(
    c: &Connection,
    actor: &Actor,
    route: &Route,
    cid: &str,
    sid: &str,
    epoch: u64,
) -> Result<bool> {
    if route.cid != cid || route.sid != sid {
        return Ok(false);
    }
    let own = audience::member(c, &actor.identity_id, cid, true)?;
    if !route
        .audience
        .iter()
        .any(|(pid, e)| pid == &own.participant_id && *e == epoch)
    {
        return Ok(false);
    }
    let identity:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2 AND membership_epoch=?3 AND active=1",params![route.pid,cid,route.epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(identity.is_some_and(|identity| actor.permits_recipient(&identity)))
}
fn validate(cap: &ObservationCapabilities, data: &ObservationDraft) -> Result<()> {
    let ids: Vec<&str> = match data {
        ObservationDraft::Agent { observation } => {
            observation.validate()?;
            if !observation.content.supported_by(cap) {
                return Err(Error::new(
                    "unsupported-capability",
                    "Agent observation is not declared by this provider source",
                ));
            }
            vec![]
        }
        ObservationDraft::Transcript {
            text,
            turn_id,
            fragment_id,
            ..
        } => {
            if !cap.transcript {
                return Err(Error::new(
                    "unsupported-capability",
                    "Provider source does not report transcripts",
                ));
            }
            if text.is_empty() {
                return Err(invalid());
            }
            turn_id
                .iter()
                .chain(fragment_id)
                .map(String::as_str)
                .collect()
        }
        ObservationDraft::Speech { turn_id, .. } => {
            if !cap.speech {
                return Err(Error::new(
                    "unsupported-capability",
                    "Provider source does not report speech activity",
                ));
            }
            turn_id.iter().map(String::as_str).collect()
        }
        ObservationDraft::ActionRequest {
            action,
            provider_tool_call_id,
            arguments,
            turn_id,
        } => {
            if !cap.actions.contains(action) || !arguments.is_object() {
                return Err(Error::new(
                    "unsupported-capability",
                    "Action is not declared by this provider source",
                ));
            }
            let mut ids = vec![action.as_str(), provider_tool_call_id.as_str()];
            ids.extend(turn_id.iter().map(String::as_str));
            ids
        }
    };
    for id in ids {
        connect_protocol::transitions::id(id)?;
    }
    if serde_json::to_vec(data)
        .map_err(|_| Error::storage())?
        .len()
        > if matches!(data, ObservationDraft::Agent { .. }) {
            MAX_AGENT_DRAFT_BYTES
        } else {
            16 * 1024
        }
    {
        return Err(Error::new(
            "capacity-exceeded",
            "Observation exceeds encoded byte budget",
        ));
    }
    Ok(())
}
impl Store {
    /// Backend-only registration after an adapter has verified its actual media
    /// route. Audience comes from the admitted route policy, never provider text.
    pub fn bind_session_observation_source(
        &self,
        credential: &str,
        connection: &str,
        provider: &str,
        correlation: &str,
        participants: &[String],
        capabilities: ObservationCapabilities,
    ) -> Result<SessionObservationSource> {
        self.bind_observation_source_inner(
            credential,
            connection,
            provider,
            correlation,
            participants,
            capabilities,
            None,
            None,
            None,
        )
    }
    fn bind_observation_source_inner(
        &self,
        credential: &str,
        connection: &str,
        provider: &str,
        correlation: &str,
        participants: &[String],
        capabilities: ObservationCapabilities,
        memory: Option<memory::Memory>,
        activity: Option<Arc<dyn SessionObservationActivity>>,
        work: Option<crate::TaskObservationBinding>,
    ) -> Result<SessionObservationSource> {
        for id in [connection, provider, correlation] {
            connect_protocol::transitions::id(id)?;
        }
        if participants.is_empty()
            || participants.len() > 100
            || capabilities.actions.len() > 16
            || serde_json::to_vec(&capabilities)
                .map_err(|_| Error::storage())?
                .len()
                > 2048
        {
            return Err(invalid());
        }
        for action in &capabilities.actions {
            connect_protocol::transitions::id(action)?;
        }
        let mut actions = capabilities.actions.clone();
        actions.sort();
        actions.dedup();
        if actions.len() != capabilities.actions.len() {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("session.observation.publish")?;
        actor.require("session_get")?;
        let (cid,sid,pid,epoch,generation,owner):(String,String,String,u64,u64,String)=tx.query_row("SELECT s.conversation_id,v.session_id,v.participant_id,v.membership_epoch,v.generation,v.identity_id FROM connections v JOIN sessions s ON s.id=v.session_id WHERE v.id=?1",[connection],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        if owner != actor.identity_id {
            actor.require("session.observation.publish.any")?;
        }
        if !actor.permits_recipient(&owner) {
            return Err(Error::forbidden());
        }
        let mut selected = vec![];
        for participant in participants {
            connect_protocol::transitions::id(participant)?;
            let identity:Option<String>=tx.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2 AND active=1",params![participant,cid],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            let identity = identity.ok_or_else(Error::forbidden)?;
            let member = audience::member(&tx, &identity, &cid, true)?;
            if !actor.permits_recipient(&identity)
                || !audience::original_to_identity(
                    &tx,
                    &identity,
                    &cid,
                    connect_protocol::membership::ContentKind::Session,
                    &sid,
                )?
            {
                return Err(Error::forbidden());
            }
            selected.push((participant.clone(), member.membership_epoch));
        }
        selected.sort();
        if !crate::connection_control::private_consultation::observation_allowed(
            &tx, connection, &selected,
        )? {
            return Err(Error::forbidden());
        }
        if selected.windows(2).any(|v| v[0] == v[1]) || !selected.contains(&(pid.clone(), epoch)) {
            return Err(invalid());
        }
        let route = Route {
            credential: credential.into(),
            actor: actor.identity_id.clone(),
            cid,
            sid,
            pid,
            epoch,
            connection: connection.into(),
            generation,
            provider: provider.into(),
            correlation: correlation.into(),
            audience: selected,
            capabilities,
            memory,
            activity,
            work,
        };
        if !route_current(&tx, &route, self)? {
            return Err(Error::forbidden());
        }
        work::contract(self, &tx, &route)?;
        let mut buffer = self
            .events
            .observations
            .lock()
            .map_err(|_| Error::storage())?;
        buffer.expire();
        let mut stale = vec![];
        for (id, old) in &buffer.routes {
            if !route_current(&tx, old, self)? {
                stale.push(id.clone());
            }
        }
        for id in stale {
            buffer.routes.remove(&id);
            buffer.source_floors.remove(&id);
        }
        for (key, old) in &buffer.routes {
            if old.connection == route.connection
                && old.generation == route.generation
                && old.provider == route.provider
                && old.correlation == route.correlation
            {
                if old.work.as_ref().map(|w| w.descriptor())
                    != route.work.as_ref().map(|w| w.descriptor())
                    || old.audience != route.audience
                    || old.capabilities != route.capabilities
                    || old.credential != route.credential
                    || old.memory.as_ref().map(|m| m.identity())
                        != route.memory.as_ref().map(|m| m.identity())
                    || !match (&old.activity, &route.activity) {
                        (None, None) => true,
                        (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                        _ => false,
                    }
                {
                    return Err(Error::new(
                        "conflict",
                        "Observation route policy changed under its source correlation",
                    ));
                }
                return Ok(SessionObservationSource {
                    key: key.clone(),
                    runtime: self.link_epoch.clone(),
                });
            }
        }
        if buffer.routes.len() >= MAX_ROUTES {
            return Err(Error::new(
                "capacity-exceeded",
                "Observation source capacity exceeded",
            ));
        }
        let key = uuid::Uuid::new_v4().to_string();
        current(&tx, &actor)?;
        tx.commit().map_err(|_| Error::storage())?;
        // A refused/failed registration must not leave a retained activity
        // restriction behind under an otherwise retryable correlation.
        buffer.routes.insert(key.clone(), route);
        Ok(SessionObservationSource {
            key,
            runtime: self.link_epoch.clone(),
        })
    }
    /// No scheduling, authorization grant or action execution occurs here.
    pub fn publish_session_observation(
        &self,
        source: &SessionObservationSource,
        event: &str,
        data: ObservationDraft,
    ) -> Result<String> {
        self.publish_observation_inner(source, event, data, None)
    }
    fn publish_observation_inner(
        &self,
        source: &SessionObservationSource,
        event: &str,
        data: ObservationDraft,
        interval: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<String> {
        connect_protocol::transitions::id(event)?;
        let held = self.observation_publication_authority(source, interval)?;
        if source.runtime != self.link_epoch {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let mut buffer = self
            .events
            .observations
            .lock()
            .map_err(|_| Error::storage())?;
        buffer.expire();
        let route = buffer
            .routes
            .get(&source.key)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        if !self.authority_covers(&held, &memory::scopes(self, &tx, &route)?)
            || !route_current(&tx, &route, self)?
        {
            return Err(Error::forbidden());
        }
        validate(&route.capabilities, &data)?;
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&data).map_err(|_| Error::storage())?,
        ));
        if let Some(old) = buffer
            .entries
            .iter()
            .find(|entry| entry.source == source.key && entry.event == event)
        {
            if old.digest != digest {
                return Err(Error::new(
                    "conflict",
                    "Provider event ID was reused with changed observation",
                ));
            }
            return Ok(old.value.observation_id.clone());
        }
        if buffer.sequence == i64::MAX {
            return Err(Error::new(
                "capacity-exceeded",
                "Observation cursor exhausted",
            ));
        }
        if !route_current(&tx, &route, self)? {
            return Err(Error::forbidden());
        }
        let sequence = buffer.sequence + 1;
        let id = format!("obs_{}", uuid::Uuid::new_v4());
        let value = SessionObservation {
            work: route.work.as_ref().map(|w| w.descriptor().clone()),
            observation_id: id.clone(),
            conversation_id: route.cid,
            session_id: route.sid,
            participant_id: route.pid,
            membership_epoch: route.epoch,
            connection_id: route.connection,
            generation: route.generation,
            source: EventSource {
                kind: route.provider,
                correlation_id: route.correlation,
            },
            observed_at: crate::now(),
            data,
        };
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| Error::storage())?
            .len();
        // Publish no buffer/cursor mutation if the validating snapshot fails.
        tx.commit().map_err(|_| Error::storage())?;
        buffer.sequence = sequence;
        buffer.bytes += bytes;
        buffer.entries.push_back(Stored {
            sequence,
            source: source.key.clone(),
            event: event.into(),
            digest,
            value,
            bytes,
        });
        buffer.bound();
        Ok(id)
    }
    pub fn session_observations(
        &self,
        credential: &str,
        cid: &str,
        sid: &str,
        query: ObservationQuery,
    ) -> Result<ObservationPage> {
        self.session_observations_held(credential, cid, sid, query, None)
    }
    pub(crate) fn session_observations_held(
        &self,
        credential: &str,
        cid: &str,
        sid: &str,
        query: ObservationQuery,
        interval: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<ObservationPage> {
        connect_protocol::transitions::id(sid)?;
        if query.wait_ms > 30_000 {
            return Err(invalid());
        }
        let held = self.stable_authority_with(
            interval,
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                self.observation_page_scopes(&c, credential, cid, sid)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(
            &held,
            &self.observation_page_scopes(&tx, credential, cid, sid)?,
        ) {
            return Err(crate::events::admission::changed());
        }
        let actor = actor_from_connection(&tx, credential)?;
        let epoch = authorize(&tx, &actor, cid, "session.observation")?;
        actor.require("session_get")?;
        audience::require_original(
            &tx,
            &actor,
            cid,
            connect_protocol::membership::ContentKind::Session,
            sid,
        )?;
        let size = limit(self, query.limit)?.min(EVENT_QUEUE_LIMIT);
        let mut buffer = self
            .events
            .observations
            .lock()
            .map_err(|_| Error::storage())?;
        buffer.expire();
        let mut gap = false;
        let mut after = if let Some(value) = query.after {
            let token: Token = cursor::open(&tx, &value)?;
            if token.purpose != "observations"
                || token.identity != actor.identity_id
                || token.cid != cid
                || token.sid != sid
                || token.epoch != epoch
            {
                return Err(Error::forbidden());
            }
            if token.sequence < 0 {
                return Err(invalid());
            }
            gap = token.runtime != self.link_epoch
                || token.expires_at <= crate::now()
                || token.sequence > buffer.sequence;
            if gap {
                buffer.floor
            } else {
                token.sequence
            }
        } else {
            buffer.floor
        };
        let mut routes = vec![];
        let mut allowed = std::collections::HashSet::new();
        for (key, route) in &buffer.routes {
            if reader(&tx, &actor, route, cid, sid, epoch)? && route_current(&tx, route, self)? {
                allowed.insert(key.clone());
                routes.push(ObservationRoute {
                    work: route.work.as_ref().map(|w| w.descriptor().clone()),
                    correlation_id: route.correlation.clone(),
                    participant_id: route.pid.clone(),
                    connection_id: route.connection.clone(),
                    generation: route.generation,
                    source: route.provider.clone(),
                    capabilities: route.capabilities.clone(),
                });
            }
        }
        let visible_floor = allowed
            .iter()
            .filter_map(|source| buffer.source_floors.get(source))
            .copied()
            .max()
            .unwrap_or(0);
        if after < visible_floor {
            gap = true;
            after = buffer.floor;
        }
        let mut bytes = serde_json::to_vec(&routes)
            .map_err(|_| Error::storage())?
            .len();
        let mut observations = vec![];
        let mut through = after;
        let mut more = false;
        for entry in buffer.entries.iter().filter(|entry| entry.sequence > after) {
            if !allowed.contains(&entry.source) {
                through = entry.sequence;
                continue;
            }
            if observations.len() == size
                || bytes + entry.bytes > connect_protocol::MAX_PAGE_BYTES - 8192
            {
                more = true;
                break;
            }
            bytes += entry.bytes;
            through = entry.sequence;
            observations.push(entry.value.clone());
        }
        if !more {
            through = buffer.sequence;
        }
        let expires_at = actor.expires_at.min(crate::now() + TTL);
        let token = cursor::seal(
            &tx,
            &Token {
                purpose: "observations".into(),
                identity: actor.identity_id.clone(),
                cid: cid.into(),
                sid: sid.into(),
                epoch,
                runtime: self.link_epoch.clone(),
                sequence: through,
                expires_at,
            },
        )?;
        let result = ObservationPage {
            conversation_id: cid.into(),
            session_id: sid.into(),
            observations,
            routes,
            cursor: token,
            expires_at,
            has_more: more,
            resync_required: gap,
        };
        if serde_json::to_vec(&result)
            .map_err(|_| Error::storage())?
            .len()
            > connect_protocol::MAX_PAGE_BYTES
        {
            return Err(Error::new(
                "capacity-exceeded",
                "Observation page exceeds encoded byte budget",
            ));
        }
        current(&tx, &actor)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
