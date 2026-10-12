//! Durable outbound SIP intent. Network effects are claimed after commit.

use crate::{conversations::authorized_conversation, now, Actor, Store};
use connect_protocol::{Error, Result, Session, SipLeg};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
mod dispatch;
mod termination;
pub use termination::SipConnectionEnd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SipLegAction {
    Start,
    End,
}

#[derive(Clone, Debug)]
pub struct SipLegJob {
    pub leg: SipLeg,
    pub credential_id: String,
    pub actor_id: String,
    pub action: SipLegAction,
    /// Exact claimed end generation, distinct from the dialer's authority.
    pub end_intent_id: Option<String>,
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SipLeg> {
    Ok(SipLeg {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        session_id: row.get(2)?,
        destination: row.get(3)?,
        state: row.get(4)?,
        connection_id: row.get(5)?,
        ended_reason: row.get(6)?,
        end_requested: row.get::<_, u8>(7)? > 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn get(c: &Connection, conversation_id: &str, id: &str) -> Result<SipLeg> {
    c.query_row(
        "SELECT id,conversation_id,session_id,destination,state,connection_id,ended_reason,end_requested,created_at,updated_at FROM sip_legs WHERE id=?1 AND conversation_id=?2",
        params![id, conversation_id],
        row,
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}

pub(crate) fn status(c: &Connection, conversation_id: &str, id: &str) -> Result<SipLeg> {
    get(c, conversation_id, id)
}

pub(crate) fn dial(
    c: &Connection,
    actor: &Actor,
    credential_id: &str,
    conversation_id: &str,
    session_id: &str,
    destination: &str,
) -> Result<SipLeg> {
    let session = crate::sessions::get(c, conversation_id, session_id)?;
    require_dial_authority(actor, &session)?;
    if !valid_destination(destination) {
        return Err(Error::new(
            "invalid-request",
            "SIP destination must be a bounded sip: URI",
        ));
    }
    let timestamp = now();
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO sip_legs(id,conversation_id,session_id,credential_id,actor_id,destination,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
        params![id, conversation_id, session_id, credential_id, actor.identity_id, destination, timestamp],
    )
    .map_err(|_| Error::storage())?;
    get(c, conversation_id, &id)
}

pub(crate) fn end(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    id: &str,
    request_id: &str,
) -> Result<SipLeg> {
    let leg = get(c, conversation_id, id)?;
    let session = crate::sessions::get(c, conversation_id, &leg.session_id)?;
    if actor.identity_id != session.initiator_id && actor.identity_id != session.participant_id {
        return Err(Error::forbidden());
    }
    if matches!(leg.state.as_str(), "ended" | "failed" | "cancelled") {
        return Ok(leg);
    }
    let (state, requested) = if leg.state == "pending" {
        ("cancelled", 0)
    } else {
        (leg.state.as_str(), 1)
    };
    c.execute(
        "UPDATE sip_legs SET state=?1,end_requested=?2,updated_at=?3 WHERE id=?4",
        params![state, requested, now(), id],
    )
    .map_err(|_| Error::storage())?;
    crate::media_end::record(
        c,
        crate::media_end::Kind::Sip,
        id,
        &actor.credential_id,
        request_id,
        requested == 1,
    )?;
    get(c, conversation_id, id)
}

fn require_dial_authority(actor: &Actor, session: &Session) -> Result<()> {
    if actor.identity_id != session.initiator_id
        || !matches!(session.state.as_str(), "accepted" | "ready")
        || !session.media.iter().any(|value| value == "audio")
    {
        return Err(Error::forbidden());
    }
    Ok(())
}

fn valid_destination(value: &str) -> bool {
    (5..=256).contains(&value.len())
        && value.starts_with("sip:")
        && value.bytes().all(|byte| byte.is_ascii_graphic())
        && !value.contains(['<', '>', '\\', '"'])
}

impl Store {
    pub fn recover_sip_legs(&self) -> Result<usize> {
        self.connection()?
            .execute(
                "UPDATE sip_legs SET state=CASE WHEN state='active' THEN 'failed' ELSE 'uncertain' END,ended_reason='server-restarted',end_requested=0,updated_at=?1 WHERE state IN ('starting','active','ending')",
                [now()],
            )
            .map_err(|_| Error::storage())
    }

    pub fn claim_sip_leg_job(&self) -> Result<Option<SipLegJob>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let claimed: Option<(String, String, String, String)> = tx
            .query_row(
                "SELECT id,conversation_id,credential_id,actor_id FROM sip_legs WHERE state='pending' OR (state='active' AND end_requested=1) ORDER BY sequence LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let Some((id, conversation_id, mut credential_id, mut actor_id)) = claimed else {
            return Ok(None);
        };
        let mut leg = get(&tx, &conversation_id, &id)?;
        let mut end_intent_id = None;
        let action = if leg.state == "pending" {
            let allowed = (|| {
                let actor = crate::identity::actor_from_connection(&tx, &credential_id)?;
                actor.require("sip_dial")?;
                crate::tasks::authorize_effect_link(&tx, "sip_leg", &id, now())?;
                if actor.identity_id != actor_id {
                    return Err(Error::forbidden());
                }
                authorized_conversation(&tx, &actor, &conversation_id)?;
                let session = crate::sessions::get(&tx, &conversation_id, &leg.session_id)?;
                require_dial_authority(&actor, &session)
            })();
            if crate::media_end::permitted(allowed)?.is_none() {
                tx.execute(
                    "UPDATE sip_legs SET state='failed',ended_reason='authority-revoked',updated_at=?1 WHERE id=?2",
                    params![now(), id],
                )
                .map_err(|_| Error::storage())?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            SipLegAction::Start
        } else {
            let authorized =
                crate::media_end::authorize(&tx, crate::media_end::Kind::Sip, &id, now()).and_then(
                    |(actor, intent)| {
                        authorized_conversation(&tx, &actor, &conversation_id)?;
                        let session = crate::sessions::get(&tx, &conversation_id, &leg.session_id)?;
                        if actor.identity_id != session.initiator_id
                            && actor.identity_id != session.participant_id
                        {
                            return Err(Error::forbidden());
                        }
                        Ok((actor, intent))
                    },
                );
            let Some((actor, intent)) = crate::media_end::permitted(authorized)? else {
                crate::media_end::blocked(&tx, crate::media_end::Kind::Sip, &id)?;
                tx.execute(
                    "UPDATE sip_legs SET end_requested=0,updated_at=?1 WHERE id=?2",
                    params![now(), id],
                )
                .map_err(|_| Error::storage())?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            };
            crate::media_end::claimed(&tx, &intent)?;
            end_intent_id = Some(intent);
            credential_id = actor.credential_id;
            actor_id = actor.identity_id;
            SipLegAction::End
        };
        let state = if action == SipLegAction::Start {
            "starting"
        } else {
            "ending"
        };
        tx.execute(
            "UPDATE sip_legs SET state=?1,updated_at=?2 WHERE id=?3",
            params![state, now(), id],
        )
        .map_err(|_| Error::storage())?;
        leg = get(&tx, &conversation_id, &id)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(SipLegJob {
            leg,
            credential_id,
            actor_id,
            action,
            end_intent_id,
        }))
    }

    pub fn prepare_sip_leg(&self, job: &SipLegJob, connection_id: &str) -> Result<()> {
        let changed = self.connection()?.execute(
            "UPDATE sip_legs SET connection_id=?1,updated_at=?2 WHERE id=?3 AND state='starting' AND connection_id IS NULL",
            params![connection_id, now(), job.leg.id],
        ).map_err(|_| Error::storage())?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::new("conflict", "SIP leg is no longer starting"))
        }
    }

    /// Return whether the owner requested termination while an outbound SIP
    /// activation is still waiting for a final response.
    pub fn sip_leg_end_requested(&self, id: &str) -> Result<bool> {
        self.connection()?
            .query_row(
                "SELECT end_requested FROM sip_legs WHERE id=?1",
                [id],
                |row| row.get::<_, u8>(0).map(|value| value > 0),
            )
            .optional()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::forbidden)
    }

    /// Persist a worker result without replacing an authoritative terminal
    /// callback that raced with activation or cleanup. In particular, a late
    /// cancellation cannot erase a failure and activation cannot reopen an end.
    pub fn finish_sip_leg_job(
        &self,
        job: &SipLegJob,
        state: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        if !matches!(
            state,
            "active" | "ended" | "failed" | "uncertain" | "cancelled"
        ) {
            return Err(Error::new("invalid-request", "Invalid SIP leg state"));
        }
        let clear_end_request = u8::from(state != "active");
        self.connection()?
            .execute(
                "UPDATE sip_legs SET state=?1,ended_reason=?2,end_requested=CASE WHEN ?3=1 THEN 0 ELSE end_requested END,updated_at=?4 WHERE id=?5 AND state NOT IN ('ended','failed','cancelled')",
                params![state, reason, clear_end_request, now(), job.leg.id],
            )
            .map_err(|_| Error::storage())?;
        Ok(())
    }
}
