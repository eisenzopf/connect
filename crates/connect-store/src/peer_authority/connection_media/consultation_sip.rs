//! Exact initial and committed SIP outsiders. These own only unchanged public programmes
//! plus private exclusion; they can never stand in for a private native slot.
use super::*;
use crate::{InitialTelephoneMediaOrigin, InitialTelephoneMediaRead};
use std::collections::{BTreeMap, BTreeSet};

mod origins;
mod revisions;
pub use origins::{SipAudienceOrigin, SipAudienceRead};
#[derive(serde::Deserialize)]
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
impl Store {
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
