//! Correlate the accepted local operation with provider-owned durable jobs.
use crate::Store;
use connect_protocol::{tasks::EffectState, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection};

pub(super) fn register(
    c: &Connection,
    effect: &str,
    operation: &Operation,
    outcome: &Outcome,
) -> Result<()> {
    let (kind, id) = match outcome {
        Outcome::Accepted { message_id, .. } => ("message", message_id.as_str()),
        Outcome::Call(call) => (
            if matches!(operation, Operation::CallEnd { .. }) {
                "call_end"
            } else {
                "call"
            },
            call.id.as_str(),
        ),
        Outcome::SipLeg(leg) => (
            if matches!(operation, Operation::SipEnd { .. }) {
                "sip_end"
            } else {
                "sip_leg"
            },
            leg.id.as_str(),
        ),
        Outcome::Session(session) => ("session", session.id.as_str()),
        _ => return Ok(()),
    };
    link(c, effect, kind, id)?;
    let end = match kind {
        "call_end" => Some(crate::media_end::Kind::Call),
        "sip_end" => Some(crate::media_end::Kind::Sip),
        _ => None,
    };
    if let Some(end) = end {
        let fulfilled = match outcome {
            Outcome::Call(v) => matches!(v.state.as_str(), "ended" | "cancelled"),
            Outcome::SipLeg(v) => matches!(v.state.as_str(), "ended" | "cancelled"),
            _ => false,
        };
        crate::media_end::protect(
            c,
            end,
            id,
            operation.request_id().ok_or_else(Error::storage)?,
            effect,
            fulfilled,
        )?;
    }
    if kind == "message" {
        let mut statement = c
            .prepare("SELECT id FROM provider_jobs WHERE message_id=?1")
            .map_err(|_| Error::storage())?;
        let ids = statement
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        for id in ids {
            link(c, effect, "provider_job", &id)?;
        }
    }
    Ok(())
}
fn link(c: &Connection, effect: &str, kind: &str, id: &str) -> Result<()> {
    c.execute(
        "INSERT INTO task_effect_links(effect_id,kind,resource_id) VALUES(?1,?2,?3)",
        params![effect, kind, id],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(super) fn state(c: &Connection, effect: &str, store: &Store) -> Result<EffectState> {
    if let Some(state) = crate::media_end::observation(c, effect)? {
        return Ok(state);
    }
    let mut statement = c.prepare("SELECT j.state,r.connector FROM task_effect_links l JOIN provider_jobs j ON j.id=l.resource_id JOIN connector_routes r ON r.id=j.route_id WHERE l.effect_id=?1 AND l.kind='provider_job'").map_err(|_|Error::storage())?;
    let jobs = statement
        .query_map([effect], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !jobs.is_empty() {
        if jobs
            .iter()
            .any(|(s, _)| matches!(s.as_str(), "uncertain" | "sending"))
        {
            return Ok(EffectState::Uncertain);
        }
        if jobs
            .iter()
            .any(|(s, _)| matches!(s.as_str(), "failed" | "cancelled" | "blocked"))
        {
            return Ok(EffectState::Failed);
        }
        let complete = jobs.iter().all(|(s, connector)| {
            s == "delivered"
                || (s == "completed"
                    && store
                        .connector(connector)
                        .is_ok_and(|descriptor| descriptor.conversational))
                || (s == "accepted"
                    && store
                        .connector(connector)
                        .is_ok_and(|descriptor| !descriptor.sms))
        });
        return Ok(if complete {
            EffectState::Succeeded
        } else {
            EffectState::Dispatched
        });
    }
    for (kind, table) in [("call_end", "voice_calls"), ("sip_end", "sip_legs")] {
        let sql=format!("SELECT v.state FROM task_effect_links l JOIN {table} v ON v.id=l.resource_id WHERE l.effect_id=?1 AND l.kind=?2");
        let state: Option<String> = c
            .query_row(&sql, params![effect, kind], |r| r.get(0))
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some(state) = state {
            return Ok(match state.as_str() {
                "ended" | "cancelled" => EffectState::Succeeded,
                "failed" | "forbidden" => EffectState::Failed,
                "uncertain" => EffectState::Uncertain,
                _ => EffectState::Dispatched,
            });
        }
    }
    // Voice/SIP start acceptance is distinct from a completed conversation leg.
    let call:Option<(String,Option<String>)> = c.query_row("SELECT v.state,v.provider_id FROM task_effect_links l JOIN voice_calls v ON v.id=l.resource_id WHERE l.effect_id=?1 AND l.kind='call'",[effect],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    if let Some((state, provider)) = call {
        return Ok(if provider.is_some() {
            EffectState::Succeeded
        } else if matches!(
            state.as_str(),
            "failed" | "cancelled" | "ended" | "forbidden"
        ) {
            EffectState::Failed
        } else if state == "uncertain" {
            EffectState::Uncertain
        } else {
            EffectState::Dispatched
        });
    }
    let sip:Option<(String,Option<String>)> = c.query_row("SELECT v.state,v.connection_id FROM task_effect_links l JOIN sip_legs v ON v.id=l.resource_id WHERE l.effect_id=?1 AND l.kind='sip_leg'",[effect],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    if let Some((state, connection)) = sip {
        return Ok(if connection.is_some() {
            EffectState::Succeeded
        } else if matches!(state.as_str(), "failed" | "cancelled" | "ended") {
            EffectState::Failed
        } else if state == "uncertain" {
            EffectState::Uncertain
        } else {
            EffectState::Dispatched
        });
    }
    Ok(EffectState::Succeeded)
}
use rusqlite::OptionalExtension;
