use super::*;
pub(super) struct Facts {
    pub actor: Actor,
    pub delegate: Actor,
    pub candidate: String,
    pub primary: String,
    pub conversation: String,
    pub audience: Vec<(String, String, u64)>,
    pub audience_json: String,
    pub endpoint: Value,
    pub expires_at: i64,
}
pub(super) fn resolve(
    store: &Store,
    c: &Connection,
    context: &PrivateProviderContext,
) -> Result<Facts> {
    let origin = &context.selection.origin;
    let closed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts WHERE delivery_id=?1 AND closed_at IS NOT NULL)",[&context.delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
    if closed {
        return Err(Error::forbidden());
    }

    let owner = store.require_runtime_owner()?;
    if origin.runtime != store.runtime_epoch()
        || !owner.current()
        || !origin
            .owner
            .upgrade()
            .is_some_and(|p| Arc::ptr_eq(&owner, &p))
    {
        return Err(Error::forbidden());
    }
    let (candidate,primary,audience_json):(String,String,String)=c.query_row("SELECT connection_id,primary_connection_id,audience_json FROM native_consultation_legs WHERE operation_record_id=?1 AND original_digest=?2 AND runtime_epoch=?3",params![origin.record,origin.digest,origin.runtime],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::forbidden())?;
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(&audience_json).map_err(|_| Error::storage())?;
    if audience.len() != 2 {
        return Err(Error::forbidden());
    }
    let snapshot = context.selection.snapshot();
    let candidate_source = snapshot["private"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .find(|p| p["connection_id"] == candidate)
        .ok_or_else(Error::forbidden)?;
    let delegate_id = candidate_source["credential_id"]
        .as_str()
        .ok_or_else(Error::storage)?;
    let actor = crate::identity::actor_from_connection(c, &context.context_credential)?;
    actor.require("context_view")?;
    let delegate = crate::identity::actor_from_connection(c, delegate_id)?;
    delegate.require("context_view")?;
    let own =
        crate::audience::member(c, &actor.identity_id, &origin.request.conversation_id, true)?;
    let peer = crate::audience::member(
        c,
        &delegate.identity_id,
        &origin.request.conversation_id,
        true,
    )?;
    if !audience.contains(&(
        actor.identity_id.clone(),
        own.participant_id.clone(),
        own.membership_epoch,
    )) || !audience.contains(&(
        delegate.identity_id.clone(),
        peer.participant_id.clone(),
        peer.membership_epoch,
    )) || candidate_source["participant_id"] != peer.participant_id
        || candidate_source["membership_epoch"] != peer.membership_epoch
        || audience
            .iter()
            .any(|(id, _, _)| !actor.permits_recipient(id) || !delegate.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    let ai:bool=c.query_row("SELECT p.kind='ai' AND i.kind='ai' FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.participant_id=?1 AND p.membership_epoch=?2",params![peer.participant_id,peer.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !ai {
        return Err(Error::new(
            "unsupported-media-profile",
            "Private provider context requires a configured AI Participant",
        ));
    }
    let (revision,address,secret,device,device_kind,device_platform):(u64,String,String,String,String,String)=c.query_row("SELECT e.revision,e.address,e.secret_ref,e.device_id,d.kind,d.platform FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 JOIN identities i ON i.id=e.identity_id AND i.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='vapi-voice' AND e.verification_state='verified' AND e.secret_ref IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio') AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='private-consultation')",params![context.endpoint,delegate.identity_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let endpoint = json!({"id":context.endpoint,"revision":revision,"address":address,"secret_ref":secret,"device_id":device,"device_kind":device_kind,"device_platform":device_platform});
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let source_expiry = if state == "completed" {
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_private_lifetimes l JOIN native_consultation_attempts n ON n.operation_record_id=l.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE l.operation_record_id=?1 AND l.restored_at IS NULL AND n.applied_json IS NOT NULL)",[origin.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current {
            return Err(Error::forbidden());
        }
        let selected:String=c.query_row("SELECT selection_json FROM native_consultation_attempts WHERE operation_record_id=?1",[origin.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        let selected_value: Value =
            serde_json::from_str(&selected).map_err(|_| Error::storage())?;
        if selected_value["private"] != snapshot["private"]
            || selected_value["remote"] != snapshot["remote"]
        {
            return Err(Error::forbidden());
        }
        crate::connection_control::private_consultation::active::validate(
            store,
            c,
            origin.record_id(),
            &selected,
            &audience_json,
        )?
        .expires_at
    } else {
        context.selection.scopes(store, c, false)?;
        let (source, _) = origin.require(store, c)?;
        source.expires_at.min(origin.request.expires_at)
    };
    let expires_at = source_expiry
        .min(crate::now() + 5)
        .min(actor.expires_at)
        .min(delegate.expires_at);
    Ok(Facts {
        actor,
        delegate,
        candidate,
        primary,
        conversation: origin.request.conversation_id.clone(),
        audience,
        audience_json,
        endpoint,
        expires_at,
    })
}
pub(super) fn scopes(
    store: &Store,
    c: &Connection,
    context: &PrivateProviderContext,
    write: bool,
) -> Result<Vec<AuthorityScope>> {
    let facts = resolve(store, c, context)?;
    let state: String = c
        .query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [context.consultation_record_id()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut scopes = if state == "completed" {
        crate::connection_control::private_consultation::active::validate(
            store,
            c,
            context.consultation_record_id(),
            &crate::provisioning::canonical_json(&context.selection.snapshot())?,
            &facts.audience_json,
        )?
        .scopes
    } else {
        context.selection.scopes(store, c, write)?
    };
    scopes.push(AuthorityScope::identity(
        &facts.actor.identity_id,
        if write {
            AuthorityMode::Write
        } else {
            AuthorityMode::Read
        },
    )?);
    scopes.push(AuthorityScope::identity(
        &facts.delegate.identity_id,
        AuthorityMode::Read,
    )?);
    AuthorityScope::merged(&scopes)
}
pub(super) fn bound(store: &Store, c: &Connection, context: &PrivateProviderContext) -> Result<()> {
    let facts = resolve(store, c, context)?;
    let matched:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts WHERE delivery_id=?1 AND operation_record_id=?2 AND attempt_id=?3 AND context_credential_id=?4 AND endpoint_id=?5 AND runtime_epoch=?6 AND endpoint_json=?7 AND selection_json=?8 AND source_digest=?9 AND delegate_credential_id=?10 AND audience_json=?11)",params![context.delivery,context.consultation_record_id(),context.attempt,context.context_credential,context.endpoint,store.runtime_epoch(),crate::provisioning::canonical_json(&facts.endpoint)?,crate::provisioning::canonical_json(&context.selection.snapshot())?,context.selection.origin.digest,facts.delegate.credential_id,facts.audience_json],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !matched {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn progress(c: &Connection, root: &str, store: &Store) -> Result<()> {
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Private context journal capacity exceeded",
        ));
    }
    let (owner,cid,sid,operation,pid,epoch,state,revision):(String,String,String,String,String,u64,String,u64)=c.query_row("SELECT o.owner_id,o.conversation_id,o.session_id,o.operation_id,m.participant_id,m.membership_epoch,o.state,o.revision FROM operations o JOIN operation_members m ON m.operation_record_id=o.record_id AND m.identity_id=o.owner_id WHERE o.record_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|_|Error::storage())?;
    if revision >= 9_007_199_254_740_990 {
        return Err(Error::new("conflict", "Operation revision exhausted"));
    }
    c.execute(
        "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
        params![root, crate::now()],
    )
    .map_err(|_| Error::storage())?;
    let data =
        json!({"source_kind":"session","source_id":sid,"operation_id":operation,"state":state});
    c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'operation_updated',?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4()),owner,cid,root,revision+1,data.to_string(),pid,epoch]).map_err(|_|Error::storage())?;
    Ok(())
}

/// Called only under the actual media completion/plan fence. Initialization is
/// independent evidence; kind labels and decoded media cannot manufacture it.
pub(super) fn uncached_ready_until(
    c: &Connection,
    store: &Store,
    root: &str,
    selected: &str,
    audience: &str,
) -> Result<Option<i64>> {
    let saved:Option<(String,String,String,String,String,String,String)>=c.query_row("SELECT p.connection_id,p.selection_json,p.audience_json,p.context_credential_id,p.delegate_credential_id,p.endpoint_id,p.endpoint_json FROM private_provider_contexts p JOIN context_deliveries d ON d.id=p.delivery_id AND d.state='delivered' AND d.provider='vapi' AND d.provider_ref=p.provider_ref WHERE p.operation_record_id=?1 AND p.runtime_epoch=?2 AND p.started=1 AND p.provider_ref IS NOT NULL AND p.delivered_at IS NOT NULL AND p.closed_at IS NULL",params![root,store.runtime_epoch()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|Error::storage())?;
    let Some((
        candidate,
        original,
        original_audience,
        credential,
        delegate,
        endpoint,
        endpoint_json,
    )) = saved
    else {
        return Ok(None);
    };
    if original_audience != audience {
        return Ok(None);
    }
    let original: Value = serde_json::from_str(&original).map_err(|_| Error::storage())?;
    let selected: Value = serde_json::from_str(selected).map_err(|_| Error::storage())?;
    if original["private"] != selected["private"] || original["remote"] != selected["remote"] {
        return Ok(None);
    }
    let endpoint_json: Value =
        serde_json::from_str(&endpoint_json).map_err(|_| Error::storage())?;
    let (Ok(actor), Ok(delegate)) = (
        crate::identity::actor_from_connection(c, &credential),
        crate::identity::actor_from_connection(c, &delegate),
    ) else {
        return Ok(None);
    };
    if !actor.permits("context_view") || !delegate.permits("context_view") {
        return Ok(None);
    }
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(audience).map_err(|_| Error::storage())?;
    let mut owner = false;
    let mut peer = false;
    for (identity, pid, epoch) in &audience {
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM participants WHERE participant_id=?1 AND identity_id=?2 AND membership_epoch=?3 AND active=1)",params![pid,identity,epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current || !actor.permits_recipient(identity) || !delegate.permits_recipient(identity) {
            return Ok(None);
        }
        owner |= identity == &actor.identity_id;
        peer |= identity == &delegate.identity_id;
    }
    if !owner || !peer {
        return Ok(None);
    }
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 JOIN identities i ON i.id=e.identity_id AND i.revoked=0 JOIN connections v ON v.identity_id=e.identity_id JOIN participants p ON p.participant_id=v.participant_id AND p.identity_id=v.identity_id AND p.active=1 AND p.kind='ai' WHERE e.id=?1 AND e.identity_id=?2 AND e.verification_state='verified' AND e.transport='vapi-voice' AND e.revision=?3 AND e.device_id=?4 AND e.address=?5 AND e.secret_ref=?6 AND v.id=?7 AND d.kind=?8 AND d.platform=?9 AND i.kind='ai' AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio') AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='private-consultation'))",params![endpoint,delegate.identity_id,endpoint_json["revision"].as_u64().ok_or_else(Error::storage)?,endpoint_json["device_id"].as_str().ok_or_else(Error::storage)?,endpoint_json["address"].as_str().ok_or_else(Error::storage)?,endpoint_json["secret_ref"].as_str().ok_or_else(Error::storage)?,candidate,endpoint_json["device_kind"].as_str().ok_or_else(Error::storage)?,endpoint_json["device_platform"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Ok(None);
    }
    let (encoded,cid):(String,String)=c.query_row("SELECT d.selection_json,d.conversation_id FROM context_deliveries d JOIN private_provider_contexts p ON p.delivery_id=d.id WHERE p.operation_record_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let view: ContextView = serde_json::from_str(&encoded).map_err(|_| Error::storage())?;
    match crate::context::authorize_private_export_events(
        c,
        &actor,
        &delegate.identity_id,
        Some(&delegate),
        &cid,
        &view.items.iter().map(|i| i.sequence).collect::<Vec<_>>(),
        Some(root),
    ) {
        Ok(()) => {}
        Err(e) if matches!(e.code.as_str(), "unauthorized" | "forbidden" | "gone") => {
            return Ok(None)
        }
        Err(e) => return Err(e),
    }
    let current = crate::context::view_for_private_scope_at(
        store,
        c,
        &actor,
        &cid,
        1000,
        Some(&delegate.identity_id),
        Some(&delegate),
        Some(view.watermark),
        Some(root),
    )?;
    if serde_json::to_value(&current).map_err(|_| Error::storage())?
        != serde_json::to_value(&view).map_err(|_| Error::storage())?
    {
        return Ok(None);
    }
    Ok(Some(crate::records::context::deadline(&view).map_or(
        actor.expires_at.min(delegate.expires_at),
        |until| actor.expires_at.min(delegate.expires_at).min(until),
    )))
}
pub(crate) fn ready(
    c: &Connection,
    store: &Store,
    root: &str,
    selected: &str,
    audience: &str,
) -> Result<bool> {
    Ok(super::cache::ready_until(c, store, root, selected, audience)?.is_some())
}

pub(crate) fn require_for_native(
    c: &Connection,
    store: &Store,
    root: &str,
    selected: &str,
) -> Result<()> {
    let (candidate,audience):(String,String)=c.query_row("SELECT connection_id,audience_json FROM consultation_candidate_bindings WHERE operation_record_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(||Error::new("not-ready","A fresh private candidate must be registered"))?;
    let ai:bool=c.query_row("SELECT p.kind='ai' OR i.kind='ai' FROM connections v JOIN participants p ON p.participant_id=v.participant_id JOIN identities i ON i.id=v.identity_id WHERE v.id=?1",[candidate],|r|r.get(0)).map_err(|_|Error::storage())?;
    if ai && !ready(c, store, root, selected, &audience)? {
        return Err(Error::new(
            "not-ready",
            "Private provider context initialization is not confirmed",
        ));
    }
    Ok(())
}
