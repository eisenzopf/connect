//! Durable local preparation. Native transport/negotiation commits follow this
//! immutable intent; a preparing row never proves media readiness.
use crate::{Actor, Store};
use connect_protocol::{connection_control::*, membership::ContentKind, Error, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
pub(crate) mod handoff_preparation;
pub(crate) mod private_consultation;
pub(crate) mod sip_consultation;

fn conflict() -> Error {
    Error::new("conflict", "Connection intent or revision conflicts")
}
fn own(
    c: &Connection,
    actor: &Actor,
    r: &PrepareRequest,
) -> Result<connect_protocol::membership::MemberView> {
    let member = crate::audience::member(c, &actor.identity_id, &r.intent.conversation_id, true)?;
    if r.intent
        .by_participant
        .as_ref()
        .is_some_and(|p| p != &member.participant_id)
    {
        return Err(Error::forbidden());
    }
    Ok(member)
}
pub(crate) fn intent_resources(
    c: &Connection,
    actor: &Actor,
    intent: &OfferIntent,
) -> Result<Vec<String>> {
    let request = PrepareRequest::new(intent.clone(), "scope-only".into());
    resources(c, actor, &request)
}
pub(crate) fn resources(c: &Connection, actor: &Actor, r: &PrepareRequest) -> Result<Vec<String>> {
    let mut normalized = r.clone();
    normalized.normalize()?;
    actor.require("connection.offer")?;
    own(c, actor, r)?;
    let handoff_preparation = handoff_preparation::current(c, actor, &r.intent)?;
    if let Some(saved) =
        crate::operations::receipts::receipt(c, &actor.identity_id, &r.preparation_id)?
    {
        crate::operations::receipts::check_receipt_authority(c, actor, &saved, None)?;
        if saved.conversation_id.as_deref() != Some(r.intent.conversation_id.as_str()) {
            return Err(Error::forbidden());
        }
        return Ok(saved.resources);
    }
    let conversation =
        crate::conversations::authorized_conversation(c, actor, &r.intent.conversation_id)?;
    if conversation.closed {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        &r.intent.conversation_id,
        ContentKind::Session,
        &r.intent.session_id,
    )?;
    if handoff_preparation {
        return Ok(vec![actor.identity_id.clone()]);
    }
    if let Some(context) = private_consultation::context(
        c,
        &r.intent,
        (r.endpoint_id != "scope-only").then_some(r.endpoint_id.as_str()),
    )? {
        if !context
            .audience
            .iter()
            .any(|(id, _, _)| id == &actor.identity_id)
        {
            return Err(Error::forbidden());
        }
        return Ok(context.audience.into_iter().map(|(id, _, _)| id).collect());
    }
    let mut statement=c.prepare("SELECT DISTINCT p.identity_id FROM resource_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=a.conversation_id AND p.membership_epoch=a.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE a.kind='session' AND a.id=?1 AND a.conversation_id=?2 ORDER BY p.identity_id LIMIT 257").map_err(|_|Error::storage())?;
    let ids = statement
        .query_map(
            params![r.intent.session_id, r.intent.conversation_id],
            |row| row.get(0),
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|_| Error::storage())?;
    if ids.is_empty() || ids.len() > 256 {
        return Err(Error::new(
            "capacity-exceeded",
            "Session audience exceeds Connection bound",
        ));
    }
    Ok(ids)
}
pub(crate) fn apply(
    c: &Connection,
    actor: &Actor,
    r: &PrepareRequest,
    store: &Store,
) -> Result<Outcome> {
    let member = own(c, actor, r)?;
    handoff_preparation::current(c, actor, &r.intent)?;
    private_consultation::current(store, c, &r.intent, Some(&r.endpoint_id))?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 UNION ALL SELECT 1 FROM native_replacements WHERE candidate_connection_id=?1)",
            [&r.intent.connection_id],
            |row| row.get(0),
        )
        .map_err(|_| Error::storage())?;
    if exists
        || r.intent
            .expected_revision
            .is_some_and(|revision| revision != 0)
    {
        return Err(conflict());
    }
    let session = crate::session_control::view(c, &r.intent.conversation_id, &r.intent.session_id)?;
    if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
        return Err(Error::new(
            "not-ready",
            "Session has not accepted its media intent",
        ));
    }
    let accepted:Option<String>=c.query_row("SELECT cap.capabilities_json FROM session_participants sp JOIN session_presence_capabilities cap ON cap.session_id=sp.session_id AND cap.participant_id=sp.participant_id AND cap.membership_epoch=sp.membership_epoch WHERE sp.session_id=?1 AND sp.participant_id=?2 AND sp.membership_epoch=?3 AND sp.state='present'",params![r.intent.session_id,member.participant_id,member.membership_epoch],|row|row.get(0)).optional().map_err(|_|Error::storage())?;
    let accepted: Value = serde_json::from_str(&accepted.ok_or_else(Error::forbidden)?)
        .map_err(|_| Error::storage())?;
    let endpoint:Option<(String,String)>=c.query_row("SELECT e.tenant_id,e.capabilities_json FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='quic' AND e.verification_state='verified'",params![r.endpoint_id,actor.identity_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|_|Error::storage())?;
    let (tenant, endpoint_caps) = endpoint.ok_or_else(Error::forbidden)?;
    let endpoint_caps: Vec<String> =
        serde_json::from_str(&endpoint_caps).map_err(|_| Error::storage())?;
    for stream in r.intent.offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::storage)?
    {
        let kind = stream["kind"].as_str().ok_or_else(Error::storage)?;
        if !session.media.iter().any(|m| m == kind) || !endpoint_caps.iter().any(|m| m == kind) {
            return Err(Error::new(
                "unsupported-media-profile",
                "Connection media is outside the accepted profile",
            ));
        }
        let allowed = accepted
            .get(kind)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::new(
                    "unsupported-media-profile",
                    "Participant did not accept this media",
                )
            })?;
        let offered = session
            .capabilities_offer
            .as_ref()
            .and_then(|c| c.get(kind))
            .and_then(Value::as_array)
            .ok_or_else(Error::forbidden)?;
        if !stream["codec_preferences"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .any(|codec| {
                if kind == "data" {
                    codec
                        .as_str()
                        .is_some_and(|name| matches!(name, "text" | "json" | "binary"))
                        && allowed.iter().any(|v| v == "reliable-ordered")
                        && offered.iter().any(|v| v == "reliable-ordered")
                } else {
                    allowed.contains(codec) && offered.contains(codec)
                }
            })
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "Connection has no accepted codec",
            ));
        }
    }
    let count:u64=c.query_row("SELECT COUNT(*) FROM connections WHERE state NOT IN ('retired','failed','interrupted')",[],|row|row.get(0)).map_err(|_|Error::storage())?;
    if count >= store.limits.connections as u64 {
        return Err(Error::new(
            "capacity-exceeded",
            "Connection capacity exhausted",
        ));
    }
    let recipients = resources(c, actor, r)?;
    if recipients.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    let journal: u64 = c
        .query_row("SELECT COUNT(*) FROM journal", [], |row| row.get(0))
        .map_err(|_| Error::storage())?;
    if !crate::tasks::budget::journal(
        c,
        store.limits.journal_events,
        journal,
        recipients.len() as u64,
    )? {
        return Err(Error::new(
            "capacity-exceeded",
            "Connection journal capacity exhausted",
        ));
    }
    crate::session_reconnect::permit_offer(c, store, &r.intent.session_id)?;
    c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state) VALUES(?1,?2,?3,?4,?5,?6,?7,1,'preparing')",params![r.intent.connection_id,r.intent.session_id,member.participant_id,member.membership_epoch,actor.identity_id,tenant,r.endpoint_id]).map_err(|_|Error::storage())?;
    c.execute(
        "INSERT INTO connection_offers VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            r.intent.connection_id,
            format!("conn_{}", uuid::Uuid::new_v4().simple()),
            r.intent.operation_id,
            actor.credential_id,
            crate::provisioning::canonical_json(&r.intent.offer)?,
            crate::now()
        ],
    )
    .map_err(|_| Error::storage())?;
    let value = PreparedConnection {
        connection_id: r.intent.connection_id.clone(),
        conversation_id: r.intent.conversation_id.clone(),
        session_id: r.intent.session_id.clone(),
        participant_id: member.participant_id,
        membership_epoch: member.membership_epoch,
        endpoint_id: r.endpoint_id.clone(),
        generation: 1,
        revision: 0,
        state: "preparing".into(),
    };
    for recipient in recipients {
        let mut data = json!({"source_kind":"session","source_id":r.intent.session_id,"session_id":r.intent.session_id,"connection_id":r.intent.connection_id,"participant_id":value.participant_id,"connection":value});
        if recipient == actor.identity_id {
            data["operation_id"] = json!(r.preparation_id);
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,source_json) VALUES(?1,?2,?3,'connection_preparing',?4,?5,?6)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,r.intent.conversation_id,serde_json::to_string(&data).map_err(|_|Error::storage())?,session.revision,serde_json::to_string(&json!({"kind":"connect-connection-preparation","correlation_id":r.intent.connection_id})).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
    }
    Ok(Outcome::ConnectionPrepared(value))
}

impl Store {
    /// Exact signed preparation under the already-held native Device/key fence.
    /// Native endpoint selection is a trusted backend assertion, not a grant.
    pub fn prepare_connection_with_peer(
        &self,
        credential: &str,
        intent: OfferIntent,
        endpoint_id: String,
        proof: &crate::PeerAuthorityGuard,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        self.execute_runtime_composed(
            credential,
            &connect_protocol::Operation::ConnectionPrepare {
                request: PrepareRequest::new(intent.clone(), endpoint_id.clone()),
            },
            &crate::DiscoveryRuntime::default(),
            Some(Self::peer_scopes(proof)),
            Some(proof),
        )?;
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let id = PrepareRequest::new(intent, endpoint_id).preparation_id;
        let record = crate::transitions::current_control_record(&c, &actor, &id)?;
        self.require_peer_source(&c, proof, credential)?;
        Ok(record)
    }
}
#[cfg(test)]
mod tests;
