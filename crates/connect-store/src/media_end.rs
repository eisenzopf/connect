//! End intent generations prevent history from selecting dispatch authority.
use crate::{identity::actor_from_connection_at, Actor};
use connect_protocol::{tasks::EffectState, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Call,
    Sip,
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Call => "call_end",
            Self::Sip => "sip_end",
        }
    }
}

pub(crate) fn record(
    c: &Connection,
    kind: Kind,
    resource: &str,
    credential: &str,
    request: &str,
    pending: bool,
) -> Result<()> {
    c.execute("UPDATE media_end_intents SET current=0,state=CASE WHEN state='pending' THEN 'superseded' ELSE state END WHERE kind=?1 AND resource_id=?2 AND current=1",params![kind.name(),resource]).map_err(|_|Error::storage())?;
    c.execute(
        "INSERT INTO media_end_intents VALUES(?1,?2,?3,?4,?5,NULL,?6,?7)",
        params![
            uuid::Uuid::new_v4().to_string(),
            kind.name(),
            resource,
            credential,
            request,
            if pending { "pending" } else { "fulfilled" },
            pending
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}

/// Register only the actual current mutation, never a cached or superseded end.
pub(crate) fn protect(
    c: &Connection,
    kind: Kind,
    resource: &str,
    request: &str,
    effect: &str,
    fulfilled: bool,
) -> Result<()> {
    let changed=c.execute("UPDATE media_end_intents SET effect_id=?1 WHERE kind=?2 AND resource_id=?3 AND wire_request_id=?4 AND effect_id IS NULL AND (current=1 OR state='fulfilled')",params![effect,kind.name(),resource,request]).map_err(|_|Error::storage())?;
    if changed == 0 {
        c.execute(
            "INSERT INTO media_end_intents VALUES(?1,?2,?3,NULL,NULL,?4,?5,0)",
            params![
                uuid::Uuid::new_v4().to_string(),
                kind.name(),
                resource,
                effect,
                if fulfilled { "fulfilled" } else { "legacy" }
            ],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(())
}

pub(crate) fn authorize(
    c: &Connection,
    kind: Kind,
    resource: &str,
    timestamp: i64,
) -> Result<(Actor, String)> {
    authorize_generation(c, kind, resource, timestamp, None)
}
pub(crate) fn authorize_generation(
    c: &Connection,
    kind: Kind,
    resource: &str,
    timestamp: i64,
    expected: Option<&str>,
) -> Result<(Actor, String)> {
    let row:Option<(String,Option<String>,Option<String>,String)>=c.query_row("SELECT id,credential_id,effect_id,state FROM media_end_intents WHERE kind=?1 AND resource_id=?2 AND current=1",params![kind.name(),resource],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
    let (id, credential, effect, state) = row.ok_or_else(Error::forbidden)?;
    if match expected {
        Some(expected) => state != "claimed" || id != expected,
        None => state != "pending",
    } {
        return Err(Error::forbidden());
    }
    let actor = actor_from_connection_at(
        c,
        credential.as_deref().ok_or_else(Error::forbidden)?,
        timestamp,
    )?;
    actor.require(kind.name())?;
    if let Some(effect) = effect {
        crate::tasks::authorize_effect(c, &effect, timestamp)?;
    }
    Ok((actor, id))
}
pub(crate) fn claimed(c: &Connection, id: &str) -> Result<()> {
    let changed=c.execute("UPDATE media_end_intents SET state='claimed' WHERE id=?1 AND current=1 AND state='pending'",[id]).map_err(|_|Error::storage())?;
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
/// The exact generation already reserved for one bounded physical request.
/// This validates current authority; it cannot reserve another request.
pub(crate) fn dispatched_authority(
    c: &Connection,
    kind: Kind,
    resource: &str,
    intent: &str,
    timestamp: i64,
) -> Result<(Actor, i64)> {
    let (credential, effect): (String, Option<String>) = c.query_row(
        "SELECT credential_id,effect_id FROM media_end_intents WHERE id=?1 AND kind=?2 AND resource_id=?3 AND current=1 AND state='dispatching'",
        params![intent, kind.name(), resource], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let actor = actor_from_connection_at(c, &credential, timestamp)?;
    actor.require(kind.name())?;
    let expires = match effect {
        Some(effect) => actor
            .expires_at
            .min(crate::tasks::effect_deadline(c, &effect, timestamp)?),
        None => actor.expires_at,
    };
    Ok((actor, expires))
}
pub(crate) fn blocked(c: &Connection, kind: Kind, resource: &str) -> Result<()> {
    c.execute("UPDATE media_end_intents SET state='blocked' WHERE kind=?1 AND resource_id=?2 AND current=1 AND state='pending'",params![kind.name(),resource]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn observation(c: &Connection, effect: &str) -> Result<Option<EffectState>> {
    let state: Option<String> = c
        .query_row(
            "SELECT state FROM media_end_intents WHERE effect_id=?1",
            [effect],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    Ok(match state.as_deref() {
        Some("blocked" | "superseded") => Some(EffectState::Failed),
        Some("legacy") => Some(EffectState::Uncertain),
        Some("fulfilled") => Some(EffectState::Succeeded),
        _ => None,
    })
}

/// Only a known authorization denial may cancel work. Infrastructure failure
/// rolls back the caller's transaction and leaves its intent pending.
pub(crate) fn permitted<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => Ok(None),
        Err(error) => Err(error),
    }
}

/// Once this commits, recovery treats the physical request as possibly sent.
pub(crate) fn dispatching(c: &Connection, id: &str) -> Result<()> {
    let changed=c.execute("UPDATE media_end_intents SET state='dispatching' WHERE id=?1 AND current=1 AND state='claimed'",[id]).map_err(|_|Error::storage())?;
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
pub(crate) fn reject_claim(c: &Connection, id: &str) -> Result<Option<bool>> {
    let current: Option<bool> = c
        .query_row(
            "SELECT current FROM media_end_intents WHERE id=?1 AND state='claimed'",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if current.is_some() {
        c.execute(
            "UPDATE media_end_intents SET state='blocked' WHERE id=?1 AND state='claimed'",
            [id],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(current)
}
pub(crate) fn admit_start(c: &Connection, kind: &str, resource: &str) -> Result<bool> {
    c.execute(
        "INSERT OR IGNORE INTO media_start_admissions VALUES(?1,?2)",
        params![kind, resource],
    )
    .map(|n| n == 1)
    .map_err(|_| Error::storage())
}
