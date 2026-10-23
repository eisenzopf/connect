//! Target-owned isolated native preparation, independent of a caller's handoff.
//! The original signed offer, preparation receipt and native attempt already
//! retain this exact intent and physical identity; no caller-selected grant or
//! additional mutable registration/ready row is authoritative.
use super::*;

pub(crate) fn current(c: &Connection, actor: &Actor, intent: &OfferIntent) -> Result<bool> {
    let Some(marker) = intent.offer.get("handoff_preparation") else {
        return Ok(false);
    };
    if intent.offer.get("private_consultation").is_some()
        || marker.as_object().map(|o| o.len()) != Some(1)
        || marker["profile"] != "connect-handoff-preparation-v1"
    {
        return Err(Error::new(
            "unsupported-media-profile",
            "Unknown native handoff preparation profile",
        ));
    }
    actor.require("connection.offer")?;
    actor.require("session.handoff")?;
    let streams = intent.offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::forbidden)?;
    if streams.len() != 1 || streams[0]["kind"] != "audio" || streams[0]["direction"] != "sendrecv"
    {
        return Err(Error::new(
            "unsupported-media-profile",
            "Handoff preparation requires one duplex audio stream",
        ));
    }
    let request = PrepareRequest::new(intent.clone(), "scope-only".into());
    own(c, actor, &request)?;
    crate::audience::require_original(
        c,
        actor,
        &intent.conversation_id,
        ContentKind::Session,
        &intent.session_id,
    )?;
    Ok(true)
}

/// Connection metadata only. Original preparation events still have their own
/// target-only recipients; a captured handoff never republishes those events.
pub(crate) fn visible(c: &Connection, actor: &Actor, connection: &str) -> Result<bool> {
    let cid: Option<String> = c.query_row("SELECT s.conversation_id FROM connections v JOIN sessions s ON s.id=v.session_id JOIN connection_offers m ON m.connection_id=v.id WHERE v.id=?1 AND json_extract(m.offer_json,'$.handoff_preparation.profile')='connect-handoff-preparation-v1'", [connection], |r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let Some(cid) = cid else {
        return Ok(true);
    };
    let member = match crate::audience::member(c, &actor.identity_id, &cid, true) {
        Ok(member) => member,
        Err(error) if error.code == "forbidden" => return Ok(false),
        Err(error) => return Err(error),
    };
    observation_allowed(
        c,
        connection,
        &[(member.participant_id, member.membership_epoch)],
    )
}

pub(crate) fn observation_allowed(
    c: &Connection,
    connection: &str,
    selected: &[(String, u64)],
) -> Result<bool> {
    Ok(observation_audience(c, connection)?
        .is_none_or(|allowed| selected.iter().all(|member| allowed.contains(member))))
}
/// A prepared physical source starts isolated and only gains the audience of
/// its latest confirmed handoff. Session presence alone does not include a
/// different target that is still preparing on the same Session.
pub(crate) fn observation_audience(
    c: &Connection,
    connection: &str,
) -> Result<Option<Vec<(String, u64)>>> {
    let own: Option<(String,u64,String)> = c.query_row("SELECT v.participant_id,v.membership_epoch,m.runtime_connection_id FROM connections v JOIN connection_offers m ON m.connection_id=v.id WHERE v.id=?1 AND json_extract(m.offer_json,'$.handoff_preparation.profile')='connect-handoff-preparation-v1'", [connection], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    let Some((pid, epoch, runtime)) = own else {
        return Ok(None);
    };
    let mut allowed = vec![(pid, epoch)];
    let applied: Option<(String,String,String)> = c.query_row("SELECT n.plan_json,n.selection_json,o.record_id FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.applied_json IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(n.plan_json,'$.receivers') receiver WHERE receiver.key=?1) ORDER BY o.created_at DESC,o.rowid DESC LIMIT 1", [&runtime], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    if let Some((plan, slots, record)) = applied {
        let plan: Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
        let slots: Value = serde_json::from_str(&slots).map_err(|_| Error::storage())?;
        let group = plan["private_connections"]
            .as_array()
            .ok_or_else(Error::storage)?;
        for slot in slots.as_array().ok_or_else(Error::storage)? {
            if group.contains(&slot["runtime_connection_id"])
                || slot["runtime_connection_id"] == plan["handoff"]["outgoing"]
            {
                allowed.push((
                    slot["participant_id"]
                        .as_str()
                        .ok_or_else(Error::storage)?
                        .into(),
                    slot["membership_epoch"]
                        .as_u64()
                        .ok_or_else(Error::storage)?,
                ));
            }
        }
        let owner: Option<(String,u64)> = c.query_row("SELECT m.participant_id,m.membership_epoch FROM operations o JOIN operation_members m ON m.operation_record_id=o.record_id AND m.identity_id=o.owner_id WHERE o.record_id=?1", [&record], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        allowed.extend(owner);
    }
    Ok(Some(allowed))
}
