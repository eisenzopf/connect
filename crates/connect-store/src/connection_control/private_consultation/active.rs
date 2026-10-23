//! Current physical authority for an established private context. No execution
//! credential, expired lease, provider memory or retained reader is reused.
use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Incarnation {
    connection_id: String,
    runtime_connection_id: String,
    participant_id: String,
    membership_epoch: u64,
    generation: u64,
    credential_id: String,
    key_id: String,
    key_algorithm: String,
    public_key_sha256: String,
    device_id: String,
    device_kind: String,
    device_platform: String,
    endpoint_id: String,
    physical_expires_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    remote: Incarnation,
    private: Vec<Incarnation>,
    outside: Vec<Incarnation>,
    outside_sip: Option<Vec<crate::peer_authority::SipSnapshot>>,
}
pub(crate) struct Established {
    pub primary: String,
    pub remote: String,
    pub candidate: String,
    pub session: String,
    pub scopes: Vec<AuthorityScope>,
    pub expires_at: i64,
}

/// Resolve the exact captured physical scene under current native credential,
/// Device, Endpoint, membership and presence authority. Outside peers may leave
/// independently; they cannot inherit either private slot.
pub(crate) fn validate(
    store: &Store,
    c: &Connection,
    record: &str,
    selected: &str,
    audience: &str,
) -> Result<Established> {
    let selection: Selection = serde_json::from_str(selected).map_err(|_| Error::storage())?;
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(audience).map_err(|_| Error::storage())?;
    let (candidate, primary, session, conversation): (String,String,String,String) = c.query_row(
        "SELECT l.connection_id,l.primary_connection_id,v.session_id,s.conversation_id FROM native_consultation_legs l JOIN connections v ON v.id=l.primary_connection_id JOIN sessions s ON s.id=v.session_id WHERE l.operation_record_id=?1",
        [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).map_err(|_|Error::storage())?;
    if selection.private.len() != 2
        || audience.len() != 2
        || !selection.private.iter().any(|v| v.connection_id == primary)
        || !selection
            .private
            .iter()
            .any(|v| v.connection_id == candidate)
        || [primary.as_str(), candidate.as_str()].contains(&selection.remote.connection_id.as_str())
    {
        return Err(Error::forbidden());
    }
    // Validate even the unused metadata shape; it never adds authority.
    let _ = (selection.outside, selection.outside_sip);
    let mut scopes = vec![AuthorityScope::conversation(
        &conversation,
        AuthorityMode::Read,
    )?];
    let mut expires_at = i64::MAX;
    let mut private_actors = vec![];
    for original in selection
        .private
        .iter()
        .chain(std::iter::once(&selection.remote))
    {
        if crate::now() >= original.physical_expires_at {
            return Err(Error::forbidden());
        }
        let current:Option<(String,i64,i64,Vec<u8>)>=c.prepare_cached("SELECT v.identity_id,k.expires_at,key.expires_at,key.public_key FROM connections v
             JOIN connection_offers mapping ON mapping.connection_id=v.id
             JOIN native_connection_lifetimes life ON life.connection_id=v.id AND life.generation=v.generation AND life.runtime_epoch=v.observed_runtime_id AND life.retired=0
             JOIN native_admission_sources n ON n.operation_record_id=life.operation_record_id
             JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed'
             JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
             JOIN credentials k ON k.id=n.credential_id AND k.identity_id=v.identity_id AND k.revoked=0 AND k.expires_at>?8
             JOIN peer_signing_keys key ON key.key_id=n.key_id AND key.identity_id=v.identity_id AND key.device_id=n.device_id AND key.revoked=0 AND key.not_before<=?8 AND key.expires_at>?8
             JOIN credential_signing_keys binding ON binding.credential_id=k.id AND binding.key_id=key.key_id
             JOIN endpoints ep ON ep.id=v.endpoint_id AND ep.device_id=n.device_id AND ep.identity_id=v.identity_id AND ep.verification_state='verified'
             JOIN devices d ON d.id=ep.device_id AND d.identity_id=v.identity_id AND d.revoked=0
             JOIN participants p ON p.participant_id=v.participant_id AND p.identity_id=v.identity_id AND p.membership_epoch=v.membership_epoch AND p.active=1
             JOIN identities i ON i.id=p.identity_id AND i.revoked=0
             JOIN session_participants sp ON sp.session_id=v.session_id AND sp.participant_id=v.participant_id AND sp.membership_epoch=v.membership_epoch AND sp.state='present'
             JOIN sessions s ON s.id=v.session_id AND s.state IN ('accepted','ready','active')
             JOIN conversations room ON room.id=s.conversation_id AND room.closed=0
             WHERE v.id=?1 AND v.generation=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND mapping.runtime_connection_id=?5 AND v.observed_runtime_id=?6 AND v.session_id=?7 AND v.state IN ('ready','active')
             AND n.credential_id=?9 AND n.key_id=?10 AND ep.id=?11 AND ep.device_id=?12 AND d.kind=?13 AND d.platform=?14 AND key.algorithm=?15
             AND EXISTS(SELECT 1 FROM json_each(ep.capabilities_json) WHERE value='audio')
             AND EXISTS(SELECT 1 FROM json_each(s.media) WHERE value='audio')").map_err(|_|Error::storage())?.query_row(
            params![original.connection_id,original.generation,original.participant_id,original.membership_epoch,original.runtime_connection_id,store.runtime_epoch(),session,crate::now(),original.credential_id,original.key_id,original.endpoint_id,original.device_id,original.device_kind,original.device_platform,original.key_algorithm],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(|_|Error::storage())?;
        let (identity, credential_expiry, key_expiry, public_key) =
            current.ok_or_else(Error::forbidden)?;
        if hex::encode(Sha256::digest(&public_key)) != original.public_key_sha256 {
            return Err(Error::forbidden());
        }
        let actor = crate::identity::actor_from_connection(c, &original.credential_id)?;
        actor.require("connection.offer")?;
        if original.connection_id != selection.remote.connection_id
            && !audience.iter().any(|(i, p, e)| {
                i == &identity && p == &original.participant_id && *e == original.membership_epoch
            })
        {
            return Err(Error::forbidden());
        }
        if original.connection_id != selection.remote.connection_id {
            actor.require("stream.subscribe")?;
            private_actors.push(actor);
        }
        scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Read)?);
        expires_at = expires_at
            .min(credential_expiry)
            .min(key_expiry)
            .min(original.physical_expires_at);
    }
    for actor in &private_actors {
        if private_actors
            .iter()
            .any(|other| !actor.permits_recipient(&other.identity_id))
        {
            return Err(Error::forbidden());
        }
    }
    Ok(Established {
        primary,
        remote: selection.remote.connection_id,
        candidate,
        session,
        scopes: AuthorityScope::merged(&scopes)?,
        expires_at,
    })
}

pub(crate) fn current(store: &Store, c: &Connection, record: &str) -> Result<Option<Established>> {
    let row:Option<(String,String,String,Option<i64>)>=c.query_row(
        "SELECT runtime_epoch,selection_json,audience_json,restored_at FROM native_private_lifetimes WHERE operation_record_id=?1",
        [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|_|Error::storage())?;
    let Some((runtime, selection, audience, restored)) = row else {
        return Ok(None);
    };
    if runtime != store.runtime_epoch() || restored.is_some() {
        return Err(Error::forbidden());
    }
    let mut current = validate(store, c, record, &selection, &audience)?;
    let ai:bool=c.query_row("SELECT p.kind='ai' OR i.kind='ai' FROM connections v JOIN participants p ON p.participant_id=v.participant_id JOIN identities i ON i.id=v.identity_id WHERE v.id=?1",[&current.candidate],|r|r.get(0)).map_err(|_|Error::storage())?;
    if ai {
        let expiry = crate::private_provider::ready_until(c, store, record, &selection, &audience)?
            .ok_or_else(Error::forbidden)?;
        current.expires_at = current.expires_at.min(expiry);
    }
    Ok(Some(current))
}
