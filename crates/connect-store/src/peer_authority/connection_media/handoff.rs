//! Current purpose-bound handoff routes; no native effect or role commit.
use super::*;
use crate::{NativeHandoffContext, NativeHandoffOrigin};
use connect_protocol::{provisioning::IdentityKind, transitions::OutgoingDisposition};
use std::collections::{BTreeMap, BTreeSet};

/// Immutable physical sources. This value holds no idle authority reader.
#[derive(Clone)]
pub struct NativeHandoffSelection {
    pub(crate) origin: NativeHandoffOrigin,
    sources: Vec<Arc<NativeMediaOrigin>>,
}
/// Owned, short current-authority interval for planning a handoff. It does not
/// prove decoded duplex readiness or authorize a native execution attempt.
pub struct NativeHandoffRead {
    selection: NativeHandoffSelection,
    context: NativeHandoffContext,
    reads: BTreeMap<(String, Option<String>), NativeMediaRead>,
    expires_at: i64,
    caller: PeerSessionProjection,
}
impl NativeHandoffRead {
    /// Metadata under this owned interval, never an independent caller grant.
    pub fn caller(&self) -> &PeerSessionProjection {
        &self.caller
    }
    pub(crate) fn limit(&mut self, deadline: i64) -> Result<()> {
        self.expires_at = self.expires_at.min(deadline);
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn selection(&self) -> &NativeHandoffSelection {
        &self.selection
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires_at && self.reads.values().all(NativeMediaRead::current)
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn context(&self) -> &NativeHandoffContext {
        &self.context
    }
    pub fn media(&self, target: &str, source: Option<&str>) -> Option<&NativeMediaRead> {
        self.reads.get(&(target.into(), source.map(str::to_owned)))
    }
    pub fn into_selection(self) -> Result<NativeHandoffSelection> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(self.selection)
    }
}
impl NativeHandoffSelection {
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        let mut sources = self.sources.iter().collect::<Vec<_>>();
        sources.sort_by_key(|s| s.connection_id());
        serde_json::json!(sources
            .into_iter()
            .map(|s| returns::describe(s))
            .collect::<Vec<_>>())
    }
    pub(crate) fn scopes(
        &self,
        store: &Store,
        c: &Connection,
        write: bool,
    ) -> Result<Vec<AuthorityScope>> {
        let (_, _, mut scopes, _) = resolve(store, c, self)?;
        if write {
            let actor = self.origin.require(store, c)?;
            scopes.push(AuthorityScope::identity(
                &actor.identity_id,
                AuthorityMode::Write,
            )?);
            scopes.extend(crate::authority_admission::conversation_scopes(
                c,
                &self.origin.request().conversation_id,
                AuthorityMode::Write,
            )?);
        }
        AuthorityScope::merged(&scopes)
    }
    pub(crate) fn validate_plan(
        &self,
        store: &Store,
        c: &Connection,
        plan: &serde_json::Value,
    ) -> Result<()> {
        let (ctx, views, _, _) = resolve(store, c, self)?;
        let h = &plan["handoff"];
        let keys = [
            "operation_id",
            "audience_revision",
            "private_connections",
            "receivers",
            "activate_muted",
            "expected_receiver_revisions",
            "handoff",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if plan
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect::<BTreeSet<_>>())
            != Some(keys)
            || plan["operation_id"] != self.origin.record_id()
            || !plan["audience_revision"]
                .as_u64()
                .is_some_and(|r| r < 9_007_199_254_740_990)
            || h.as_object().map(|o| o.len()) != Some(6)
            || !h["participant_revision"]
                .as_u64()
                .is_some_and(|r| r < 9_007_199_254_740_990)
        {
            return Err(Error::forbidden());
        }
        let source = self
            .sources
            .iter()
            .find(|s| Some(s.connection_id()) == ctx.source_connection_id.as_deref())
            .ok_or_else(Error::forbidden)?;
        let incoming = self
            .sources
            .iter()
            .find(|s| Some(s.connection_id()) == ctx.target_connection_id.as_deref())
            .ok_or_else(Error::forbidden)?;
        let remote = self
            .sources
            .iter()
            .find(|s| s.connection_id() == ctx.retained_connection_id)
            .ok_or_else(Error::forbidden)?;
        let prepared_target = views[&(incoming.connection_id().into(), None)]
            .0
            .handoff_preparation;
        if if prepared_target {
            plan["activate_muted"] != incoming.runtime_connection_id()
        } else {
            !plan["activate_muted"].is_null()
        } {
            return Err(Error::forbidden());
        }
        let source_role = &views[&(source.connection_id().into(), None)]
            .0
            .presence_role;
        if h["outgoing"] != source.runtime_connection_id()
            || h["incoming"] != incoming.runtime_connection_id()
            || h["retained"] != remote.runtime_connection_id()
            || h["incoming_role"] != *source_role
            || source_role == "observer"
            || h["disposition"]
                != serde_json::to_value(ctx.outgoing).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        let mut group = BTreeSet::from([
            incoming.runtime_connection_id(),
            remote.runtime_connection_id(),
        ]);
        let outgoing = self
            .sources
            .iter()
            .filter(|s| s.participant_id() == ctx.source_participant)
            .collect::<Vec<_>>();
        if ctx.outgoing == OutgoingDisposition::Observe {
            group.extend(outgoing.iter().map(|s| s.runtime_connection_id()));
        }
        let declared = plan["private_connections"]
            .as_array()
            .ok_or_else(Error::forbidden)?;
        if declared.len() != group.len()
            || declared
                .iter()
                .map(|v| v.as_str().ok_or_else(Error::forbidden))
                .collect::<Result<BTreeSet<_>>>()?
                != group
        {
            return Err(Error::forbidden());
        }
        let mut required_receivers =
            BTreeSet::from([incoming.connection_id(), remote.connection_id()]);
        required_receivers.extend(outgoing.iter().map(|s| s.connection_id()));
        let previous = handoff_lifetime::previous_members(
            store,
            c,
            &ctx.session_id,
            source.runtime_connection_id(),
        )?;
        for s in &self.sources {
            let quarantine:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_legs WHERE connection_id=?1 AND operation_record_id=?2)",params![s.connection_id(),ctx.consultation_record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if quarantine || previous.contains(s.runtime_connection_id()) {
                required_receivers.insert(s.connection_id());
            }
        }
        let receivers = plan["receivers"].as_object().ok_or_else(Error::forbidden)?;
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .ok_or_else(Error::forbidden)?;
        if receivers.len() != required_receivers.len() || revisions.len() != self.sources.len() {
            return Err(Error::forbidden());
        }
        for receiver in &self.sources {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![receiver.connection_id(),receiver.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if revision >= 9_007_199_254_740_990
                || revisions
                    .get(receiver.runtime_connection_id())
                    .and_then(|v| v.as_u64())
                    != Some(revision)
            {
                return Err(Error::forbidden());
            }
            if required_receivers.contains(receiver.connection_id()) {
                let actual = receivers
                    .get(receiver.runtime_connection_id())
                    .and_then(|v| v.as_array())
                    .ok_or_else(Error::forbidden)?;
                let expected = self
                    .sources
                    .iter()
                    .filter(|s| {
                        views.contains_key(&(
                            receiver.connection_id().into(),
                            Some(s.connection_id().into()),
                        ))
                    })
                    .map(|s| s.runtime_connection_id())
                    .collect::<BTreeSet<_>>();
                if actual.len() != expected.len()
                    || actual
                        .iter()
                        .map(|v| v.as_str().ok_or_else(Error::forbidden))
                        .collect::<Result<BTreeSet<_>>>()?
                        != expected
                {
                    return Err(Error::forbidden());
                }
            }
        }
        Ok(())
    }
    pub fn read(&self, store: &Store) -> Result<NativeHandoffRead> {
        store.try_native_handoff_read(&self.origin, &self.sources)
    }
}
type Views = BTreeMap<
    (String, Option<String>),
    (
        PeerConnectionProjection,
        Option<PeerConnectionProjection>,
        bool,
    ),
>;
fn resolve(
    store: &Store,
    c: &Connection,
    selection: &NativeHandoffSelection,
) -> Result<(NativeHandoffContext, Views, Vec<AuthorityScope>, i64)> {
    let actor = selection.origin.require(store, c)?;
    let mut context = selection.origin.context(store, c)?;
    let mut scopes = vec![AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?];
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &context.conversation_id,
        AuthorityMode::Read,
    )?);
    let mut expires_at = (crate::now() + 5)
        .min(actor.expires_at)
        .min(selection.origin.request().expires_at);
    let actual = selection
        .sources
        .iter()
        .map(|s| s.connection_id().to_owned())
        .collect::<BTreeSet<_>>();
    if actual.len() != selection.sources.len() {
        return Err(Error::forbidden());
    }
    let mut statement = c.prepare("SELECT v.id FROM connections v JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 WHERE v.session_id=?1 AND v.observed_runtime_id=?2 AND v.state IN ('ready','active')").map_err(|_|Error::storage())?;
    let expected = statement
        .query_map(params![context.session_id, store.runtime_epoch()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(|_| Error::storage())?;
    if actual != expected {
        return Err(Error::forbidden());
    }
    let select = |pid: &str, explicit: &Option<String>| -> Result<String> {
        let candidates = selection
            .sources
            .iter()
            .filter(|s| {
                s.participant_id() == pid
                    && explicit.as_ref().is_none_or(|id| id == s.connection_id())
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(Error::forbidden());
        }
        Ok(candidates[0].connection_id().into())
    };
    let outgoing = select(&context.source_participant, &context.source_connection_id)?;
    let incoming = select(&context.target_participant, &context.target_connection_id)?;
    let retained = context.retained_connection_id.clone();
    if outgoing == incoming || outgoing == retained || incoming == retained {
        return Err(Error::forbidden());
    }
    context.source_connection_id = Some(outgoing);
    context.target_connection_id = Some(incoming.clone());
    let mut snapshots = BTreeMap::new();
    let mut actors = BTreeMap::new();
    let mut subscriptions = BTreeMap::new();
    let mut quarantined = BTreeSet::new();
    for source in &selection.sources {
        if source.session_id() != context.session_id
            || source.conversation_id() != context.conversation_id
        {
            return Err(Error::forbidden());
        }
        if source.participant_id() == context.source_participant
            || source.connection_id() == incoming
            || source.connection_id() == retained
        {
            // A handoff may replace its own known private audience, never an
            // unrelated active/uncertain consultation or held remote route.
            let private: Option<String> = c.query_row("SELECT operation_record_id FROM native_private_content_floors WHERE connection_id=?1",[source.connection_id()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            if private
                .as_ref()
                .is_some_and(|record| Some(record) != context.consultation_record_id.as_ref())
            {
                return Err(Error::forbidden());
            }
            let held_elsewhere: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id AND e.state IN ('prepared','submitting','confirmed','unknown') LEFT JOIN native_private_lifetimes l ON l.operation_record_id=n.operation_record_id WHERE json_extract(n.selection_json,'$.remote.connection_id')=?1 AND n.operation_record_id IS NOT ?2 AND l.restored_at IS NULL AND NOT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects re ON re.id=r.effect_id AND re.state='confirmed' WHERE r.consultation_record_id=n.operation_record_id AND r.applied_json IS NOT NULL))",params![source.connection_id(),context.consultation_record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if held_elsewhere {
                return Err(Error::forbidden());
            }
        }
        let physical = crate::identity::actor_from_connection(
            c,
            source
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )?;
        let candidate_context: Option<String> = c
            .query_row(
                "SELECT operation_record_id FROM native_consultation_legs WHERE connection_id=?1",
                [source.connection_id()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let member =
            crate::audience::member(c, &physical.identity_id, source.conversation_id(), true)?;
        if source.participant_id() == context.source_participant
            && context.outgoing == OutgoingDisposition::Observe
        {
            physical.require("session.observation")?;
        }
        if source.connection_id() == incoming {
            // The exact incoming signed offer identifies a current explicit
            // target policy. Neither a copied reference nor the caller's grant
            // can replace the target's independently current named grant.
            if context.policy_ref != source.offer_record_id() {
                return Err(Error::forbidden());
            }
            physical.require("session.handoff")?;
            // Private model memory must never become a public speaker. The
            // caller must select a fresh safe public AI instance instead.
            if member.kind == IdentityKind::Ai
                && crate::connection_control::private_consultation::private_partner(
                    c,
                    source.connection_id(),
                )?
                .is_some()
            {
                return Err(Error::new(
                    "private-context-required",
                    "Handoff requires a fresh public AI instance",
                ));
            }
        }
        // An unselected old private candidate is closed to all handoff media.
        // Its provider may already be unavailable; use current physical closure
        // facts rather than borrowing the private provider's readiness/grant.
        let own_candidate = candidate_context
            .as_ref()
            .is_some_and(|record| Some(record) == context.consultation_record_id.as_ref());
        let isolate = own_candidate
            && source.connection_id() != incoming
            && (source.participant_id() != context.source_participant
                || context.outgoing == OutgoingDisposition::Leave)
            && source.connection_id() != retained;
        let (view, required, expiry) = if isolate {
            quarantined.insert(source.connection_id().to_owned());
            returns::closure_facts(store, c, source)?
        } else {
            facts(store, c, source)?
        };
        if !actor.permits_recipient(&view.session.actor.identity_id) {
            return Err(Error::forbidden());
        }
        subscriptions.insert(
            source.connection_id().to_owned(),
            !isolate && subscription_permission(c, source)?,
        );
        actors.insert(source.connection_id().to_owned(), physical);
        snapshots.insert(source.connection_id().to_owned(), view);
        scopes.extend(required);
        expires_at = expires_at.min(expiry);
    }
    let remote = snapshots.get(&retained).ok_or_else(Error::forbidden)?;
    if remote.connection.participant_id == context.source_participant
        || remote.connection.participant_id == context.target_participant
    {
        return Err(Error::forbidden());
    }
    let mut views = BTreeMap::new();
    for target in &selection.sources {
        let tid = target.connection_id();
        let target_view = &snapshots[tid];
        let outgoing_target = target.participant_id() == context.source_participant;
        let affected_target = outgoing_target || tid == incoming || tid == retained;
        views.insert(
            (tid.into(), None),
            (target_view.clone(), None, subscriptions[tid]),
        );
        for source in &selection.sources {
            let sid = source.connection_id();
            if tid == sid || quarantined.contains(tid) || quarantined.contains(sid) {
                continue;
            }
            let outgoing_source = source.participant_id() == context.source_participant;
            let affected_source = outgoing_source || sid == incoming || sid == retained;
            let intended = (tid == incoming && sid == retained)
                || (tid == retained && sid == incoming)
                || (outgoing_target
                    && context.outgoing == OutgoingDisposition::Observe
                    && (sid == incoming || sid == retained));
            let allowed = if affected_target || affected_source {
                intended
            } else {
                // Unrelated independently prepared peers remain closed even
                // inside the complete scene read for another handoff.
                let mut allowed =
                    !target_view.handoff_preparation && !snapshots[sid].handoff_preparation;
                for (private, other) in [(target, source), (source, target)] {
                    if crate::connection_control::private_consultation::private_partner(
                        c,
                        private.connection_id(),
                    )?
                    .is_some_and(|partner| partner != other.connection_id())
                    {
                        allowed = false;
                    }
                }
                allowed
            };
            if !allowed {
                continue;
            }
            let publisher = &snapshots[sid];
            let permitted = subscriptions[tid]
                && actors[tid].permits_recipient(&publisher.session.actor.identity_id)
                && actors[sid].permits_recipient(&target_view.session.actor.identity_id);
            if intended && !permitted {
                return Err(Error::forbidden());
            }
            if permitted {
                views.insert(
                    (tid.into(), Some(sid.into())),
                    (target_view.clone(), Some(publisher.clone()), true),
                );
            }
        }
    }
    Ok((context, views, AuthorityScope::merged(&scopes)?, expires_at))
}
impl Store {
    pub fn try_native_handoff_read(
        &self,
        origin: &NativeHandoffOrigin,
        sources: &[Arc<NativeMediaOrigin>],
    ) -> Result<NativeHandoffRead> {
        let selection = NativeHandoffSelection {
            origin: origin.clone(),
            sources: sources.to_vec(),
        };
        let scopes = {
            let c = self.try_connection()?;
            resolve(self, &c, &selection)?.2
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (context, views, current, expires_at) = resolve(self, &c, &selection)?;
        if !self.authority_covers(&held, &current) || crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let actor = selection.origin.require(self, &c)?;
        let member =
            crate::audience::member(&c, &actor.identity_id, &context.conversation_id, true)?;
        let mut caller = views
            .values()
            .next()
            .ok_or_else(Error::forbidden)?
            .0
            .session
            .clone();
        caller.present=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![context.session_id,member.participant_id,member.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        caller.actor = member;
        let reads = views
            .into_iter()
            .map(|(key, (target, source, may_subscribe))| {
                (
                    key,
                    NativeMediaRead {
                        target,
                        source,
                        _scopes: held.clone(),
                        owner: owner.clone(),
                        expires_at,
                        may_subscribe,
                    },
                )
            })
            .collect();
        Ok(NativeHandoffRead {
            selection,
            context,
            reads,
            expires_at,
            caller,
        })
    }
}
