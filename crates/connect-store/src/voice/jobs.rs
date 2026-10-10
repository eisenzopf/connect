//! Claimed voice work rechecks current account and protected task authority.
use super::{event, get, route, VoiceAction, VoiceJob};
use crate::{conversations::authorized_conversation, identity::actor_from_connection, Store};
use connect_protocol::{Error, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
type VoiceClaim = (String, String, String, u8, Option<String>);
impl Store {
    /// Claim one pending mutation or observation. Known authorization denial
    /// is persisted without dispatch; infrastructure errors leave it unclaimed.
    pub fn claim_voice_job(&self) -> Result<Option<VoiceJob>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<VoiceClaim>=tx.query_row("SELECT id,conversation_id,credential_id,end_requested,control_url FROM voice_calls WHERE state='pending' OR (provider_id IS NOT NULL AND state NOT IN ('ended','failed','cancelled','forbidden','starting','ending') AND next_poll<=?1) ORDER BY end_requested DESC,sequence LIMIT 1",[crate::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_| Error::storage())?;
        let Some((id, conv, credential, end_requested, control_url)) = row else {
            return Ok(None);
        };
        let mut call = get(&tx, &conv, &id)?;
        let r = route(&tx, &call.route_id)?;
        let mut end_intent_id = None;
        let action = if call.state == "pending" {
            let authorized = actor_from_connection(&tx, &credential).and_then(|a| {
                a.require("call_start")?;
                crate::tasks::authorize_effect_link(&tx, "call", &id, crate::now())?;
                authorized_conversation(&tx, &a, &conv)
            });
            let authorized = crate::media_end::permitted(authorized)?;
            if !r.enabled || !authorized.is_some_and(|c| !c.closed) {
                call.state = "forbidden".into();
                tx.execute(
                    "UPDATE voice_calls SET state='forbidden' WHERE id=?1",
                    [&id],
                )
                .map_err(|_| Error::storage())?;
                event(&tx, &call, self)?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            }
            call.state = "starting".into();
            VoiceAction::Start
        } else if end_requested == 1 {
            let authorized =
                crate::media_end::authorize(&tx, crate::media_end::Kind::Call, &id, crate::now())
                    .and_then(|(actor, intent)| {
                        authorized_conversation(&tx, &actor, &conv)?;
                        Ok(intent)
                    });
            let Some(intent) = crate::media_end::permitted(authorized)? else {
                crate::media_end::blocked(&tx, crate::media_end::Kind::Call, &id)?;
                tx.execute("UPDATE voice_calls SET end_requested=0,end_credential_id=NULL,next_poll=0 WHERE id=?1",[&id]).map_err(|_|Error::storage())?;
                event(&tx, &get(&tx, &conv, &id)?, self)?;
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(None);
            };
            crate::media_end::claimed(&tx, &intent)?;
            end_intent_id = Some(intent);
            tx.execute("UPDATE voice_calls SET end_requested=2 WHERE id=?1", [&id])
                .map_err(|_| Error::storage())?;
            call.state = "ending".into();
            VoiceAction::End
        } else {
            VoiceAction::Poll
        };
        tx.execute(
            "UPDATE voice_calls SET state=?2,next_poll=?3 WHERE id=?1",
            params![id, call.state, crate::now() + 2],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(VoiceJob {
            call,
            route: r,
            action,
            control_url,
            end_intent_id,
        }))
    }
}
