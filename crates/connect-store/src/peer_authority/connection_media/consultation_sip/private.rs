//! Typed private SIP/native pairs. Metadata derivation grants no IO; owned
//! directional grants are issued only under the complete scene interval.
use super::*;
use crate::identity::Actor;
use crate::{ConsultationMediaOrigin, NativeConsultationOrigin};
use std::collections::{BTreeMap, BTreeSet};

type Pairs = BTreeSet<(String, String)>;

impl Store {
    /// Current captured SIP private-slot facts, for interval derivation and
    /// established-context validation. No reader or execution right is returned.
    pub(crate) fn sip_private_snapshot_facts(
        &self,
        c: &Connection,
        binding: &serde_json::Value,
        conversation: &str,
        session: &str,
    ) -> Result<(Vec<AuthorityScope>, Vec<Actor>, i64)> {
        let (scopes, expiry) =
            self.sip_audience_snapshot_facts(c, binding, conversation, session)?;
        let text = |key: &str| binding[key].as_str().ok_or_else(Error::storage);
        let mut actors = vec![crate::identity::actor_from_connection(
            c,
            text("route_credential_id")?,
        )?];
        if binding["source_kind"] == "initial-sip" {
            actors.push(crate::identity::actor_from_connection(
                c,
                text("endpoint_credential_id")?,
            )?);
        }
        actors
            .last()
            .ok_or_else(Error::storage)?
            .require("stream.subscribe")?;
        Ok((scopes, actors, expiry))
    }
}

pub(in super::super) fn private_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeConsultationOrigin,
    private: &[ConsultationMediaOrigin],
) -> Result<(Vec<AuthorityScope>, i64)> {
    let phones = private
        .iter()
        .filter_map(ConsultationMediaOrigin::sip)
        .collect::<Vec<_>>();
    if phones.is_empty() {
        return Ok((vec![], i64::MAX));
    }
    // A SIP candidate needs its own protected admission. This increment never
    // substitutes an already public call for the registered native consultant.
    if phones.len() != 1
        || private.len() != 2
        || origin.request().connection_id.as_deref() != Some(phones[0].connection_id())
    {
        return Err(Error::new("unsupported-media-profile", "Private SIP selection requires a SIP primary and its separately admitted native consultant"));
    }
    let mut scopes = vec![];
    let mut expiry = i64::MAX;
    let mut principals = Vec::new();
    for source in private {
        let original: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2 AND membership_epoch=?3)",params![origin.record_id(),source.participant_id(),source.membership_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !original {
            return Err(Error::forbidden());
        }
        match source {
            ConsultationMediaOrigin::Native(native) => {
                let registered: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_legs WHERE operation_record_id=?1 AND connection_id=?2 AND primary_connection_id=?3 AND credential_id=?4 AND original_digest=?5 AND runtime_epoch=?6)",params![origin.record_id(),native.connection_id(),phones[0].connection_id(),origin.credential,origin.digest,store.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !registered
                    || crate::connection_control::private_consultation::private_partner(
                        c,
                        native.connection_id(),
                    )?
                    .as_deref()
                        != Some(phones[0].connection_id())
                {
                    return Err(Error::forbidden());
                }
                if !subscription_permission(c, native)? {
                    return Err(Error::forbidden());
                }
                let actor = store.native_media_route_actor(c, native)?;
                principals.push((actor.identity_id.clone(), vec![actor]));
            }
            ConsultationMediaOrigin::Sip(_) => {
                let binding = source.snapshot(store, c)?;
                let (needed, actors, until) = store.sip_private_snapshot_facts(
                    c,
                    &binding,
                    &origin.request().conversation_id,
                    &origin.request().session_id,
                )?;
                scopes.extend(needed);
                expiry = expiry.min(until);
                principals.push((
                    binding["identity_id"]
                        .as_str()
                        .ok_or_else(Error::storage)?
                        .to_owned(),
                    actors,
                ));
            }
        }
    }
    for (_, actors) in &principals {
        for (identity, _) in &principals {
            if actors.iter().any(|a| !a.permits_recipient(identity)) {
                return Err(Error::forbidden());
            }
        }
    }
    Ok((AuthorityScope::merged(&scopes)?, expiry))
}

pub(in super::super) fn edges(
    store: &Store,
    c: &Connection,
    private: &[ConsultationMediaOrigin],
    outside: &[Arc<NativeMediaOrigin>],
    outside_sip: &[SipAudienceOrigin],
    phones: &BTreeMap<String, Arc<SipAudienceRead>>,
    reads: &BTreeMap<(String, Option<String>), Arc<NativeMediaRead>>,
) -> Result<(Pairs, Pairs)> {
    let selected = private
        .iter()
        .map(|p| p.connection_id())
        .collect::<BTreeSet<_>>();
    let outside_phones = outside_sip
        .iter()
        .map(|p| p.connection_id())
        .collect::<BTreeSet<_>>();
    let outside_natives = outside
        .iter()
        .map(|p| p.connection_id())
        .collect::<BTreeSet<_>>();
    let mut receive = Pairs::new();
    let mut send = Pairs::new();
    for native in private
        .iter()
        .filter_map(ConsultationMediaOrigin::native)
        .chain(outside.iter().map(Arc::as_ref))
    {
        for (id, phone) in phones {
            let new_pair =
                selected.contains(native.connection_id()) && selected.contains(id.as_str());
            let preserved_pair = outside_natives.contains(native.connection_id())
                && outside_phones.contains(id.as_str())
                && crate::connection_control::private_consultation::private_partner(
                    c,
                    native.connection_id(),
                )?
                .as_deref()
                    == Some(id.as_str())
                && crate::connection_control::private_consultation::private_partner(c, id)?
                    .as_deref()
                    == Some(native.connection_id());
            if !new_pair && !preserved_pair {
                continue;
            }
            let native_read = reads
                .get(&(native.connection_id().into(), None))
                .ok_or_else(Error::forbidden)?;
            let view = native_read.target();
            let record = view
                .private_consultation
                .as_deref()
                .ok_or_else(Error::forbidden)?;
            let registered: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_legs WHERE operation_record_id=?1 AND connection_id=?2 AND primary_connection_id=?3 AND runtime_epoch=?4)",params![record,native.connection_id(),id,store.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !registered {
                return Err(Error::forbidden());
            }
            let actor = store.native_media_route_actor(c, native)?;
            if !phone.current()
                || !native_read.current()
                || !native_read.permits_subscriptions()
                || !phone.may_receive()
                || !actor.permits_recipient(&phone.member().identity_id)
                || !phone.permits_recipient(&view.session.actor.identity_id)
            {
                return Err(Error::forbidden());
            }
            receive.insert((native.connection_id().into(), id.clone()));
            send.insert((id.clone(), native.connection_id().into()));
        }
    }
    Ok((receive, send))
}
