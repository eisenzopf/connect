//! Fresh complete private-route planning views. These do not reserve an engine attempt.
use super::consultation_sip;
use super::*;
use crate::{NativeConsultationOrigin, SipAudienceOrigin, SipAudienceRead};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type ReadKey = (String, Option<String>);
type Views = BTreeMap<
    ReadKey,
    (
        PeerConnectionProjection,
        Option<PeerConnectionProjection>,
        bool,
    ),
>;
/// One bounded current planning interval. This is not a native execution lease;
/// the coordinator must durably reserve its exact attempt before any Core call.
pub struct NativeConsultationRead {
    origin: NativeConsultationOrigin,
    reads: BTreeMap<ReadKey, Arc<NativeMediaRead>>,
    sip_reads: BTreeMap<String, Arc<SipAudienceRead>>,
    expires_at: i64,
    selection: NativeConsultationSelection,
}
/// Immutable selected origins, never an owned reader or execution permission.
#[derive(Clone)]
pub struct NativeConsultationSelection {
    pub(crate) origin: NativeConsultationOrigin,
    remote: ConsultationMediaOrigin,
    remote_snapshot: serde_json::Value,
    private: Vec<Arc<NativeMediaOrigin>>,
    outside: Vec<Arc<NativeMediaOrigin>>,
    outside_sip: Vec<SipAudienceOrigin>,
    outside_sip_snapshots: Vec<serde_json::Value>,
}
impl NativeConsultationSelection {
    pub(crate) fn scopes(
        &self,
        store: &Store,
        c: &Connection,
        write: bool,
    ) -> Result<Vec<AuthorityScope>> {
        let mut scopes = selected_views(
            store,
            c,
            &self.origin,
            &self.remote,
            &self.private,
            &self.outside,
            &self.outside_sip,
        )?
        .1;
        if write {
            let (actor, _) = self.origin.require(store, c)?;
            scopes.extend([
                AuthorityScope::identity(&actor.identity_id, AuthorityMode::Write)?,
                AuthorityScope::conversation(
                    &self.origin.request.conversation_id,
                    AuthorityMode::Write,
                )?,
            ]);
        }
        AuthorityScope::merged(&scopes)
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        let describe = |s: &NativeMediaOrigin| serde_json::json!({"connection_id":s.connection_id(),"runtime_connection_id":s.runtime_connection_id(),"participant_id":s.participant_id(),"membership_epoch":s.membership_epoch(),"generation":s.generation(),"credential_id":s.binding.credentials.last(),"key_id":s.binding.key_id,"key_algorithm":s.binding.algorithm,"public_key_sha256":hex::encode(Sha256::digest(&s.public_key)),"device_id":s.binding.device.id,"device_kind":s.binding.device.kind,"device_platform":s.binding.device.platform,"endpoint_id":s.endpoint_id,"physical_expires_at":s.expires_at});
        let mut private = self.private.iter().collect::<Vec<_>>();
        private.sort_by_key(|s| s.connection_id());
        {
            let mut outside = self.outside.iter().collect::<Vec<_>>();
            outside.sort_by_key(|s| s.connection_id());
            let mut value = serde_json::json!({"remote":self.remote_snapshot,"private":private.into_iter().map(|s|describe(s)).collect::<Vec<_>>(),"outside":outside.into_iter().map(|s|describe(s)).collect::<Vec<_>>()});
            if !self.outside_sip_snapshots.is_empty() {
                value["outside_sip"] = serde_json::json!(self.outside_sip_snapshots);
            }
            value
        }
    }
    pub(crate) fn validate_plan(&self, plan: &serde_json::Value) -> Result<()> {
        let invalid = || Error::new("invalid-request", "Invalid native consultation plan");
        if plan["operation_id"] != self.origin.record
            || !plan["audience_revision"]
                .as_u64()
                .is_some_and(|r| r < 9_007_199_254_740_991)
        {
            return Err(invalid());
        }
        let expected = self
            .private
            .iter()
            .map(|s| s.runtime_connection_id())
            .collect::<BTreeSet<_>>();
        let ids = plan["private_connections"].as_array().ok_or_else(invalid)?;
        let selected = ids
            .iter()
            .map(|v| v.as_str().ok_or_else(invalid))
            .collect::<Result<BTreeSet<_>>>()?;
        if selected != expected || selected.len() != ids.len() {
            return Err(invalid());
        }
        let activated = plan["activate_muted"].as_str().ok_or_else(invalid)?;
        if !expected.contains(activated)
            || self
                .origin
                .request
                .connection_id
                .as_ref()
                .is_some_and(|id| {
                    self.private
                        .iter()
                        .any(|p| p.connection_id() == id && p.runtime_connection_id() == activated)
                })
        {
            return Err(invalid());
        }
        let receivers = plan["receivers"].as_object().ok_or_else(invalid)?;
        if receivers.len() != expected.len() + 1
            || !receivers
                .get(self.remote.runtime_connection_id())
                .is_some_and(|r| r.as_array().is_some_and(Vec::is_empty))
        {
            return Err(invalid());
        }
        for id in &expected {
            let sources = receivers
                .get(*id)
                .and_then(|r| r.as_array())
                .ok_or_else(invalid)?;
            let actual = sources
                .iter()
                .map(|v| v.as_str().ok_or_else(invalid))
                .collect::<Result<BTreeSet<_>>>()?;
            if actual.len() != sources.len()
                || actual != expected.iter().copied().filter(|s| s != id).collect()
            {
                return Err(invalid());
            }
        }
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .ok_or_else(invalid)?;
        let complete = expected
            .iter()
            .copied()
            .chain(std::iter::once(self.remote.runtime_connection_id()))
            .chain(self.outside.iter().map(|p| p.runtime_connection_id()))
            .chain(self.outside_sip.iter().map(|p| p.runtime_connection_id()))
            .collect::<BTreeSet<_>>();
        if revisions.len() > 512
            || revisions
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != complete
            || !expected
                .iter()
                .copied()
                .chain(std::iter::once(self.remote.runtime_connection_id()))
                .all(|s| revisions.contains_key(s))
        {
            return Err(invalid());
        }
        for (id, revision) in revisions {
            connect_protocol::transitions::id(id)?;
            if !revision.as_u64().is_some_and(|r| r < 9_007_199_254_740_991) {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub(crate) fn validate_revisions(
        &self,
        c: &Connection,
        plan: &serde_json::Value,
    ) -> Result<()> {
        for source in self
            .remote
            .native()
            .into_iter()
            .chain(self.private.iter().map(Arc::as_ref))
            .chain(self.outside.iter().map(Arc::as_ref))
        {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[source.connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if plan["expected_receiver_revisions"][source.runtime_connection_id()].as_u64()
                != Some(revision)
            {
                return Err(Error::new(
                    "conflict",
                    "Native consultation receiver revision changed",
                ));
            }
        }
        let sips = self
            .remote
            .sip()
            .cloned()
            .into_iter()
            .chain(self.outside_sip.iter().cloned())
            .collect::<Vec<_>>();
        consultation_sip::validate_revisions(c, &sips, plan)?;
        Ok(())
    }
    pub(crate) fn read(&self, store: &Store) -> Result<NativeConsultationRead> {
        store.try_native_consultation_scene_read(
            &self.origin,
            &self.remote,
            &self.private,
            &self.outside,
            &self.outside_sip,
        )
    }
}
impl NativeConsultationRead {
    pub fn is_retained_sip(&self, id: &str) -> bool {
        self.selection
            .remote
            .sip()
            .is_some_and(|o| o.connection_id() == id)
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub(crate) fn into_selection(self) -> Result<NativeConsultationSelection> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(self.selection.clone())
    }
    pub(crate) fn limit(&mut self, expires: i64) -> Result<()> {
        self.expires_at = self.expires_at.min(expires);
        for read in self.reads.values_mut() {
            let read = Arc::get_mut(read).ok_or_else(Error::forbidden)?;
            read.expires_at = read.expires_at.min(expires);
        }
        Ok(())
    }
    pub(crate) fn selection(&self) -> &NativeConsultationSelection {
        &self.selection
    }

    pub fn current(&self) -> bool {
        crate::now() < self.expires_at
            && self.reads.values().all(|r| r.current())
            && self.sip_reads.values().all(|r| r.current())
    }
    pub fn sip(&self, connection: &str) -> Option<Arc<SipAudienceRead>> {
        self.sip_reads.get(connection).cloned()
    }
    pub fn origin(&self) -> &NativeConsultationOrigin {
        &self.origin
    }
    pub fn media(&self, target: &str, source: Option<&str>) -> Option<&NativeMediaRead> {
        self.reads
            .get(&(target.into(), source.map(str::to_owned)))
            .map(Arc::as_ref)
    }
}
fn selected_views(
    store: &Store,
    c: &Connection,
    origin: &NativeConsultationOrigin,
    remote: &ConsultationMediaOrigin,
    private: &[Arc<NativeMediaOrigin>],
    outside: &[Arc<NativeMediaOrigin>],
    outside_sip: &[SipAudienceOrigin],
) -> Result<(Views, Vec<AuthorityScope>, i64)> {
    let (actor, mut scopes) = origin.require(store, c)?;
    let request = &origin.request;
    let connect_protocol::transitions::TransitionIntent::Consult {
        private_participants,
        retained_connid,
        hold_treatment,
    } = &request.intent
    else {
        return Err(Error::forbidden());
    };
    if *hold_treatment != connect_protocol::transitions::HoldTreatment::Silence {
        return Err(Error::new(
            "unsupported-media-profile",
            "Configured consultation hold source is not installed",
        ));
    }
    if private.len() > 16 {
        return Err(Error::new(
            "unsupported-media-profile",
            "Private native audience exceeds the engine profile",
        ));
    }
    let selected = private
        .iter()
        .map(|p| p.participant_id())
        .collect::<BTreeSet<_>>();
    if selected.len() != private.len()
        || selected != private_participants.iter().map(String::as_str).collect()
        || remote.connection_id() != retained_connid
        || selected.contains(remote.participant_id())
        || request
            .connection_id
            .as_ref()
            .is_some_and(|id| !private.iter().any(|p| p.connection_id() == id))
    {
        return Err(Error::forbidden());
    }
    let original_remote: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2 AND membership_epoch=?3)", params![origin.record,remote.participant_id(),remote.membership_epoch()], |r| r.get(0)).map_err(|_|Error::storage())?;
    if !original_remote {
        return Err(Error::forbidden());
    }
    let sips = remote
        .sip()
        .cloned()
        .into_iter()
        .chain(outside_sip.iter().cloned())
        .collect::<Vec<_>>();
    let (sip_scopes, sip_expiry) = store.sip_audience_facts(
        c,
        &request.conversation_id,
        &request.session_id,
        &sips,
        remote
            .native()
            .into_iter()
            .map(|o| o.connection_id().to_owned())
            .chain(
                private
                    .iter()
                    .chain(outside)
                    .map(|s| s.connection_id().to_owned()),
            ),
    )?;
    scopes.extend(sip_scopes);
    let mut result = BTreeMap::new();
    let mut expiry = request
        .expires_at
        .min(actor.expires_at)
        .min(crate::now() + 5)
        .min(sip_expiry);
    let mut connections = BTreeSet::new();
    let mut runtime_ids = BTreeSet::new();
    for target in remote
        .native()
        .into_iter()
        .chain(private.iter().map(Arc::as_ref))
        .chain(outside.iter().map(Arc::as_ref))
    {
        if target.conversation_id() != request.conversation_id
            || target.session_id() != request.session_id
            || !connections.insert(target.connection_id())
        {
            return Err(Error::forbidden());
        }
        let original:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2 AND membership_epoch=?3)",params![origin.record,target.participant_id(),target.membership_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !original
            && !outside
                .iter()
                .any(|p| p.connection_id() == target.connection_id())
        {
            return Err(Error::forbidden());
        }
        runtime_ids.insert(target.runtime_connection_id().to_owned());
        let (target_view, source_view, needed, until) = views(store, c, target, None)?;
        if !crate::transitions::consultation_source::ready(
            store,
            c,
            target.connection_id(),
            &target_view.connection.state,
        )? {
            return Err(Error::forbidden());
        }
        scopes.extend(needed);
        expiry = expiry.min(until);
        let subscriptions = subscription_permission(c, target)?;
        result.insert(
            (target.connection_id().into(), None),
            (target_view, source_view, subscriptions),
        );
    }
    if sips
        .iter()
        .any(|s| runtime_ids.contains(s.runtime_connection_id()))
    {
        return Err(Error::forbidden());
    }
    let mut roster=c.prepare("SELECT m.runtime_connection_id FROM connections n JOIN connection_offers m ON m.connection_id=n.id JOIN native_connection_lifetimes l ON l.connection_id=n.id AND l.generation=n.generation AND l.runtime_epoch=n.observed_runtime_id WHERE n.session_id=?1 AND n.state IN ('ready','active') AND n.observed_runtime_id=?2 AND l.retired=0").map_err(|_|Error::storage())?;
    let actual = roster
        .query_map(params![request.session_id, store.runtime_epoch()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(|_| Error::storage())?;
    if actual != runtime_ids {
        return Err(Error::new(
            "authority-unavailable",
            "Native consultation scene selection changed",
        ));
    }
    for target in private {
        for source in private
            .iter()
            .filter(|source| source.connection_id() != target.connection_id())
        {
            let (target_view, source_view, needed, until) = views(store, c, target, Some(source))?;
            if !subscription_permission(c, target)? {
                return Err(Error::forbidden());
            }
            scopes.extend(needed);
            expiry = expiry.min(until);
            result.insert(
                (
                    target.connection_id().into(),
                    Some(source.connection_id().into()),
                ),
                (target_view, source_view, true),
            );
        }
    }
    // The exact new audience plan keeps other private groups' programs. Carry
    // only their already authorized physical partner pairs, under fresh current
    // views; never add a route from either group into the new private pair.
    for target in outside {
        let Some(partner) = crate::connection_control::private_consultation::private_partner(
            c,
            target.connection_id(),
        )?
        else {
            continue;
        };
        if let Some(source) = outside.iter().find(|s| s.connection_id() == partner) {
            let (target_view, source_view, needed, until) = views(store, c, target, Some(source))?;
            if !subscription_permission(c, target)? {
                return Err(Error::forbidden());
            }
            scopes.extend(needed);
            expiry = expiry.min(until);
            result.insert(
                (
                    target.connection_id().into(),
                    Some(source.connection_id().into()),
                ),
                (target_view, source_view, true),
            );
        }
    }
    if crate::now() >= expiry {
        return Err(Error::forbidden());
    }
    Ok((result, AuthorityScope::merged(&scopes)?, expiry))
}
impl Store {
    /// Resolve the complete selected private set and retained native remote,
    /// acquire their merged readers once, then resolve everything again. There
    /// is no weaker fallback when an origin, original epoch or grant changed.
    pub fn try_native_consultation_read(
        &self,
        origin: &NativeConsultationOrigin,
        remote: &NativeMediaOrigin,
        private: &[Arc<NativeMediaOrigin>],
    ) -> Result<NativeConsultationRead> {
        self.try_native_consultation_scene_read(
            origin,
            &ConsultationMediaOrigin::Native(Arc::new(remote.clone())),
            private,
            &[],
            &[],
        )
    }
    /// Outside routes keep their programmes. Their owned current views permit
    /// only the exact private-audience exclusion fence, never private receipt.
    pub fn try_native_consultation_scene_read(
        &self,
        origin: &NativeConsultationOrigin,
        remote: &ConsultationMediaOrigin,
        private: &[Arc<NativeMediaOrigin>],
        outside: &[Arc<NativeMediaOrigin>],
        outside_sip: &[SipAudienceOrigin],
    ) -> Result<NativeConsultationRead> {
        if private.len() + outside.len() + outside_sip.len() + 1 > self.limits.connections {
            return Err(Error::forbidden());
        }
        let scopes = {
            let c = self.try_connection()?;
            selected_views(self, &c, origin, remote, private, outside, outside_sip)?.1
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (current, needed, expiry) =
            selected_views(self, &c, origin, remote, private, outside, outside_sip)?;
        if !self.authority_covers(&held, &needed) {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let reads = current
            .into_iter()
            .map(|(key, (target, source, may_subscribe))| {
                (
                    key,
                    Arc::new(NativeMediaRead {
                        target,
                        source,
                        _scopes: held.clone(),
                        owner: owner.clone(),
                        expires_at: expiry,
                        may_subscribe,
                    }),
                )
            })
            .collect();
        let sips = remote
            .sip()
            .cloned()
            .into_iter()
            .chain(outside_sip.iter().cloned())
            .collect::<Vec<_>>();
        let result = NativeConsultationRead {
            origin: origin.clone(),
            reads,
            sip_reads: consultation_sip::reads(self, &c, &sips, held.clone(), expiry)?,
            expires_at: expiry,
            selection: NativeConsultationSelection {
                origin: origin.clone(),
                remote: remote.clone(),
                remote_snapshot: remote.snapshot(self, &c)?,
                private: private.to_vec(),
                outside: outside.to_vec(),
                outside_sip: outside_sip.to_vec(),
                outside_sip_snapshots: consultation_sip::snapshot(self, &c, outside_sip)?,
            },
        };
        if !result.current() {
            return Err(Error::forbidden());
        }
        Ok(result)
    }
}
