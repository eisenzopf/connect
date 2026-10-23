//! Every link use intersects fresh credential, delegation and both source ACLs.
use crate::{audience, Actor, Store};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use connect_protocol::{
    attachments::{ArtifactLink, LinkPurpose},
    membership::ContentKind,
    Artifact, Error, Operation, Result,
};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
const DOMAIN: &[u8] = b"connect-attachment-link/1\0";
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskLinkContext {
    pub task_id: String,
    pub grant_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    version: u8,
    installation: String,
    runtime: String,
    credential: String,
    identity: String,
    participant: String,
    epoch: u64,
    conversation: String,
    artifact: String,
    message: Option<String>,
    purpose: LinkPurpose,
    expires_at: i64,
    sha256: String,
    size: u64,
    task: Option<TaskLinkContext>,
}
fn denied() -> Error {
    Error::forbidden()
}
fn keys(c: &Connection) -> Result<(String, String)> {
    c.query_row(
        "SELECT id,cursor_key FROM installation WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|_| Error::storage())
}
fn mac(key: &str, bytes: &[u8]) -> Result<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(DOMAIN);
    mac.update(bytes);
    Ok(mac)
}
fn path(cid: &str, id: &str, token: &str) -> String {
    format!("/v1/conversations/{cid}/artifacts/{id}/links/{token}")
}
fn resource(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    id: &str,
    message: Option<&str>,
    purpose: LinkPurpose,
    reference: bool,
) -> Result<Artifact> {
    audience::require(c, actor, cid, ContentKind::Artifact, id)?;
    let artifact = crate::artifacts::metadata(c, cid, id)?;
    if artifact.expires_at.is_some_and(|v| v <= crate::now()) {
        return Err(denied());
    }
    match purpose {
        LinkPurpose::Upload => {
            actor.require("artifact_append")?;
            audience::require_original(c, actor, cid, ContentKind::Artifact, id)?;
            if artifact.owner_id != actor.identity_id
                || if reference {
                    artifact.state != "ready"
                } else {
                    artifact.state != "uploading"
                }
            {
                return Err(denied());
            }
            if message.is_some() {
                return Err(denied());
            }
        }
        LinkPurpose::Download => {
            actor.require("artifact_get")?;
            if artifact.state != "ready" {
                return Err(denied());
            }
            let message = message.ok_or_else(denied)?;
            audience::require(c, actor, cid, ContentKind::Message, message)?;
            let bound:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM message_attachments WHERE message_id=?1 AND artifact_id=?2 AND conversation_id=?3 AND content_type=?4 AND size_bytes=?5)",params![message,id,cid,artifact.media_type,artifact.size],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !bound {
                return Err(denied());
            }
        }
    }
    Ok(artifact)
}
pub(crate) fn issue(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    id: &str,
    message: Option<&str>,
    purpose: LinkPurpose,
    expires_in: u32,
    task: Option<&TaskLinkContext>,
    store: &Store,
) -> Result<ArtifactLink> {
    if expires_in == 0 || expires_in > 300 {
        return Err(Error::new(
            "invalid-request",
            "Attachment link lifetime must be 1–300 seconds",
        ));
    }
    let artifact = resource(c, actor, cid, id, message, purpose, false)?;
    let member = audience::member(c, &actor.identity_id, cid, true)?;
    let (installation, key) = keys(c)?;
    let expires_at = (crate::now() + i64::from(expires_in))
        .min(actor.expires_at)
        .min(artifact.expires_at.unwrap_or(i64::MAX));
    if expires_at <= crate::now() {
        return Err(denied());
    }
    let claims = Claims {
        version: 1,
        installation,
        runtime: store.link_epoch.clone(),
        credential: actor.credential_id.clone(),
        identity: actor.identity_id.clone(),
        participant: member.participant_id,
        epoch: member.membership_epoch,
        conversation: cid.into(),
        artifact: id.into(),
        message: message.map(str::to_owned),
        purpose,
        expires_at,
        sha256: artifact.sha256,
        size: artifact.size,
        task: task.cloned(),
    };
    let bytes = serde_json::to_vec(&claims).map_err(|_| Error::storage())?;
    let token = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        hex::encode(mac(&key, &bytes)?.finalize().into_bytes())
    );
    Ok(ArtifactLink {
        conversation_id: cid.into(),
        artifact_id: id.into(),
        message_id: message.map(str::to_owned),
        path: path(cid, id, &token),
        expires_at,
        purpose,
    })
}
fn claims(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    id: &str,
    link: &str,
    task: Option<&TaskLinkContext>,
    store: &Store,
) -> Result<Claims> {
    if link.len() > 8192 {
        return Err(denied());
    }
    let token = link.strip_prefix(&path(cid, id, "")).ok_or_else(denied)?;
    let (body, signature) = token.split_once('.').ok_or_else(denied)?;
    let bytes = URL_SAFE_NO_PAD.decode(body).map_err(|_| denied())?;
    let signature = hex::decode(signature).map_err(|_| denied())?;
    let (installation, key) = keys(c)?;
    mac(&key, &bytes)?
        .verify_slice(&signature)
        .map_err(|_| denied())?;
    let claims: Claims = serde_json::from_slice(&bytes).map_err(|_| denied())?;
    let member = audience::member(c, &actor.identity_id, cid, true)?;
    if actor.expires_at <= crate::now()
        || claims.version != 1
        || claims.installation != installation
        || claims.runtime != store.link_epoch
        || claims.credential != actor.credential_id
        || claims.identity != actor.identity_id
        || claims.participant != member.participant_id
        || claims.epoch != member.membership_epoch
        || claims.conversation != cid
        || claims.artifact != id
        || claims.task.as_ref() != task
        || claims.expires_at <= crate::now()
    {
        return Err(denied());
    }
    Ok(claims)
}
pub(crate) fn authorize(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    id: &str,
    link: &str,
    purpose: LinkPurpose,
    task: Option<&TaskLinkContext>,
    store: &Store,
) -> Result<()> {
    let claims = claims(c, actor, cid, id, link, task, store)?;
    if claims.purpose != purpose {
        return Err(denied());
    }
    let artifact = resource(c, actor, cid, id, claims.message.as_deref(), purpose, false)?;
    if claims.sha256 != artifact.sha256 || claims.size != artifact.size {
        return Err(denied());
    }
    Ok(())
}
pub(crate) fn check_operation(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    task: Option<&TaskLinkContext>,
    store: &Store,
) -> Result<()> {
    match operation {
        Operation::ArtifactLinkGet {
            conversation_id,
            artifact_id,
            link,
            ..
        } => authorize(
            c,
            actor,
            conversation_id,
            artifact_id,
            link,
            LinkPurpose::Download,
            task,
            store,
        ),
        Operation::ArtifactLinkAppend {
            conversation_id,
            artifact_id,
            link,
            ..
        } => authorize(
            c,
            actor,
            conversation_id,
            artifact_id,
            link,
            LinkPurpose::Upload,
            task,
            store,
        ),
        _ => Ok(()),
    }
}
pub(crate) fn canonical_intent(mut value: serde_json::Value) -> serde_json::Value {
    if value.get("op").and_then(|v| v.as_str()) == Some("artifact_link_append") {
        if let Some(object) = value.as_object_mut() {
            object.remove("link");
            object.insert("op".into(), "artifact_append".into());
        }
    }
    value
}
