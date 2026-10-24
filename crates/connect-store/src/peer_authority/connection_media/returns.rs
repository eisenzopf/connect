//! Purpose-bound fresh native views for restoring one captured audience.
//! No view here permits private-candidate content to enter a public receiver.
use super::consultation_sip;
use super::*;
use crate::{NativeConsultationReturnOrigin, SipAudienceOrigin, SipAudienceRead};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct NativeReturnSelection {
    pub(crate) origin: NativeConsultationReturnOrigin,
    sources: Vec<Arc<NativeMediaOrigin>>,
    sips: Vec<SipAudienceOrigin>,
    sip_snapshots: Vec<serde_json::Value>,
}
pub struct NativeReturnRead {
    selection: NativeReturnSelection,
    reads: BTreeMap<(String, Option<String>), Arc<NativeMediaRead>>,
    expires_at: i64,
    sip_reads: BTreeMap<String, Arc<SipAudienceRead>>,
    sip_send: BTreeSet<(String, String)>,
    candidate: String,
}
impl NativeReturnRead {
    pub(crate) fn limit(&mut self, deadline: i64) -> Result<()> {
        self.expires_at = self.expires_at.min(deadline);
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn selection(&self) -> &NativeReturnSelection {
        &self.selection
    }
    pub(crate) fn origin(&self) -> &NativeConsultationReturnOrigin {
        &self.selection.origin
    }

    pub fn is_primary_sip(&self, id: &str) -> bool {
        self.selection.origin.request.connection_id.as_deref() == Some(id)
            && self.sip_reads.contains_key(id)
    }
    pub fn is_candidate_sip(&self, id: &str) -> bool {
        self.candidate == id && self.sip_reads.contains_key(id)
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires_at
            && self.reads.values().all(|r| r.current())
            && self.sip_reads.values().all(|r| r.current())
    }
    pub fn sip(&self, id: &str) -> Option<Arc<SipAudienceRead>> {
        self.sip_reads.get(id).cloned()
    }
    pub fn sip_receive(
        &self,
        native: &str,
        sip: &str,
    ) -> Option<(&SipAudienceRead, &NativeMediaRead)> {
        if !self.current() || self.candidate == sip {
            return None;
        }
        let phone = self.sip_reads.get(sip)?;
        let target = self.reads.get(&(native.into(), Some(sip.into())))?;
        Some((phone.as_ref(), target.as_ref()))
    }
    /// Separately authorized SIP-target/native-source return edge. This never
    /// converts the phone into a native projection or reuses receive permission.
    pub fn sip_send(
        &self,
        sip: &str,
        native: &str,
    ) -> Option<(&SipAudienceRead, &NativeMediaRead)> {
        if !self.current() || !self.sip_send.contains(&(sip.into(), native.into())) {
            return None;
        }
        Some((
            self.sip_reads.get(sip)?.as_ref(),
            self.reads.get(&(native.into(), None))?.as_ref(),
        ))
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn media(&self, target: &str, source: Option<&str>) -> Option<&NativeMediaRead> {
        // SIP edges are available only as their typed source/target pair;
        // they must never masquerade as a native source projection.
        if source.is_some_and(|id| self.sip_reads.contains_key(id)) {
            return None;
        }
        self.reads
            .get(&(target.into(), source.map(str::to_owned)))
            .map(Arc::as_ref)
    }
    pub(crate) fn into_selection(self) -> Result<NativeReturnSelection> {
        if !self.current() {
            return Err(Error::new(
                "forbidden",
                "Native return planning read expired",
            ));
        }
        Ok(self.selection.clone())
    }
}
impl NativeReturnSelection {
    pub(crate) fn read(&self, store: &Store) -> Result<NativeReturnRead> {
        store.try_native_return_read(&self.origin, &self.sources, &self.sips)
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        let mut sources = self.sources.iter().collect::<Vec<_>>();
        sources.sort_by_key(|s| s.connection_id());
        let mut selected = sources.into_iter().map(|s| describe(s)).collect::<Vec<_>>();
        selected.extend(self.sip_snapshots.clone());
        serde_json::json!(selected)
    }
    pub(crate) fn scopes(
        &self,
        store: &Store,
        c: &Connection,
        write: bool,
    ) -> Result<Vec<AuthorityScope>> {
        let (_, mut scopes, _) = resolve(store, c, self)?;
        if write {
            scopes.extend(crate::authority_admission::conversation_scopes(
                c,
                &self.origin.request.conversation_id,
                AuthorityMode::Write,
            )?);
        }
        AuthorityScope::merged(&scopes)
    }
    pub(crate) fn validate_plan(&self, c: &Connection, plan: &serde_json::Value) -> Result<()> {
        if plan["operation_id"] != self.origin.record
            || plan["consultation_record_id"] != self.origin.consultation_record_id()
            || plan["restore"] != true
            || !plan["audience_revision"]
                .as_u64()
                .is_some_and(|v| v < 9_007_199_254_740_990)
            || !plan["activate_muted"].is_null()
            || !plan["receivers"].as_object().is_some_and(|v| v.is_empty())
        {
            return Err(Error::forbidden());
        }
        let context:serde_json::Value=c.query_row("SELECT context_selection_json FROM native_consultation_returns WHERE operation_record_id=?1",[&self.origin.record],|r|r.get::<_,String>(0)).map_err(|_|Error::storage()).and_then(|s|serde_json::from_str(&s).map_err(|_|Error::storage()))?;
        let selected_ids = self
            .sources
            .iter()
            .map(|s| s.connection_id())
            .chain(self.sips.iter().map(|s| s.connection_id()))
            .collect::<BTreeSet<_>>();
        let mut departed = BTreeSet::<String>::new();
        for port in context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .chain(std::iter::once(&context["remote"]))
        {
            let id = port["connection_id"].as_str().ok_or_else(Error::storage)?;
            if selected_ids.contains(id) {
                continue;
            }
            let ended = if port.get("source_kind").is_some() {
                Store::recorded_sip_departure(c, port, &self.origin.runtime)?
            } else {
                c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connections v ON v.id=l.connection_id JOIN connection_offers m ON m.connection_id=v.id WHERE l.connection_id=?1 AND l.generation=?2 AND l.runtime_epoch=?3 AND l.retired=1 AND v.state IN ('retired','failed') AND v.participant_id=?4 AND v.membership_epoch=?5 AND m.runtime_connection_id=?6)",params![id,port["generation"].as_u64().ok_or_else(Error::storage)?,self.origin.runtime,port["participant_id"].as_str().ok_or_else(Error::storage)?,port["membership_epoch"].as_u64().ok_or_else(Error::storage)?,port["runtime_connection_id"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?
            };
            if !ended {
                return Err(Error::new(
                    "forbidden",
                    "Native return departure is not recorded",
                ));
            }
            departed.insert(
                port["runtime_connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?
                    .into(),
            );
        }
        let declared = plan["known_departed_connections"]
            .as_array()
            .ok_or_else(Error::forbidden)?;
        let declared_ids = declared
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or_else(Error::forbidden))
            .collect::<Result<BTreeSet<_>>>()?;
        if declared_ids != departed || declared.len() != departed.len() {
            return Err(Error::new(
                "forbidden",
                "Native return departures differ from the original context",
            ));
        }
        let original = context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .map(|s| {
                s["runtime_connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let selected = plan["private_connections"]
            .as_array()
            .ok_or_else(Error::forbidden)?
            .iter()
            .map(|s| s.as_str().ok_or_else(Error::forbidden))
            .collect::<Result<BTreeSet<_>>>()?;
        if original != selected
            || selected.len() != 2
            || plan["private_connections"].as_array().map(Vec::len) != Some(2)
        {
            return Err(Error::forbidden());
        }
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .ok_or_else(Error::forbidden)?;
        if revisions.len() != self.sources.len() + self.sips.len() {
            return Err(Error::new(
                "forbidden",
                "Native return receiver and source sets differ",
            ));
        }
        for source in &self.sources {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![source.connection_id(),source.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if revision >= 9_007_199_254_740_990 {
                return Err(Error::forbidden());
            }
            if revisions
                .get(source.runtime_connection_id())
                .and_then(|v| v.as_u64())
                != Some(revision)
            {
                return Err(Error::new(
                    "forbidden",
                    "Native return stream revision changed",
                ));
            }
        }
        consultation_sip::validate_revisions(c, &self.sips, plan)?;
        Ok(())
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
    selection: &NativeReturnSelection,
) -> Result<(Views, Vec<AuthorityScope>, i64)> {
    let actor = selection.origin.require(store, c)?;
    let mut scopes = vec![
        AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read)?,
        AuthorityScope::conversation(
            &selection.origin.request.conversation_id,
            AuthorityMode::Read,
        )?,
    ];
    let mut expires_at = (crate::now() + 5)
        .min(actor.expires_at)
        .min(selection.origin.request.expires_at);
    let (primary,candidate,context):(String,String,String)=c.query_row("SELECT l.primary_connection_id,l.connection_id,n.context_selection_json FROM native_consultation_returns n JOIN consultation_candidate_bindings l ON l.operation_record_id=n.consultation_record_id WHERE n.operation_record_id=?1",[&selection.origin.record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    let context: serde_json::Value =
        serde_json::from_str(&context).map_err(|_| Error::storage())?;
    let actual = selection
        .sources
        .iter()
        .map(|s| s.connection_id().to_owned())
        .collect::<BTreeSet<_>>();
    if actual.len() != selection.sources.len() {
        return Err(Error::forbidden());
    }
    let mut runtime_ids = selection
        .sources
        .iter()
        .map(|s| s.runtime_connection_id())
        .collect::<BTreeSet<_>>();
    if runtime_ids.len() != selection.sources.len()
        || selection
            .sips
            .iter()
            .any(|s| !runtime_ids.insert(s.runtime_connection_id()))
    {
        return Err(Error::forbidden());
    }
    let mut statement=c.prepare("SELECT v.id FROM connections v JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 WHERE v.session_id=?1 AND v.observed_runtime_id=?2 AND v.state IN ('ready','active')").map_err(|_|Error::storage())?;
    let expected = statement
        .query_map(
            params![selection.origin.request.session_id, store.runtime_epoch()],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(|_| Error::storage())?;
    if actual != expected {
        return Err(Error::new("forbidden", "Native return source set changed"));
    }
    for source in &selection.sips {
        for original in context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .chain(std::iter::once(&context["remote"]))
        {
            if original["connection_id"] == source.connection_id()
                && original != &source.facts(store, c)?.1
            {
                return Err(Error::forbidden());
            }
        }
    }
    let (sip_scopes, sip_expiry) = store.sip_audience_facts(
        c,
        &selection.origin.request.conversation_id,
        &selection.origin.request.session_id,
        &selection.sips,
        selection
            .sources
            .iter()
            .map(|s| s.connection_id().to_owned()),
    )?;
    scopes.extend(sip_scopes);
    expires_at = expires_at.min(sip_expiry);
    let mut snapshots = BTreeMap::new();
    let mut may_subscribe = BTreeMap::new();
    for source in &selection.sources {
        if source.session_id() != selection.origin.request.session_id
            || source.conversation_id() != selection.origin.request.conversation_id
        {
            return Err(Error::forbidden());
        }
        for original in context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .chain(std::iter::once(&context["remote"]))
        {
            if original["connection_id"] == source.connection_id() && original != &describe(source)
            {
                return Err(Error::forbidden());
            }
        }
        let (view, required, expiry) = if source.connection_id() == candidate {
            closure_facts(store, c, source)?
        } else {
            facts(store, c, source)?
        };
        if !actor.permits_recipient(&view.session.actor.identity_id) {
            return Err(Error::forbidden());
        }
        scopes.extend(required);
        expires_at = expires_at.min(expiry);
        may_subscribe.insert(
            source.connection_id().to_owned(),
            subscription_permission(c, source)?,
        );
        snapshots.insert(source.connection_id().to_owned(), view);
    }
    let mut views = BTreeMap::new();
    for target in &selection.sources {
        views.insert(
            (target.connection_id().into(), None),
            (
                snapshots[target.connection_id()].clone(),
                None,
                may_subscribe[target.connection_id()],
            ),
        );
        for source in &selection.sources {
            if target.connection_id() == source.connection_id()
                || target.connection_id() == candidate
                || source.connection_id() == candidate
            {
                continue;
            }
            let mut allowed = true;
            for (private, other) in [(target, source), (source, target)] {
                if private.connection_id() != primary {
                    if let Some(partner) =
                        crate::connection_control::private_consultation::private_partner(
                            c,
                            private.connection_id(),
                        )?
                    {
                        if partner != other.connection_id() {
                            allowed = false;
                        }
                    }
                }
            }
            if allowed {
                let receiver = &snapshots[target.connection_id()];
                let publisher = &snapshots[source.connection_id()];
                if !crate::audience::member(
                    c,
                    &receiver.session.actor.identity_id,
                    target.conversation_id(),
                    true,
                )?
                .active
                {
                    return Err(Error::forbidden());
                }
                let target_credential = target
                    .binding
                    .credentials
                    .last()
                    .ok_or_else(Error::forbidden)?;
                let source_credential = source
                    .binding
                    .credentials
                    .last()
                    .ok_or_else(Error::forbidden)?;
                let receiver_actor = crate::identity::actor_from_connection(c, target_credential)?;
                let publisher_actor = crate::identity::actor_from_connection(c, source_credential)?;
                if receiver_actor.permits_recipient(&publisher.session.actor.identity_id)
                    && publisher_actor.permits_recipient(&receiver.session.actor.identity_id)
                {
                    views.insert(
                        (
                            target.connection_id().into(),
                            Some(source.connection_id().into()),
                        ),
                        (
                            receiver.clone(),
                            Some(publisher.clone()),
                            may_subscribe[target.connection_id()],
                        ),
                    );
                }
            }
        }
    }
    Ok((views, AuthorityScope::merged(&scopes)?, expires_at))
}
impl Store {
    pub fn try_native_return_read(
        &self,
        origin: &NativeConsultationReturnOrigin,
        sources: &[Arc<NativeMediaOrigin>],
        sips: &[SipAudienceOrigin],
    ) -> Result<NativeReturnRead> {
        let selection = NativeReturnSelection {
            origin: origin.clone(),
            sources: sources.to_vec(),
            sips: sips.to_vec(),
            sip_snapshots: {
                let c = self.try_connection()?;
                consultation_sip::snapshot(self, &c, sips)?
            },
        };
        let scopes = {
            let c = self.try_connection()?;
            resolve(self, &c, &selection)?.1
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (views, current, expires_at) = resolve(self, &c, &selection)?;
        if !self.authority_covers(&held, &current) || crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let mut reads = views
            .into_iter()
            .map(|(key, (target, source, may_subscribe))| {
                (
                    key,
                    Arc::new(NativeMediaRead {
                        target,
                        source,
                        _scopes: held.clone(),
                        owner: owner.clone(),
                        expires_at,
                        may_subscribe,
                    }),
                )
            })
            .collect();
        let sip_reads =
            consultation_sip::return_reads(self, &c, sips, origin, held.clone(), expires_at)?;
        let (primary, candidate): (String, String) = c.query_row(
            "SELECT primary_connection_id,connection_id FROM consultation_candidate_bindings WHERE operation_record_id=?1",
            [origin.consultation_record_id()], |r| Ok((r.get(0)?,r.get(1)?)),
        ).map_err(|_|Error::storage())?;
        let sip_send = consultation_sip::return_edges(
            self, &c, sources, &sip_reads, &mut reads, &primary, &candidate,
        )?;
        Ok(NativeReturnRead {
            candidate,
            sip_send,
            selection,
            reads,
            expires_at,
            sip_reads,
        })
    }
}

pub(super) fn describe(s: &NativeMediaOrigin) -> serde_json::Value {
    serde_json::json!({"connection_id":s.connection_id(),"runtime_connection_id":s.runtime_connection_id(),"participant_id":s.participant_id(),"membership_epoch":s.membership_epoch(),"generation":s.generation(),"credential_id":s.binding.credentials.last(),"key_id":s.binding.key_id,"key_algorithm":s.binding.algorithm,"public_key_sha256":hex::encode(Sha256::digest(&s.public_key)),"device_id":s.binding.device.id,"device_kind":s.binding.device.kind,"device_platform":s.binding.device.platform,"endpoint_id":s.endpoint_id,"physical_expires_at":s.expires_at})
}

// Candidate closure uses current physical facts, independent of an ended parent
// media wait. This view grants no receive pair involving the private candidate.
pub(super) fn closure_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    physical_facts(store, c, origin, true, true)
}
/// Only a validated current handoff lifetime can replace old offer recipients.
pub(super) fn handoff_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    physical_facts(store, c, origin, false, true)
}
pub(super) fn signal_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    physical_facts(store, c, origin, false, false)
}
fn physical_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    original_resources: bool,
    require_media: bool,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    current_owner(store, origin)?;
    super::credential_refresh::require_epoch(store, c, origin)?;
    let attempt_expiry = origin.attempt.current(c, store)?;
    let (meta, key, mut scopes, expires_at) = checked(store, c, &origin.binding)?;
    if require_media {
        meta.actor.require("connection.offer")?;
    }
    if key.public_key != origin.public_key {
        return Err(Error::forbidden());
    }
    if original_resources {
        let recipients =
            crate::connection_control::intent_resources(c, &meta.actor, &origin.intent)?;
        if recipients
            .iter()
            .any(|id| !meta.actor.permits_recipient(id))
        {
            return Err(Error::forbidden());
        }
    }
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &origin.intent.conversation_id,
        AuthorityMode::Read,
    )?);
    let projection = super::connection_projection::current_projection(
        c,
        &meta.actor,
        &origin.intent,
        &origin.endpoint_id,
        &origin.binding.device.id,
        store.runtime_epoch(),
        false,
    )?;
    if projection.runtime_connection_id != origin.runtime_connection_id
        || projection.connection.participant_id != origin.participant_id
        || projection.connection.membership_epoch != origin.membership_epoch
        || projection.connection.generation != origin.generation
    {
        return Err(Error::forbidden());
    }
    Ok((
        projection,
        scopes,
        expires_at
            .min(origin.expires_at)
            .min(attempt_expiry.unwrap_or(i64::MAX)),
    ))
}
