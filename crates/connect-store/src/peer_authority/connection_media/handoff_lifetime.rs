//! Current audio authority after an attributed native handoff, never an old
//! execution grant. Private provenance remains retained for every other use.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub(super) struct Current {
    pub record: String,
    pub pairs: BTreeSet<(String, String)>,
    pub scopes: Vec<AuthorityScope>,
    pub expires_at: i64,
    pub role: Option<String>,
}
/// Recorded membership only, used to require closure of an audience being
/// consumed. This neither issues a Core handle nor supplies media permission.
pub(super) fn previous_members(
    store: &Store,
    c: &Connection,
    session: &str,
    source_runtime: &str,
) -> Result<BTreeSet<String>> {
    let plan: Option<String> = c.query_row(
        "SELECT n.plan_json FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.runtime_epoch=?1 AND o.session_id=?2 AND n.applied_json IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(n.plan_json,'$.private_connections') g WHERE g.value=?3) ORDER BY json_extract(n.applied_json,'$.audience_revision') DESC,o.rowid DESC LIMIT 1",
        params![store.runtime_epoch(),session,source_runtime],|r|r.get(0)
    ).optional().map_err(|_|Error::storage())?;
    let Some(plan) = plan else {
        return Ok(BTreeSet::new());
    };
    let plan: serde_json::Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
    plan["private_connections"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or_else(Error::storage))
        .collect()
}
pub(super) fn current(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<Option<Current>> {
    let saved:Option<(String,String,String,String,Option<String>,String,Option<String>)>=c.query_row(
        "SELECT n.operation_record_id,o.intent_json,n.plan_json,n.selection_json,n.applied_json,o.operation_id,o.result_json FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state IN ('submitting','unknown','confirmed') WHERE n.runtime_epoch=?1 AND n.started=1 AND o.state IN ('preparing','unknown','completed') AND EXISTS(SELECT 1 FROM json_each(n.selection_json) s WHERE json_extract(s.value,'$.connection_id')=?2 AND (json_extract(s.value,'$.participant_id')=json_extract(o.intent_json,'$.intent.source_participant') OR json_extract(s.value,'$.runtime_connection_id') IN (json_extract(n.plan_json,'$.handoff.incoming'),json_extract(n.plan_json,'$.handoff.retained')))) ORDER BY o.created_at DESC,o.rowid DESC LIMIT 1",
        params![store.runtime_epoch(),origin.connection_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional().map_err(|_|Error::storage())?;
    let Some((record, request, plan, selected, observed, operation, result)) = saved else {
        return Ok(None);
    };
    let Some(observed) = observed else {
        return Err(Error::new(
            "handoff-pending",
            "Native handoff outcome is not yet attributed",
        ));
    };
    let mut request: serde_json::Value =
        serde_json::from_str(&request).map_err(|_| Error::storage())?;
    request["operation_id"] = serde_json::json!(operation);
    let request: connect_protocol::transitions::TransitionRequest =
        serde_json::from_value(request).map_err(|_| Error::storage())?;
    let connect_protocol::transitions::TransitionIntent::Handoff {
        source_participant,
        target_participant,
        outgoing,
        ..
    } = &request.intent
    else {
        return Err(Error::storage());
    };
    let plan: serde_json::Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
    let selected: serde_json::Value =
        serde_json::from_str(&selected).map_err(|_| Error::storage())?;
    let observed: serde_json::Value =
        serde_json::from_str(&observed).map_err(|_| Error::storage())?;
    let group = plan["private_connections"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .map(|v| v.as_str().ok_or_else(Error::storage))
        .collect::<Result<BTreeSet<_>>>()?;
    if !group.contains(origin.runtime_connection_id()) {
        return Err(Error::forbidden());
    }
    if result.is_none() {
        let unchanged:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND topology_revision=?2 AND state IN ('accepted','ready','active'))",params![request.session_id,request.expected_revision],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !unchanged {
            return Err(Error::forbidden());
        }
    }
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &request.conversation_id,
        AuthorityMode::Read,
    )?;
    let mut expires_at = crate::now() + 5;
    let mut bindings = BTreeMap::new();
    for slot in selected.as_array().ok_or_else(Error::storage)? {
        let runtime = slot["runtime_connection_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        if group.contains(runtime) {
            bindings.insert(
                runtime.to_owned(),
                slot["connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?
                    .to_owned(),
            );
        }
    }
    let mut role_override = None;
    let mut found = false;
    let parent = request
        .payload_extensions
        .get("consultation_record_id")
        .and_then(|v| v.as_str());
    for slot in selected.as_array().ok_or_else(Error::storage)? {
        let runtime = slot["runtime_connection_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        if !group.contains(runtime) || slot["connection_id"] != origin.connection_id() {
            continue;
        }
        found = true;
        let id = slot["connection_id"].as_str().ok_or_else(Error::storage)?;
        let pid = slot["participant_id"].as_str().ok_or_else(Error::storage)?;
        let key = slot["credential_id"].as_str().ok_or_else(Error::storage)?;
        let physical:Option<(String,String,i64,Vec<u8>,String)>=c.query_row(
            "SELECT v.identity_id,sp.role,k.expires_at,k.public_key,n.operation_record_id FROM connections v JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 JOIN native_admission_sources n ON n.operation_record_id=l.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed' JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' JOIN connection_offers m ON m.connection_id=v.id JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1 JOIN session_participants sp ON sp.session_id=v.session_id AND sp.participant_id=v.participant_id AND sp.membership_epoch=v.membership_epoch AND sp.state='present' JOIN endpoints ep ON ep.id=v.endpoint_id AND ep.verification_state='verified' AND ep.device_id=n.device_id JOIN devices d ON d.id=ep.device_id AND d.identity_id=v.identity_id AND d.revoked=0 JOIN credentials cr ON cr.id=n.credential_id AND cr.identity_id=v.identity_id AND cr.revoked=0 AND cr.expires_at>?9 JOIN peer_signing_keys k ON k.key_id=n.key_id AND k.identity_id=v.identity_id AND k.device_id=d.id AND k.revoked=0 AND k.not_before<=?9 AND k.expires_at>?9 JOIN credential_signing_keys b ON b.credential_id=cr.id AND b.key_id=k.key_id JOIN identities i ON i.id=v.identity_id AND i.revoked=0 JOIN sessions sess ON sess.id=v.session_id AND sess.state IN ('accepted','ready','active') JOIN conversations room ON room.id=sess.conversation_id AND room.closed=0 WHERE v.id=?1 AND v.session_id=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND v.generation=?5 AND v.observed_runtime_id=?6 AND m.runtime_connection_id=?7 AND n.credential_id=?8 AND v.state IN ('ready','active') AND ep.id=?10 AND d.id=?11 AND d.kind=?12 AND d.platform=?13 AND k.key_id=?14 AND k.algorithm=?15",
            params![id,request.session_id,pid,slot["membership_epoch"].as_u64().ok_or_else(Error::storage)?,slot["generation"].as_u64().ok_or_else(Error::storage)?,store.runtime_epoch(),runtime,key,crate::now(),slot["endpoint_id"].as_str().ok_or_else(Error::storage)?,slot["device_id"].as_str().ok_or_else(Error::storage)?,slot["device_kind"].as_str().ok_or_else(Error::storage)?,slot["device_platform"].as_str().ok_or_else(Error::storage)?,slot["key_id"].as_str().ok_or_else(Error::storage)?,slot["key_algorithm"].as_str().ok_or_else(Error::storage)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(|_|Error::storage())?;
        let (identity, presence, key_expiry, public_key, offer) =
            physical.ok_or_else(Error::forbidden)?;
        if slot["public_key_sha256"] != hex::encode(Sha256::digest(&public_key)) {
            return Err(Error::forbidden());
        }
        let other_floor:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_private_content_floors WHERE connection_id=?1 AND operation_record_id IS NOT ?2)",params![id,parent],|r|r.get(0)).map_err(|_|Error::storage())?;
        if other_floor {
            return Err(Error::forbidden());
        }
        let actor = crate::identity::actor_from_connection(c, key)?;
        actor.require("connection.offer")?;
        if pid == *target_participant {
            actor.require("session.handoff")?;
            if request
                .payload_extensions
                .get("policy_ref")
                .and_then(|v| v.as_str())
                != Some(offer.as_str())
            {
                return Err(Error::forbidden());
            }
        }
        let desired = if pid == *source_participant {
            if *outgoing != connect_protocol::transitions::OutgoingDisposition::Observe {
                return Err(Error::forbidden());
            }
            actor.require("session.observation")?;
            Some("observer".to_owned())
        } else if pid == *target_participant {
            Some(
                observed["participant_handoff"]["incoming_role"]
                    .as_str()
                    .ok_or_else(Error::storage)?
                    .to_owned(),
            )
        } else {
            None
        };
        expires_at = expires_at.min(actor.expires_at).min(key_expiry).min(
            slot["physical_expires_at"]
                .as_i64()
                .ok_or_else(Error::storage)?,
        );
        scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Read)?);
        if id == origin.connection_id() && returns::describe(origin) != *slot {
            return Err(Error::forbidden());
        }
        let _ = presence;
        if result.is_none() {
            role_override = desired;
        }
    }
    if !found || bindings.len() != group.len() || crate::now() >= expires_at {
        return Err(Error::forbidden());
    }
    let mut pairs = BTreeSet::new();
    for (receiver, sources) in plan["receivers"].as_object().ok_or_else(Error::storage)? {
        let Some(target) = bindings.get(receiver) else {
            continue;
        };
        for source in sources.as_array().ok_or_else(Error::storage)? {
            let publisher = bindings
                .get(source.as_str().ok_or_else(Error::storage)?)
                .ok_or_else(Error::forbidden)?;
            pairs.insert((target.clone(), publisher.clone()));
        }
    }
    Ok(Some(Current {
        record,
        pairs,
        scopes: AuthorityScope::merged(&scopes)?,
        expires_at,
        role: role_override,
    }))
}
