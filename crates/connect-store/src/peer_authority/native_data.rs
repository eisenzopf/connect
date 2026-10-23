//! Current source planning and original address snapshots. This creates no operation, queue,
//! delivery outcome or media grant. Runtime custody is a separate requirement.
use super::*;
use connect_protocol::{
    membership::{ContentKind, MemberView},
    native_streams::{Command, Intent},
};

pub struct NativeDataProjection {
    pub source: NativeStreamProjection,
    pub message: connect_protocol::native_data::Message,
    /// Fresh addresses before admission; the original frozen snapshots after
    /// admission. Snapshot role/presence fields grant no current delivery right.
    pub recipients: Vec<MemberView>,
    /// Original offered native protocols, not an observed selected engine port.
    pub source_protocols: Vec<String>,
}
impl std::fmt::Debug for NativeDataProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeDataProjection(redacted)")
    }
}
fn denied() -> Error {
    Error::new(
        "unsupported-media-profile",
        "Native data is outside its current source profile",
    )
}
pub(super) fn data_floor(p: &PeerConnectionProjection) -> bool {
    let supports = |caps: &serde_json::Value| {
        caps["data"]
            .as_array()
            .is_some_and(|v| v.iter().any(|n| n == "reliable-ordered"))
    };
    p.session.session.media.iter().any(|m| m == "data")
        && p.endpoint_capabilities.iter().any(|m| m == "data")
        && supports(&p.accepted_capabilities)
        && p.session
            .session
            .capabilities_offer
            .as_ref()
            .is_some_and(supports)
}
fn source(c: &Connection, intent: &Intent, view: &NativeStreamProjection) -> Result<Vec<String>> {
    if intent.command != Command::Data {
        return Err(Error::forbidden());
    }
    let message = connect_protocol::native_data::message(&intent.payload)?;
    let p = &view.connection;
    if !data_floor(p) {
        return Err(denied());
    }
    if intent
        .expected_revision
        .is_some_and(|r| r != view.stream_revision)
    {
        return Err(Error::new(
            "conflict",
            "Native source stream revision conflicts",
        ));
    }
    let streams = p.offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::storage)?;
    let stream = streams
        .iter()
        .find(|s| {
            s["id"] == message.label
                && s["kind"] == "data"
                && matches!(s["direction"].as_str(), Some("sendonly" | "sendrecv"))
        })
        .ok_or_else(denied)?;
    let declared = p.offer["capabilities"]["data_protocols"]
        .as_array()
        .ok_or_else(denied)?;
    let protocols = stream["codec_preferences"]
        .as_array()
        .ok_or_else(denied)?
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|v| {
            matches!(*v, "text" | "json" | "binary")
                && declared.iter().any(|n| n.as_str() == Some(v))
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if protocols.is_empty() {
        return Err(denied());
    }
    // An original offer may contain rejected streams. Only a real retained
    // readiness observation of this logical label can be a publication source.
    let ready:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections c,json_each(c.readiness_json,'$.streams') stream WHERE c.id=?1 AND c.generation=?2 AND stream.value=?3)",params![intent.connection_id,p.connection.generation,message.label],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !ready {
        return Err(denied());
    }
    Ok(protocols)
}
pub(crate) fn recipients(
    c: &Connection,
    actor: &crate::Actor,
    intent: &Intent,
    view: &NativeStreamProjection,
) -> Result<Vec<MemberView>> {
    source(c, intent, view)?;
    if let Some(frozen) = frozen(c, actor, intent)? {
        return Ok(frozen);
    }
    let message = connect_protocol::native_data::message(&intent.payload)?;
    let pids = match message.recipients {
        Some(pids) => pids,
        None => {
            let mut q=c.prepare("SELECT p.participant_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.revoked=0 JOIN session_participants sp ON sp.participant_id=p.participant_id AND sp.membership_epoch=p.membership_epoch WHERE p.conversation_id=?1 AND p.active=1 AND sp.session_id=?2 AND sp.state='present' ORDER BY p.participant_id LIMIT 257").map_err(|_|Error::storage())?;
            let values = q
                .query_map(params![intent.conversation_id, intent.session_id], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?;
            if values.len() > 256 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Native data audience exceeds its bounded profile",
                ));
            }
            values
        }
    };
    let mut audience = Vec::with_capacity(pids.len());
    for pid in pids {
        let identity:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE conversation_id=?1 AND participant_id=?2 AND active=1",params![intent.conversation_id,pid],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let identity = identity.ok_or_else(Error::forbidden)?;
        let member = crate::audience::member(c, &identity, &intent.conversation_id, true)?;
        let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![intent.session_id,member.participant_id,member.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !present
            || member.participant_id != pid
            || !actor.permits_recipient(&identity)
            || !crate::audience::visible_to_identity(
                c,
                &identity,
                &intent.conversation_id,
                ContentKind::Session,
                &intent.session_id,
            )?
        {
            return Err(Error::forbidden());
        }
        audience.push(member);
    }
    Ok(audience)
}
/// Historical original address snapshots, never fresh permission. Actual
/// delivery must recheck each current target against these original epochs.
pub(crate) fn frozen(
    c: &Connection,
    actor: &crate::Actor,
    intent: &Intent,
) -> Result<Option<Vec<MemberView>>> {
    let saved:Option<(String,String)>=c.query_row("SELECT d.operation_record_id,o.digest FROM native_data_messages d JOIN operations o ON o.record_id=d.operation_record_id WHERE o.owner_id=?1 AND o.operation_id=?2",params![actor.identity_id,intent.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    let Some((record, hash)) = saved else {
        return Ok(None);
    };
    if crate::transitions::digest(&serde_json::to_value(intent).map_err(|_| Error::storage())?)?.0
        != hash
    {
        return Err(Error::new(
            "conflict",
            "Native data message intent conflicts",
        ));
    }
    let mut q=c.prepare("SELECT member_json FROM native_data_recipients WHERE operation_record_id=?1 ORDER BY participant_id").map_err(|_|Error::storage())?;
    let raw = q
        .query_map([record], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    let members = raw
        .into_iter()
        .map(|v| serde_json::from_str::<MemberView>(&v).map_err(|_| Error::storage()))
        .collect::<Result<Vec<_>>>()?;
    if members
        .iter()
        .any(|p| !actor.permits_recipient(&p.identity_id))
    {
        return Err(Error::forbidden());
    }
    Ok(Some(members))
}

impl Store {
    /// Recheck this exact protected native message under its original readers.
    /// Metadata is not a durable publication or a grant to an engine/queue.
    pub fn project_native_data_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeDataProjection> {
        let PeerCommand::NativeStream(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if intent.command != Command::Data {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let view = self.native_stream_projection_at(&tx, &actor, proof)?;
        let source_protocols = source(&tx, intent, &view)?;
        let recipients = recipients(&tx, &actor, intent, &view)?;
        self.require_peer_source(&tx, proof, credential)?;
        Ok(NativeDataProjection {
            source: view,
            message: connect_protocol::native_data::message(&intent.payload)?,
            recipients,
            source_protocols,
        })
    }
}
