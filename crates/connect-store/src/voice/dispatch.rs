//! Final physical admission is separate from claim and remote observation.
use super::{event, get, route, VoiceAction, VoiceJob};
use crate::{
    conversations::authorized_conversation, identity::actor_from_connection, media_end, Store,
};
use connect_protocol::{Error, Result};
use rusqlite::{params, TransactionBehavior};

impl Store {
    /// Recheck a claimed mutation immediately before calling the provider. A
    /// repeated admission cannot resubmit a possibly sent request. Polling is
    /// observation and continues without borrowing a revoked task's authority.
    pub(super) fn admit_voice_dispatch(&self, job: &VoiceJob) -> Result<bool> {
        if job.action == VoiceAction::Poll {
            let c = self.connection()?;
            binding(&c, job)?;
            if job.call.provider_id.is_none()
                || matches!(
                    job.call.state.as_str(),
                    "pending"
                        | "starting"
                        | "ending"
                        | "ended"
                        | "failed"
                        | "cancelled"
                        | "forbidden"
                )
            {
                return Err(Error::forbidden());
            }
            return Ok(true);
        }
        let authority = self.runtime_query_authority(
            crate::authority_admission::runtime::VOICE_CALL,
            &[&job.call.id],
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(
            &tx,
            &authority,
            crate::authority_admission::runtime::VOICE_CALL,
            &[&job.call.id],
        )?;
        let mut call = get(&tx, &job.call.conversation_id, &job.call.id)?;
        let expected = if job.action == VoiceAction::Start {
            "starting"
        } else {
            "ending"
        };
        if call.state != expected {
            return Ok(false);
        }
        binding(&tx, job)?;
        if job.action == VoiceAction::Start {
            let allowed = (|| {
                let credential: String = tx
                    .query_row(
                        "SELECT credential_id FROM voice_calls WHERE id=?1",
                        [&call.id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                let actor = actor_from_connection(&tx, &credential)?;
                actor.require("call_start")?;
                crate::audience::require_original(
                    &tx,
                    &actor,
                    &call.conversation_id,
                    connect_protocol::membership::ContentKind::Call,
                    &call.id,
                )?;
                let target = route(&tx, &call.route_id)?.identity_id;
                if !crate::audience::original_to_identity(
                    &tx,
                    &target,
                    &call.conversation_id,
                    connect_protocol::membership::ContentKind::Call,
                    &call.id,
                )? {
                    return Err(Error::forbidden());
                }
                crate::tasks::authorize_effect_link(&tx, "call", &call.id, crate::now())?;
                let conversation = authorized_conversation(&tx, &actor, &call.conversation_id)?;
                Ok(!conversation.closed
                    && route(&tx, &call.route_id)?.enabled
                    && !call.end_requested)
            })();
            if !media_end::permitted(allowed)?.unwrap_or(false) {
                call.state = if call.end_requested {
                    "cancelled"
                } else {
                    "forbidden"
                }
                .into();
                tx.execute(
                    "UPDATE voice_calls SET state=?1 WHERE id=?2",
                    params![call.state, call.id],
                )
                .map_err(|_| Error::storage())?;
                event(&tx, &call, self)?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(false);
            }
            let admitted = media_end::admit_start(&tx, "call", &call.id)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(admitted);
        }
        let intent = job.end_intent_id.as_deref().ok_or_else(Error::forbidden)?;
        let allowed = media_end::authorize_generation(
            &tx,
            media_end::Kind::Call,
            &call.id,
            crate::now(),
            Some(intent),
        )
        .and_then(|(actor, _)| {
            crate::audience::require_original(
                &tx,
                &actor,
                &call.conversation_id,
                connect_protocol::membership::ContentKind::Call,
                &call.id,
            )?;
            authorized_conversation(&tx, &actor, &call.conversation_id)
        });
        if media_end::permitted(allowed)?.is_none() {
            if let Some(current) = media_end::reject_claim(&tx, intent)? {
                tx.execute("UPDATE voice_calls SET state='uncertain',end_requested=CASE WHEN ?2 THEN 0 ELSE end_requested END,next_poll=0 WHERE id=?1",params![call.id,current]).map_err(|_|Error::storage())?;
                event(&tx, &get(&tx, &call.conversation_id, &call.id)?, self)?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(false);
        }
        media_end::dispatching(&tx, intent)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

pub(super) fn binding(c: &rusqlite::Connection, job: &VoiceJob) -> Result<()> {
    let call = get(c, &job.call.conversation_id, &job.call.id)?;
    let control: Option<String> = c
        .query_row(
            "SELECT control_url FROM voice_calls WHERE id=?1",
            [&call.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut current_route = route(c, &call.route_id)?;
    // Enable/disable is current policy, not a new provider address binding.
    current_route.enabled = job.route.enabled;
    if call != job.call || current_route != job.route || control != job.control_url {
        return Err(Error::forbidden());
    }
    Ok(())
}
