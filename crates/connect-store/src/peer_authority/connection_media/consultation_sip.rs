//! Exact initial and committed SIP outsiders. These own only unchanged public programmes
//! plus private exclusion; they can never stand in for a private native slot.
use super::*;
use crate::{InitialTelephoneMediaOrigin, InitialTelephoneMediaRead};
use std::collections::{BTreeMap, BTreeSet};

mod origins;
mod revisions;
pub use origins::{SipAudienceOrigin, SipAudienceRead};
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "source_kind", rename_all = "kebab-case", deny_unknown_fields)]
#[allow(dead_code)] // Historical metadata grants no private slot.
pub(crate) enum SipSnapshot {
    InitialSip {
        record_id: String,
        connection_id: String,
        runtime_connection_id: String,
        sip_call_id: String,
        generation: u64,
        participant_id: String,
        membership_epoch: u64,
        identity_id: String,
        endpoint_id: String,
        endpoint_revision: u64,
        device_id: String,
        route_credential_id: String,
        route_owner_id: String,
        physical_expires_at: i64,
        endpoint_credential_id: String,
    },
    EstablishedSip {
        record_id: String,
        connection_id: String,
        runtime_connection_id: String,
        sip_call_id: String,
        generation: u64,
        participant_id: String,
        membership_epoch: u64,
        identity_id: String,
        endpoint_id: String,
        endpoint_revision: u64,
        device_id: String,
        route_credential_id: String,
        route_owner_id: String,
        physical_expires_at: i64,
    },
}
impl SipSnapshot {
    pub(crate) fn connection_id(&self) -> &str {
        match self {
            Self::InitialSip { connection_id, .. } | Self::EstablishedSip { connection_id, .. } => {
                connection_id
            }
        }
    }
}
impl Store {
    /// Historical absence only, bound to the captured physical call and original
    /// member epoch. This supplies no current media or command authority.
    pub(crate) fn recorded_sip_departure(
        c: &Connection,
        binding: &serde_json::Value,
        runtime: &str,
    ) -> Result<bool> {
        let snapshot: SipSnapshot =
            serde_json::from_value(binding.clone()).map_err(|_| Error::storage())?;
        let text = |key: &str| binding[key].as_str().ok_or_else(Error::storage);
        let generation = binding["generation"].as_u64().ok_or_else(Error::storage)?;
        let epoch = binding["membership_epoch"]
            .as_u64()
            .ok_or_else(Error::storage)?;
        let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND participant_id=?2 AND membership_epoch=?3 AND identity_id=?4 AND endpoint_id=?5 AND (observed_runtime_id=?6 OR observed_runtime_id IS NULL) AND generation>=?7 AND state IN ('retired','failed'))",params![snapshot.connection_id(),text("participant_id")?,epoch,text("identity_id")?,text("endpoint_id")?,runtime,generation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Ok(false);
        }
        match snapshot {
            SipSnapshot::InitialSip { .. } => {
                if generation != 1 {
                    return Ok(false);
                }
                c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND o.connection_id=?2 AND n.attempt_runtime=?3 AND n.runtime_connection_id=?4 AND n.sip_call_id=?5 AND n.signaling_started=1 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NOT NULL)",params![text("record_id")?,text("connection_id")?,runtime,text("runtime_connection_id")?,text("sip_call_id")?],|r|r.get(0)).map_err(|_|Error::storage())
            }
            SipSnapshot::EstablishedSip { .. } => {
                if generation < 2 {
                    return Ok(false);
                }
                c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id AND t.runtime_epoch=l.runtime_epoch JOIN native_replacements n ON n.operation_record_id=l.operation_record_id JOIN operations o ON o.record_id=l.operation_record_id AND o.state='completed' JOIN operation_effects e ON e.id=t.effect_id AND e.state='confirmed' WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.runtime_epoch=?3 AND l.generation=?4 AND l.retired=1 AND n.candidate_runtime_id=?5 AND json_extract(t.descriptor_json,'$.media.call_id')=?6)",params![text("record_id")?,text("connection_id")?,runtime,generation,text("runtime_connection_id")?,text("sip_call_id")?],|r|r.get(0)).map_err(|_|Error::storage())
            }
        }
    }
    /// Resolve only a previously observed metadata origin in this runtime,
    /// then check the complete captured shape against fresh current facts.
    /// The bounded readiness registry owns no reader; it cannot authorize IO.
    pub(crate) fn sip_audience_snapshot_facts(
        &self,
        c: &Connection,
        binding: &serde_json::Value,
        conversation: &str,
        session: &str,
    ) -> Result<(Vec<AuthorityScope>, i64)> {
        let snapshot: SipSnapshot =
            serde_json::from_value(binding.clone()).map_err(|_| Error::storage())?;
        let cached = self
            .session_ready_origins
            .lock()
            .map_err(|_| Error::storage())?
            .get(snapshot.connection_id())
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let origin = match cached {
            crate::session_lifecycle::ReadyOrigin::InitialTelephone(o) => {
                SipAudienceOrigin::Initial(o)
            }
            crate::session_lifecycle::ReadyOrigin::Telephone(o) => {
                SipAudienceOrigin::Established(o)
            }
            crate::session_lifecycle::ReadyOrigin::Native(_) => return Err(Error::forbidden()),
        };
        if origin.conversation_id() != conversation || origin.session_id() != session {
            return Err(Error::forbidden());
        }
        let (scopes, current, expiry) = origin.facts(self, c)?;
        if current != *binding {
            return Err(Error::forbidden());
        }
        Ok((scopes, expiry))
    }
    pub(crate) fn sip_audience_facts(
        &self,
        c: &Connection,
        conversation: &str,
        session: &str,
        sources: &[SipAudienceOrigin],
        native_ids: impl IntoIterator<Item = String>,
    ) -> Result<(Vec<AuthorityScope>, i64)> {
        let store = self;
        let mut ids = native_ids.into_iter().collect::<BTreeSet<_>>();
        let mut actual = BTreeSet::new();
        let mut scopes = Vec::new();
        let mut expiry = crate::now() + 5;
        if sources.len() > store.limits.connections {
            return Err(Error::forbidden());
        }
        for source in sources {
            if source.conversation_id() != conversation
                || source.session_id() != session
                || !ids.insert(source.connection_id().into())
                || !actual.insert(source.runtime_connection_id().to_owned())
            {
                return Err(Error::forbidden());
            }
            let (needed, _, until) = source.facts(store, c)?;
            scopes.extend(needed);
            expiry = expiry.min(until);
        }
        let mut q=c.prepare("SELECT p.runtime_connection_id FROM initial_telephone_operations p JOIN initial_telephone_answers a ON a.operation_record_id=p.operation_record_id JOIN operations o ON o.record_id=p.operation_record_id JOIN connections v ON v.id=o.connection_id WHERE o.conversation_id=?1 AND o.session_id=?2 AND p.attempt_runtime=?3 AND v.observed_runtime_id=?3 AND v.generation=1 AND v.state IN ('ready','active') AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL").map_err(|_|Error::storage())?;
        let mut expected = q
            .query_map(params![conversation, session, store.runtime_epoch()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(|_| Error::storage())?;
        let mut q=c.prepare("SELECT n.candidate_runtime_id FROM telephone_connection_lifetimes l JOIN native_replacements n ON n.operation_record_id=l.operation_record_id JOIN operations o ON o.record_id=l.operation_record_id JOIN connections v ON v.id=l.connection_id WHERE o.conversation_id=?1 AND o.session_id=?2 AND l.runtime_epoch=?3 AND v.observed_runtime_id=?3 AND v.generation=l.generation AND v.state IN ('ready','active') AND l.retired=0 AND o.state='completed'").map_err(|_|Error::storage())?;
        let established = q
            .query_map(params![conversation, session, store.runtime_epoch()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(|_| Error::storage())?;
        expected.extend(established);
        if actual != expected {
            return Err(Error::new(
                "authority-unavailable",
                "SIP audience source set changed",
            ));
        }
        Ok((scopes, expiry))
    }
}
pub(super) fn reads(
    store: &Store,
    c: &Connection,
    sources: &[SipAudienceOrigin],
    held: Arc<AuthorityGuardSet>,
    until: i64,
) -> Result<BTreeMap<String, Arc<SipAudienceRead>>> {
    sources
        .iter()
        .map(|s| {
            Ok((
                s.connection_id().into(),
                Arc::new(s.read(store, c, held.clone(), until)?),
            ))
        })
        .collect()
}
/// A source edge shares the real native target view and the separately typed
/// SIP source read. No SIP PeerConnectionProjection or signing fields exist.
pub(super) fn receive_edges(
    store: &Store,
    c: &Connection,
    natives: &[Arc<NativeMediaOrigin>],
    phones: &BTreeMap<String, Arc<SipAudienceRead>>,
    targets: &mut BTreeMap<(String, Option<String>), Arc<NativeMediaRead>>,
) -> Result<()> {
    for native in natives {
        let target = targets
            .get(&(native.connection_id().into(), None))
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let view = target.target();
        if view.private_consultation.is_some()
            || view.handoff_preparation
            || view.publication_suspended
            || !target.permits_subscriptions()
        {
            continue;
        }
        let actor = store.native_media_route_actor(c, native)?;
        for (id, phone) in phones {
            if phone.current()
                && target.current()
                && actor.permits_recipient(&phone.member().identity_id)
                && phone.permits_recipient(&view.session.actor.identity_id)
            {
                targets.insert(
                    (native.connection_id().into(), Some(id.clone())),
                    target.clone(),
                );
            }
        }
    }
    Ok(())
}
pub(super) fn snapshot(
    store: &Store,
    c: &Connection,
    sources: &[SipAudienceOrigin],
) -> Result<Vec<serde_json::Value>> {
    let mut sorted = sources.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|s| s.connection_id());
    sorted
        .into_iter()
        .map(|s| s.facts(store, c).map(|(_, v, _)| v))
        .collect()
}
pub(super) fn validate_revisions(
    c: &Connection,
    sources: &[SipAudienceOrigin],
    plan: &serde_json::Value,
) -> Result<()> {
    for source in sources {
        let current:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![source.connection_id(),source.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if plan["expected_receiver_revisions"][source.runtime_connection_id()].as_u64()
            != Some(current)
        {
            return Err(Error::new("conflict", "SIP audience revision changed"));
        }
    }
    Ok(())
}
