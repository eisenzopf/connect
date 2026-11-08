//! Immutable actual replacement lineage for one established private primary.
//! Resolution chooses physical metadata only. active::validate_for must still
//! validate every current credential, key, device, membership, recipient and
//! generation under fresh authority; Core independently fences actual routes.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Successor {
    pub consultation_record_id: String,
    original_primary_connection_id: String,
    original_selection_sha256: String,
    private_expires_at: i64,
    predecessor_record_id: String,
    predecessor_capture_sha256: Option<String>,
    predecessor: Value,
    target: Value,
}
pub(crate) struct Resolved {
    pub primary: String,
    pub selection: Value,
    pub private_expires_at: Option<i64>,
}
pub(crate) struct Floor {
    pub record: String,
    pub partner: String,
    pub audience: String,
    pub candidate: bool,
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or_else(Error::storage)
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().ok_or_else(Error::storage)
}
fn native(v: &Value) -> Result<()> {
    serde_json::from_value::<active::Incarnation>(v.clone())
        .map(|_| ())
        .map_err(|_| Error::forbidden())
}

/// The preceding physical owner is anchored to its actual retained admission.
/// Later steps additionally name the immutable preceding replacement capture
/// for this same private slot. No whole-chain replay and no ID-only alias is used:
/// this record can be minted only after resolve() validates that then-current
/// predecessor under fresh authority; every next step pins its exact bytes.
fn require_predecessor(
    c: &Connection,
    lineage: &Successor,
    original: &Value,
    runtime: &str,
) -> Result<()> {
    let p = &lineage.predecessor;
    let historical:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_admission_sources n JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id AND l.connection_id=n.connection_id AND l.runtime_epoch=n.attempt_runtime AND l.generation=n.generation JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id AND e.state='confirmed' JOIN connection_offers m ON m.connection_id=n.connection_id WHERE n.operation_record_id=?1 AND n.connection_id=?2 AND n.attempt_runtime=?3 AND n.generation=?4 AND m.runtime_connection_id=?5 AND n.credential_id=?6 AND n.key_id=?7 AND n.device_id=?8 AND n.endpoint_id=?9 AND n.participant_id=?10 AND n.membership_epoch=?11 AND n.state='completed')",
        params![lineage.predecessor_record_id,text(p,"connection_id")?,runtime,number(p,"generation")?,text(p,"runtime_connection_id")?,text(p,"credential_id")?,text(p,"key_id")?,text(p,"device_id")?,text(p,"endpoint_id")?,text(p,"participant_id")?,number(p,"membership_epoch")?],|r|r.get(0),
    ).map_err(|_|Error::storage())?;
    if !historical {
        return Err(Error::forbidden());
    }
    if p["connection_id"] == lineage.original_primary_connection_id {
        if p != original || lineage.predecessor_capture_sha256.is_some() {
            return Err(Error::forbidden());
        }
        return Ok(());
    }
    let captured:Option<String>=c.query_row(
        "SELECT s.observation_json FROM native_replacement_steps s JOIN native_replacements n ON n.operation_record_id=s.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed' JOIN native_replacement_lifetimes l ON l.operation_record_id=n.operation_record_id AND l.attempt_runtime=n.runtime_epoch AND l.generation=n.generation AND l.attempt_id=s.attempt_id AND l.effect_id=s.effect_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.candidate_connection_id=?3 AND n.candidate_runtime_id=?4 AND n.generation=?5 AND s.phase='commit' AND s.started=1 AND json_extract(o.result_json,'$.activation_state')='ready'",
        params![lineage.predecessor_record_id,runtime,text(p,"connection_id")?,text(p,"runtime_connection_id")?,number(p,"generation")?],|r|r.get(0),
    ).optional().map_err(|_|Error::storage())?;
    let captured = captured.ok_or_else(Error::forbidden)?;
    if lineage.predecessor_capture_sha256.as_deref() != Some(hash(&captured).as_str()) {
        return Err(Error::forbidden());
    }
    let captured: Value = serde_json::from_str(&captured).map_err(|_| Error::storage())?;
    let prior: Successor =
        serde_json::from_value(captured["projection"]["private_successor"].clone())
            .map_err(|_| Error::forbidden())?;
    if prior.consultation_record_id != lineage.consultation_record_id
        || prior.original_primary_connection_id != lineage.original_primary_connection_id
        || prior.original_selection_sha256 != lineage.original_selection_sha256
        || prior.target != lineage.predecessor
        || prior.private_expires_at < lineage.private_expires_at
    {
        return Err(Error::forbidden());
    }
    Ok(())
}

/// Return a copy with the latest sealed successor for the same original slot.
/// No authority is granted by this copy. Historical rows are never rewritten.
pub(crate) fn resolve(
    c: &Connection,
    record: &str,
    selected: &str,
    primary: &str,
    runtime: &str,
) -> Result<Resolved> {
    let mut selection: Value = serde_json::from_str(selected).map_err(|_| Error::storage())?;
    let original = selection["private"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .find(|p| p["connection_id"] == primary)
        .cloned()
        .ok_or_else(Error::forbidden)?;
    let mut statement = c.prepare_cached(
        "SELECT s.observation_json,n.plan_json,n.candidate_connection_id,n.candidate_runtime_id,n.generation,
         l.credential_id,l.key_id,l.device_id,l.endpoint_id,l.participant_id,l.membership_epoch,l.source_expires_at,o.connection_id
         FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed'
         JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' AND s.started=1
         JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1
         JOIN native_replacement_lifetimes l ON l.operation_record_id=o.record_id AND l.attempt_runtime=n.runtime_epoch AND l.attempt_id=s.attempt_id AND l.effect_id=e.id AND l.generation=n.generation
         WHERE n.runtime_epoch=?1 AND json_extract(o.result_json,'$.activation_state')='ready'
         AND json_extract(s.observation_json,'$.projection.private_successor.consultation_record_id')=?2
         ORDER BY n.generation DESC LIMIT 2",
    ).map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![runtime, record], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, u64>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, String>(12)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    if rows.len() == 2 && rows[0].4 == rows[1].4 {
        return Err(Error::forbidden());
    }
    let Some((
        captured,
        plan,
        connection,
        core,
        generation,
        credential,
        key,
        device,
        endpoint,
        participant,
        epoch,
        expiry,
        predecessor,
    )) = rows.into_iter().next()
    else {
        return Ok(Resolved {
            primary: primary.into(),
            selection,
            private_expires_at: None,
        });
    };
    let captured: Value = serde_json::from_str(&captured).map_err(|_| Error::storage())?;
    let lineage: Successor =
        serde_json::from_value(captured["projection"]["private_successor"].clone())
            .map_err(|_| Error::storage())?;
    let plan: Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
    native(&original)?;
    native(&lineage.predecessor)?;
    native(&lineage.target)?;
    if lineage.consultation_record_id != record
        || lineage.original_primary_connection_id != primary
        || lineage.original_selection_sha256 != hash(selected)
        || lineage.predecessor["connection_id"] != predecessor
        || lineage.predecessor["generation"] != plan["source_generation"]
        || lineage.predecessor["runtime_connection_id"] != plan["expected_preparation"]["source"]
        || lineage.target["connection_id"] != connection
        || lineage.target["runtime_connection_id"] != core
        || lineage.target["generation"] != generation
        || lineage.target["credential_id"] != credential
        || lineage.target["key_id"] != key
        || lineage.target["device_id"] != device
        || lineage.target["endpoint_id"] != endpoint
        || lineage.target["participant_id"] != participant
        || lineage.target["membership_epoch"] != epoch
        || lineage.target["physical_expires_at"] != expiry
        || lineage.target["device_kind"] != plan["target_device_kind"]
        || lineage.target["device_platform"] != plan["target_device_platform"]
        || lineage.predecessor["participant_id"] != original["participant_id"]
        || lineage.predecessor["membership_epoch"] != original["membership_epoch"]
        || lineage.target["participant_id"] != original["participant_id"]
        || lineage.target["membership_epoch"] != original["membership_epoch"]
        || number(&lineage.predecessor, "generation")?.checked_add(1) != Some(generation)
        || captured["movement"]["candidate"] != core
        || captured["movement"]["generation"] != generation
        || lineage.private_expires_at <= 0
        || [&original, &lineage.predecessor, &lineage.target]
            .iter()
            .any(|slot| {
                slot["physical_expires_at"]
                    .as_i64()
                    .is_none_or(|expiry| lineage.private_expires_at > expiry)
            })
    {
        return Err(Error::forbidden());
    }
    require_predecessor(c, &lineage, &original, runtime)?;
    let slot = selection["private"]
        .as_array_mut()
        .ok_or_else(Error::storage)?
        .iter_mut()
        .find(|p| p["connection_id"] == primary)
        .ok_or_else(Error::forbidden)?;
    *slot = lineage.target;
    Ok(Resolved {
        primary: connection,
        selection,
        private_expires_at: Some(lineage.private_expires_at),
    })
}

/// Metadata resolution for a return, including an applied consultation whose
/// media wait has not yet produced an established lifetime. The caller still
/// verifies its original return intent and fresh physical/purpose authority.
pub(crate) fn primary_for_return(c: &Connection, record: &str, runtime: &str) -> Result<String> {
    let (selected,primary,original_runtime):(String,String,String)=c.query_row(
        "SELECT n.selection_json,b.primary_connection_id,n.runtime_epoch FROM native_consultation_attempts n JOIN consultation_candidate_bindings b ON b.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.started=1 AND n.applied_json IS NOT NULL",
        [record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_|Error::storage())?;
    if original_runtime != runtime {
        return Err(Error::forbidden());
    }
    Ok(resolve(c, record, &selected, &primary, runtime)?.primary)
}

fn established(c: &Connection, record: &str) -> Result<Option<(String, String, String)>> {
    c.query_row("SELECT v.selection_json,b.primary_connection_id,v.runtime_epoch FROM native_private_lifetimes v JOIN consultation_candidate_bindings b ON b.operation_record_id=v.operation_record_id WHERE v.operation_record_id=?1 AND v.restored_at IS NULL", [record], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| Error::storage())
}
// Restriction-only ancestry: absence of a valid successor capture must never
// erase an existing private floor. UNION visits each retained physical ID once,
// even for corrupt cyclic input; this does not grant a partner or new audience.
fn has_private_ancestor(c: &Connection, connection: &str) -> Result<bool> {
    c.prepare_cached(
        "WITH RECURSIVE ancestry(id) AS (SELECT ?1 UNION SELECT o.connection_id FROM ancestry a JOIN native_replacements n ON n.candidate_connection_id=a.id JOIN operations o ON o.record_id=n.operation_record_id WHERE o.connection_id IS NOT NULL) SELECT EXISTS(SELECT 1 FROM ancestry a JOIN native_private_content_floors f ON f.connection_id=a.id)",
    )
    .map_err(|_| Error::storage())?
    .query_row([connection], |r| r.get(0))
    .map_err(|_| Error::storage())
}
fn context_consumed(c: &Connection, record: &str) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.consultation_record_id=?1 AND r.applied_json IS NOT NULL) OR EXISTS(SELECT 1 FROM native_handoffs h JOIN operations o ON o.record_id=h.operation_record_id JOIN operation_effects e ON e.id=h.effect_id AND e.state='confirmed' WHERE h.applied_json IS NOT NULL AND json_extract(o.intent_json,'$.payload_extensions.consultation_record_id')=?1)",[record],|r|r.get(0)).map_err(|_|Error::storage())
}
pub(crate) fn floor(c: &Connection, connection: &str) -> Result<Option<Floor>> {
    let direct: Option<(String,String,String,bool)> = c.prepare_cached("SELECT operation_record_id,partner_connection_id,audience_json,is_candidate FROM native_private_content_floors WHERE connection_id=?1").map_err(|_| Error::storage())?
        .query_row([connection], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_| Error::storage())?;
    if let Some((record, mut partner, audience, candidate)) = direct {
        if candidate {
            if let Some((selection, primary, runtime)) = established(c, &record)? {
                partner = resolve(c, &record, &selection, &primary, &runtime)?.primary;
            }
        }
        return Ok(Some(Floor {
            record,
            partner,
            audience,
            candidate,
        }));
    }
    // A successor must name an existing still-private primary floor. No
    // arbitrary `connections.replaces`, signed offer field or receipt alone
    // introduces a new private audience.
    let mut st = c.prepare_cached("SELECT DISTINCT json_extract(s.observation_json,'$.projection.private_successor.consultation_record_id') FROM native_replacement_steps s JOIN operations o ON o.record_id=s.operation_record_id AND o.state='completed' JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE s.phase='commit' AND s.started=1 AND n.candidate_connection_id=?1 AND json_extract(o.result_json,'$.activation_state')='ready' AND json_type(s.observation_json,'$.projection.private_successor')='object' LIMIT 2").map_err(|_| Error::storage())?;
    let records = st
        .query_map([connection], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    if records.len() > 1 {
        return Err(Error::forbidden());
    }
    let Some(record) = records.first() else {
        return if has_private_ancestor(c, connection)? {
            Err(Error::forbidden())
        } else {
            Ok(None)
        };
    };
    let Some((selection, primary, runtime)) = established(c, record)? else {
        return if context_consumed(c, record)? {
            Ok(None)
        } else {
            Err(Error::forbidden())
        };
    };
    let resolved = resolve(c, record, &selection, &primary, &runtime)?;
    let floor: Option<(String,String)> = c.query_row("SELECT partner_connection_id,audience_json FROM native_private_content_floors WHERE operation_record_id=?1 AND connection_id=?2 AND is_candidate=0", params![record,primary], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| Error::storage())?;
    let Some((partner, audience)) = floor else {
        return if context_consumed(c, record)? {
            Ok(None)
        } else {
            Err(Error::forbidden())
        };
    };
    // An older successor is still constrained by its inherited private floor;
    // its independent current native generation check will refuse stale IO.
    let _ = resolved;
    Ok(Some(Floor {
        record: record.clone(),
        partner,
        audience,
        candidate: false,
    }))
}

pub(crate) fn capture(
    store: &Store,
    c: &Connection,
    source: &str,
    retained: &str,
    target: &crate::PeerConnectionProjection,
    descriptor: &Value,
    expires_at: i64,
    held: &crate::AuthorityGuardSet,
) -> Result<Option<Successor>> {
    let Some(floor) = floor(c, source)? else {
        let held_remote:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_private_lifetimes v WHERE v.runtime_epoch=?1 AND v.restored_at IS NULL AND json_extract(v.selection_json,'$.remote.connection_id')=?2)", params![store.runtime_epoch(),source], |r|r.get(0)).map_err(|_|Error::storage())?;
        if held_remote {
            return Err(Error::new(
                "unsupported-media-profile",
                "Replacing a held private remote requires its own restoration contract",
            ));
        }
        return Ok(None);
    };
    if floor.candidate || floor.partner != retained {
        return Err(Error::new("unsupported-media-profile", "Only the established private primary and its exact retained partner can transfer this slot"));
    }
    let (selected, primary, runtime) =
        established(c, &floor.record)?.ok_or_else(Error::forbidden)?;
    if runtime != store.runtime_epoch() {
        return Err(Error::forbidden());
    }
    let current = active::current(store, c, &floor.record)?.ok_or_else(Error::forbidden)?;
    if current.primary != source
        || current.candidate != retained
        || !store.authority_covers(held, &current.scopes)
    {
        return Err(Error::forbidden());
    }
    let resolved = resolve(c, &floor.record, &selected, &primary, &runtime)?;
    let predecessor = resolved.selection["private"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .find(|p| p["connection_id"] == source)
        .cloned()
        .ok_or_else(Error::forbidden)?;
    native(&predecessor)?;
    let (kind, platform): (String, String) = c
        .query_row(
            "SELECT kind,platform FROM devices WHERE id=?1 AND identity_id=?2 AND revoked=0",
            params![target.device_id, target.session.actor.identity_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    let public_key: Vec<u8> =
        serde_json::from_value(descriptor["public_key"].clone()).map_err(|_| Error::storage())?;
    let target = json!({"connection_id":target.connection.connection_id,"runtime_connection_id":target.runtime_connection_id,
        "participant_id":target.connection.participant_id,"membership_epoch":target.connection.membership_epoch,
        "generation":target.connection.generation,"credential_id":text(descriptor,"credential_id")?,"key_id":text(descriptor,"key_id")?,
        "key_algorithm":text(descriptor,"algorithm")?,"public_key_sha256":hex::encode(Sha256::digest(public_key)),
        "device_id":target.device_id,"device_kind":kind,"device_platform":platform,"endpoint_id":target.connection.endpoint_id,"physical_expires_at":expires_at});
    native(&target)?;
    if number(&predecessor, "generation")?.checked_add(1) != Some(number(&target, "generation")?)
        || predecessor["participant_id"] != target["participant_id"]
        || predecessor["membership_epoch"] != target["membership_epoch"]
    {
        return Err(Error::forbidden());
    }
    let predecessor_record_id:String=c.query_row("SELECT operation_record_id FROM native_connection_lifetimes WHERE connection_id=?1 AND runtime_epoch=?2 AND generation=?3 AND retired=0",params![source,runtime,number(&predecessor,"generation")?],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    let predecessor_capture_sha256 = if source == primary {
        None
    } else {
        let captured:String=c.query_row("SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1",[&predecessor_record_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        Some(hash(&captured))
    };
    let lineage = Successor {
        consultation_record_id: floor.record,
        original_primary_connection_id: primary.clone(),
        original_selection_sha256: hash(&selected),
        private_expires_at: current.expires_at.min(expires_at),
        predecessor_record_id,
        predecessor_capture_sha256,
        predecessor,
        target,
    };
    let original: Value = serde_json::from_str(&selected).map_err(|_| Error::storage())?;
    let original = original["private"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .find(|p| p["connection_id"] == primary)
        .ok_or_else(Error::forbidden)?;
    require_predecessor(c, &lineage, original, &runtime)?;
    Ok(Some(lineage))
}

pub(crate) fn require_published(
    store: &Store,
    c: &Connection,
    lineage: &Successor,
    held: &crate::AuthorityGuardSet,
) -> Result<()> {
    let current =
        active::current(store, c, &lineage.consultation_record_id)?.ok_or_else(Error::forbidden)?;
    if current.primary != text(&lineage.target, "connection_id")?
        || !store.authority_covers(held, &current.scopes)
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
