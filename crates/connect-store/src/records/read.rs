//! Current source facts derived under the original held command fence. None of
//! these DTOs is an authority lease or a public transport assertion.
use super::*;
use crate::Actor;
use connect_protocol::membership::ContentKind;
use std::collections::BTreeSet;
#[derive(Clone, PartialEq, Eq)]
pub struct RecordFetchIntent {
    pub conversation_id: String,
    pub session_id: String,
    pub uuid: Option<String>,
    pub url: Option<String>,
}
impl RecordFetchIntent {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.conversation_id)
            || !valid_id(&self.session_id)
            || (self.uuid.is_none() && self.url.is_none())
            || self
                .uuid
                .as_ref()
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
            || self.url.as_ref().is_some_and(|url| {
                url.is_empty()
                    || url.len() > 2048
                    || url.chars().any(|v| v.is_control() || v.is_whitespace())
            })
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid local record selector",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq)]
pub enum RecordViewer {
    Participant {
        participant_id: String,
        membership_epoch: u64,
    },
    TenantRole {
        identity_id: String,
        tenant_id: String,
        role: String,
        revision: u64,
        private_sources: bool,
    },
}
impl RecordViewer {
    pub fn participant(&self) -> Option<(&str, u64)> {
        match self {
            Self::Participant {
                participant_id,
                membership_epoch,
            } => Some((participant_id, *membership_epoch)),
            Self::TenantRole { .. } => None,
        }
    }
}
#[derive(Clone)]
pub struct RecordViewOrigin {
    pub scope: RecordScope,
    pub viewer: RecordViewer,
}
#[derive(Clone, PartialEq, Eq)]
pub struct RecordViewBinding {
    pub uuid: String,
    pub revision: u64,
    pub body_hash: String,
    pub manifest_hash: String,
    pub viewer: RecordViewer,
}
pub(crate) fn participant_mutation_origin(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<RecordViewOrigin> {
    intent.validate()?;
    actor.require("record_get")?;
    crate::conversations::authorized_conversation(c, actor, &intent.conversation_id)?;
    crate::sessions::get(c, &intent.conversation_id, &intent.session_id)?;
    crate::audience::require(
        c,
        actor,
        &intent.conversation_id,
        ContentKind::Session,
        &intent.session_id,
    )?;
    let member = crate::audience::member(c, &actor.identity_id, &intent.conversation_id, true)?;
    let tenant_id: String = c
        .query_row(
            "SELECT tenant_id FROM conversations WHERE id=?1",
            [&intent.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(RecordViewOrigin {
        scope: RecordScope {
            tenant_id,
            conversation_id: intent.conversation_id.clone(),
            session_id: intent.session_id.clone(),
        },
        viewer: RecordViewer::Participant {
            participant_id: member.participant_id,
            membership_epoch: member.membership_epoch,
        },
    })
}
/// Record-only history sharing grants the shared record floor without live
/// Session authority. Mutations keep their independent original admission.
pub(crate) fn participant_origin(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<RecordViewOrigin> {
    actor.require("record_get")?;
    match participant_mutation_origin(c, actor, intent) {
        Ok(origin) => return Ok(origin),
        Err(error) if error.code == "forbidden" => (),
        Err(error) => return Err(error),
    }
    intent.validate()?;
    crate::conversations::authorized_conversation(c, actor, &intent.conversation_id)?;
    let id = intent
        .uuid
        .clone()
        .or_else(|| intent.url.as_deref().and_then(selected_uuid))
        .ok_or_else(Error::forbidden)?;
    crate::audience::require(c, actor, &intent.conversation_id, ContentKind::Record, &id)?;
    let member = crate::audience::member(c, &actor.identity_id, &intent.conversation_id, true)?;
    let tenant_id: String=c.query_row("SELECT tenant_id FROM session_record_intents WHERE uuid=?1 AND conversation_id=?2 AND session_id=?3",params![id,intent.conversation_id,intent.session_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    Ok(RecordViewOrigin {
        scope: RecordScope {
            tenant_id,
            conversation_id: intent.conversation_id.clone(),
            session_id: intent.session_id.clone(),
        },
        viewer: RecordViewer::Participant {
            participant_id: member.participant_id,
            membership_epoch: member.membership_epoch,
        },
    })
}
pub(crate) fn origin(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<RecordViewOrigin> {
    intent.validate()?;
    actor.require("record_get")?;
    let role:Option<(String,String,u64,bool)>=c.query_row("SELECT r.tenant_id,r.role,r.revision,r.private_sources FROM tenant_record_readers r JOIN identities i ON i.id=r.identity_id AND i.tenant_id=r.tenant_id AND i.revoked=0 WHERE r.identity_id=?1 AND r.active=1",[&actor.identity_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
    let Some((tenant_id, role, revision, private_sources)) = role else {
        return participant_origin(c, actor, intent);
    };
    let owned:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN conversations c ON c.id=s.conversation_id WHERE s.id=?1 AND c.id=?2 AND c.tenant_id=?3)",params![intent.session_id,intent.conversation_id,tenant_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !owned {
        return Err(Error::forbidden());
    }
    // A shared tenant grant adds rights; it must not mask a participant's own
    // original source authority. Prefer that proof only when the record's
    // original Session epoch or an explicit current share is actually eligible.
    if !private_sources {
        match participant_origin(c, actor, intent) {
            Ok(member) => {
                let (pid, epoch) = member.viewer.participant().ok_or_else(Error::forbidden)?;
                let eligible:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_record_intents i JOIN session_record_audiences a ON a.uuid=i.uuid WHERE i.conversation_id=?1 AND i.session_id=?2 AND a.source_group='session' AND a.participant_id=?3 AND a.membership_epoch=?4) OR EXISTS(SELECT 1 FROM history_share_items h JOIN history_shares s ON s.id=h.sharing_id AND s.active=1 WHERE ((h.kind='session' AND h.source_id=?2) OR (h.kind='record' AND h.source_id=?5)) AND h.conversation_id=?1 AND h.participant_id=?3 AND h.membership_epoch=?4)",params![intent.conversation_id,intent.session_id,pid,epoch,intent.uuid.clone().or_else(||intent.url.as_deref().and_then(selected_uuid))],|r|r.get(0)).map_err(|_|Error::storage())?;
                if eligible {
                    return Ok(member);
                }
            }
            Err(error) if matches!(error.code.as_str(), "forbidden" | "not-found") => (),
            Err(error) => return Err(error),
        }
    }
    Ok(RecordViewOrigin {
        scope: RecordScope {
            tenant_id: tenant_id.clone(),
            conversation_id: intent.conversation_id.clone(),
            session_id: intent.session_id.clone(),
        },
        viewer: RecordViewer::TenantRole {
            identity_id: actor.identity_id.clone(),
            tenant_id,
            role,
            revision,
            private_sources,
        },
    })
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<Vec<AuthorityScope>> {
    origin(c, actor, intent)?;
    crate::authority_admission::conversation_scopes(c, &intent.conversation_id, AuthorityMode::Read)
}
pub(crate) fn sources(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
    binding: &RecordViewBinding,
) -> Result<BTreeSet<String>> {
    let viewer = origin(c, actor, intent)?;
    sources_from_origin(c, actor, intent, binding, viewer, true)
}
pub(super) fn sources_from_origin(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
    binding: &RecordViewBinding,
    viewer: RecordViewOrigin,
    allow_record_share: bool,
) -> Result<BTreeSet<String>> {
    if intent.uuid.as_ref().is_some_and(|id| {
        uuid::Uuid::parse_str(id).ok().map(|v| v.to_string()) != Some(binding.uuid.clone())
    }) || intent
        .url
        .as_ref()
        .is_some_and(|url| url != &location(&binding.uuid))
    {
        return Err(Error::forbidden());
    }

    if viewer.viewer != binding.viewer {
        return Err(Error::forbidden());
    }
    let stored = row(c, &viewer.scope, &binding.uuid)?.ok_or_else(Error::forbidden)?;
    if stored.handle.revision != binding.revision
        || stored.handle.body_hash != binding.body_hash
        || stored.handle.manifest_hash != binding.manifest_hash
    {
        return Err(Error::forbidden());
    }
    let version: Option<u64> = c
        .query_row(
            "SELECT format_version FROM session_record_intents WHERE uuid=?1 AND state='ready'",
            [&binding.uuid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if version != Some(2) {
        return Err(Error::new(
            "unsupported-feature",
            "Record source policy unavailable",
        ));
    }
    let bytes = stored.bytes.ok_or_else(Error::forbidden)?;
    let body: serde_json::Value =
        serde_json::from_slice(&bytes.body).map_err(|_| Error::storage())?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes.manifest).map_err(|_| Error::storage())?;
    if manifest["policy"] != "connect-session-metadata-v1" {
        return Err(Error::forbidden());
    }
    if let RecordViewer::TenantRole {
        private_sources, ..
    } = &viewer.viewer
    {
        return tenant_sources(
            c,
            actor,
            intent,
            &binding.uuid,
            &body,
            &manifest,
            *private_sources,
        );
    }
    let (participant, epoch) = viewer.viewer.participant().ok_or_else(Error::forbidden)?;
    participant_sources(
        c,
        intent,
        &binding.uuid,
        &body,
        &manifest,
        participant,
        epoch,
        allow_record_share,
        |id| actor.permits_recipient(id),
    )
}
/// Identity audience floor only. Action permission and actual IO ownership stay
/// with the caller; passive listeners never borrow another credential's scopes.
pub(super) fn participant_sources(
    c: &Connection,
    intent: &RecordFetchIntent,
    uuid: &str,
    body: &serde_json::Value,
    manifest: &serde_json::Value,
    participant: &str,
    epoch: u64,
    allow_record_share: bool,
    recipient: impl Fn(&str) -> bool,
) -> Result<BTreeSet<String>> {
    let mut q=c.prepare("SELECT source_group FROM session_record_audiences WHERE uuid=?1 AND participant_id=?2 AND membership_epoch=?3").map_err(|_|Error::storage())?;
    let groups = q
        .query_map(params![uuid, participant, epoch], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| Error::storage())?;
    let shared:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM history_share_items i JOIN history_shares h ON h.id=i.sharing_id AND h.active=1 WHERE ((i.kind='session' AND i.source_id=?1) OR (?6=1 AND i.kind='record' AND i.source_id=?5)) AND i.conversation_id=?2 AND i.participant_id=?3 AND i.membership_epoch=?4)",params![intent.session_id,intent.conversation_id,participant,epoch,uuid,allow_record_share],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !groups.contains("session") && !shared {
        return Err(Error::forbidden());
    }
    let rules = manifest["sources"]
        .as_array()
        .filter(|s| s.len() <= 4096)
        .ok_or_else(Error::storage)?;
    let mut allowed = BTreeSet::new();
    for rule in rules {
        let id = rule["id"].as_str().ok_or_else(Error::storage)?;
        let permit = if matches!(id, "document" | "session.outcome") {
            true
        } else if let Some(index) = id
            .strip_prefix("appearance.")
            .and_then(|n| n.parse::<usize>().ok())
        {
            let subject = body["parties"]
                .get(index)
                .and_then(|p| p["connect_participant_id"].as_str());
            if let Some(subject) = subject {
                let identity:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",params![subject,intent.conversation_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
                identity.is_some_and(|id| recipient(&id))
            } else {
                false
            }
        } else if id.starts_with("connection.") {
            groups.contains(id)
        } else {
            false
        };
        // Diagnostics cover whole-record counts. No caller inherits them from
        // Session membership; keep them separate from useful shared outcomes.
        if permit {
            allowed.insert(id.to_owned());
        }
    }
    let complete = rules.iter().all(|r| {
        r["id"] == "session.capture" || r["id"].as_str().is_some_and(|id| allowed.contains(id))
    });
    let diagnostics = &body["analysis"][1]["body"];
    if complete
        && [
            "appearance_omissions",
            "connection_omissions",
            "audience_omissions",
        ]
        .iter()
        .all(|field| diagnostics[*field].as_u64() == Some(0))
    {
        allowed.insert("session.capture".into());
    }
    Ok(allowed)
}

fn tenant_sources(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
    uuid: &str,
    body: &serde_json::Value,
    manifest: &serde_json::Value,
    private_sources: bool,
) -> Result<BTreeSet<String>> {
    let rules = manifest["sources"]
        .as_array()
        .filter(|r| r.len() <= 4096)
        .ok_or_else(Error::storage)?;
    let mut allowed = BTreeSet::new();
    for rule in rules {
        if rule["state"] == "metadata-unavailable" {
            continue;
        }
        let id = rule["id"].as_str().ok_or_else(Error::storage)?;
        let permit = if matches!(id, "document" | "session.outcome") {
            true
        } else if let Some(index) = id
            .strip_prefix("appearance.")
            .and_then(|n| n.parse::<usize>().ok())
        {
            if let Some(pid) = body["parties"]
                .get(index)
                .and_then(|v| v["connect_participant_id"].as_str())
            {
                let owner:Option<String>=c.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",params![pid,intent.conversation_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
                owner.is_some_and(|owner| actor.permits_recipient(&owner))
            } else {
                false
            }
        } else if id.starts_with("connection.") && private_sources {
            let mut q=c.prepare("SELECT DISTINCT p.identity_id FROM session_record_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=?3 WHERE a.uuid=?1 AND a.source_group=?2").map_err(|_|Error::storage())?;
            let owners = q
                .query_map(params![uuid, id, intent.conversation_id], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::storage())?;
            !owners.is_empty() && owners.iter().all(|id| actor.permits_recipient(id))
        } else {
            false
        };
        if permit {
            allowed.insert(id.into());
        }
    }
    let complete = rules.iter().all(|r| {
        r["id"] == "session.capture" || r["id"].as_str().is_some_and(|id| allowed.contains(id))
    });
    if complete
        && [
            "appearance_omissions",
            "connection_omissions",
            "audience_omissions",
        ]
        .iter()
        .all(|field| body["analysis"][1]["body"][*field].as_u64() == Some(0))
    {
        allowed.insert("session.capture".into());
    }
    Ok(allowed)
}

#[derive(Clone)]
pub struct RecordPolicySnapshot {
    pub origin: RecordViewOrigin,
    pub binding: RecordViewBinding,
    pub sources: BTreeSet<String>,
}
pub(crate) fn policy(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<Option<RecordPolicySnapshot>> {
    policy_for(c, actor, intent, origin(c, actor, intent)?, true)
}
pub(super) fn participant_policy(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<Option<RecordPolicySnapshot>> {
    policy_for(
        c,
        actor,
        intent,
        participant_mutation_origin(c, actor, intent)?,
        false,
    )
}
fn policy_for(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
    origin: RecordViewOrigin,
    allow_record_share: bool,
) -> Result<Option<RecordPolicySnapshot>> {
    let selected = match (&intent.uuid, &intent.url) {
        (Some(id), None) => uuid::Uuid::parse_str(id).ok().map(|v| v.to_string()),
        (None, Some(url)) => selected_uuid(url),
        (Some(id), Some(url)) => selected_uuid(url).filter(|selected| {
            uuid::Uuid::parse_str(id)
                .ok()
                .is_some_and(|id| id.to_string() == *selected)
        }),
        _ => None,
    };
    let Some(id) = selected else {
        return Ok(None);
    };
    let Some(stored) = row(c, &origin.scope, &id)? else {
        return Ok(None);
    };
    if stored.bytes.is_none() {
        return Ok(None);
    }
    let binding = RecordViewBinding {
        uuid: id,
        revision: stored.handle.revision,
        body_hash: stored.handle.body_hash,
        manifest_hash: stored.handle.manifest_hash,
        viewer: origin.viewer.clone(),
    };
    let sources = sources_from_origin(
        c,
        actor,
        intent,
        &binding,
        origin.clone(),
        allow_record_share,
    )?;
    Ok(Some(RecordPolicySnapshot {
        origin,
        binding,
        sources,
    }))
}

#[cfg(test)]
mod tests;

/// Shared metadata discovery does not read bodies or expose canonical digests.
/// It uses the same original-epoch/history requirement as the source reader.
pub(crate) fn availability(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
) -> Result<Option<connect_protocol::events::RecordAvailability>> {
    use connect_protocol::events::RecordAvailability;
    if !actor.permits("record_get")
        || !crate::audience::visible(c, actor, cid, ContentKind::Session, sid)?
    {
        return Ok(None);
    }
    let member = crate::audience::member(c, &actor.identity_id, cid, true)?;
    let value: Option<(String,String,u64,bool)> = c.query_row(
        "SELECT i.uuid,i.state,i.format_version,
        EXISTS(SELECT 1 FROM session_record_audiences a WHERE a.uuid=i.uuid AND a.source_group='session' AND a.participant_id=?3 AND a.membership_epoch=?4)
        OR (i.state='pending' AND EXISTS(SELECT 1 FROM json_each(i.snapshot_json,'$.audience') a WHERE json_extract(a.value,'$.participant_id')=?3 AND json_extract(a.value,'$.membership_epoch')=?4))
        OR EXISTS(SELECT 1 FROM history_share_items h JOIN history_shares s ON s.id=h.sharing_id AND s.active=1 WHERE h.kind='session' AND h.source_id=i.session_id AND h.conversation_id=i.conversation_id AND h.participant_id=?3 AND h.membership_epoch=?4)
        FROM session_record_intents i WHERE i.conversation_id=?1 AND i.session_id=?2",
        params![cid,sid,member.participant_id,member.membership_epoch], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
    ).optional().map_err(|_|Error::storage())?;
    let Some((uuid, state, version, allowed)) = value else {
        return Ok(None);
    };
    if !allowed {
        return Ok(None);
    }
    if version != 2 {
        return Ok(Some(RecordAvailability::Unavailable {
            session_id: sid.into(),
        }));
    }
    if state == "pending" {
        return Ok(Some(RecordAvailability::Pending {
            session_id: sid.into(),
        }));
    }
    let revision:Option<u64>=c.query_row("SELECT revision FROM session_records WHERE uuid=?1 AND conversation_id=?2 AND session_id=?3 AND deleted=0 AND (document_expires_at IS NULL OR document_expires_at>unixepoch())",params![uuid,cid,sid],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    Ok(Some(match revision {
        Some(revision) => RecordAvailability::Ready {
            session_id: sid.into(),
            uuid,
            revision,
        },
        None => RecordAvailability::Unavailable {
            session_id: sid.into(),
        },
    }))
}

impl Store {
    pub fn record_read_authority(
        &self,
        credential: &str,
        intent: &RecordFetchIntent,
        wait: &AuthorityWait,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.stable_authority(wait, || {
            let c = self.connection()?;
            let actor = crate::identity::actor_from_connection(&c, credential)?;
            let mut required = scopes(&c, &actor, intent)?;
            required.push(AuthorityScope::identity(
                &actor.identity_id,
                AuthorityMode::Read,
            )?);
            Ok(required)
        })
    }
    /// Trusted bearer composition under an already owned read fence. A DTO or
    /// resource selector cannot substitute for current credential/source policy.
    pub fn record_read_policy(
        &self,
        held: &AuthorityGuardSet,
        credential: &str,
        intent: &RecordFetchIntent,
    ) -> Result<(RecordViewOrigin, Option<RecordPolicySnapshot>)> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let mut required = scopes(&tx, &actor, intent)?;
        required.push(AuthorityScope::identity(
            &actor.identity_id,
            AuthorityMode::Read,
        )?);
        if !self.authority_covers(held, &required) {
            return Err(Error::forbidden());
        }
        let origin = origin(&tx, &actor, intent)?;
        let policy = policy(&tx, &actor, intent)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok((origin, policy))
    }
}
