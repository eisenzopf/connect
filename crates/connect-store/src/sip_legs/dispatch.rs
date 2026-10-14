//! SIP preparation can await; authorization must survive until wire admission.
use super::{get, permitted_dial, require_dial_authority, SipLegAction, SipLegJob};
use crate::{
    conversations::authorized_conversation, identity::actor_from_connection, media_end, Store,
};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection, TransactionBehavior};

fn allowed(c: &Connection, job: &SipLegJob) -> Result<bool> {
    let leg = get(c, &job.leg.conversation_id, &job.leg.id)?;
    if !matches!(leg.state.as_str(), "starting" | "active") || leg.end_requested {
        return Ok(false);
    }
    let actor = actor_from_connection(c, &job.credential_id)?;
    actor.require("sip_dial")?;
    if actor.identity_id != job.actor_id {
        return Err(Error::forbidden());
    }
    crate::tasks::authorize_effect_link(c, "sip_leg", &leg.id, crate::now())?;
    if authorized_conversation(c, &actor, &leg.conversation_id)?.closed {
        return Ok(false);
    }
    let session = crate::sessions::get(c, &leg.conversation_id, &leg.session_id)?;
    require_dial_authority(&actor, &session)?;
    Ok(true)
}
impl Store {
    /// Read-only cancellation observation while origination is already in
    /// flight. Cleanup must not misrepresent a possibly sent INVITE as unsent.
    pub fn sip_origination_cancelled(&self, job: &SipLegJob) -> Result<bool> {
        let c = self.connection()?;
        Ok(!permitted_dial(allowed(&c, job))?.unwrap_or(false))
    }
    /// Recheck at the final commit boundary. Records one possible physical
    /// dispatch; repeat admission cannot redial or send another hangup.
    pub fn sip_dispatch_allowed(&self, job: &SipLegJob) -> Result<bool> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let leg = get(&tx, &job.leg.conversation_id, &job.leg.id)?;
        if job.action == SipLegAction::Start {
            if leg.state != "starting" {
                return Ok(false);
            }
            let admission = allowed(&tx, job);
            let unsupported = admission
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "unsupported-media-profile");
            if !permitted_dial(admission)?.unwrap_or(false) {
                tx.execute("UPDATE sip_legs SET state='cancelled',ended_reason=?1,end_requested=0,updated_at=?2 WHERE id=?3 AND state='starting'",params![if unsupported { "unsupported-media-profile" } else { "authority-revoked-before-dispatch" },crate::now(),leg.id]).map_err(|_|Error::storage())?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(false);
            }
            let admitted = media_end::admit_start(&tx, "sip_leg", &leg.id)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(admitted);
        }
        if leg.state != "ending" {
            return Ok(false);
        }
        let intent = job.end_intent_id.as_deref().ok_or_else(Error::forbidden)?;
        let authorized = media_end::authorize_generation(
            &tx,
            media_end::Kind::Sip,
            &leg.id,
            crate::now(),
            Some(intent),
        )
        .and_then(|(actor, _)| {
            authorized_conversation(&tx, &actor, &leg.conversation_id)?;
            let session = crate::sessions::get(&tx, &leg.conversation_id, &leg.session_id)?;
            if actor.identity_id != session.initiator_id
                && actor.identity_id != session.participant_id
            {
                return Err(Error::forbidden());
            }
            Ok(())
        });
        if media_end::permitted(authorized)?.is_none() {
            if let Some(current) = media_end::reject_claim(&tx, intent)? {
                tx.execute("UPDATE sip_legs SET state='active',end_requested=CASE WHEN ?1 THEN 0 ELSE end_requested END,updated_at=?2 WHERE id=?3",params![current,crate::now(),leg.id]).map_err(|_|Error::storage())?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(false);
        }
        media_end::dispatching(&tx, intent)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
