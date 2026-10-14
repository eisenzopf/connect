//! Trusted connector observations commit with durable effects and current policy.
use crate::{
    conversations::authorized_conversation, identity::actor_from_connection, providers::route,
    ProviderJob, Route, Store,
};
use connect_connector_api::{host_context::HostContext, lifecycle::*};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

fn invalid() -> Error {
    Error::new("invalid-request", "Invalid connector observation")
}
fn conflict() -> Error {
    Error::new("conflict", "Connector observation requires reconciliation")
}
fn identifier(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}
fn authorize_route(c: &Connection, route: &Route) -> Result<()> {
    let delegate = actor_from_connection(c, &route.credential_id)?;
    delegate.require("send")?;
    if !route.enabled || authorized_conversation(c, &delegate, &route.conversation_id)?.closed {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn authorized_job(c: &Connection, id: &str, connector: &str) -> Result<(ProviderJob, String)> {
    let (rid, body, credential, state): (String,String,String,String) = c.query_row(
        "SELECT j.route_id,m.body,j.credential_id,j.state FROM provider_jobs j JOIN messages m ON m.id=j.message_id WHERE j.id=?1", [id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).map_err(|_| Error::forbidden())?;
    let r = route(c, &rid)?;
    if r.connector != connector {
        return Err(Error::forbidden());
    }
    authorize_route(c, &r)?;
    let actor = actor_from_connection(c, &credential)?;
    actor.require("send")?;
    if authorized_conversation(c, &actor, &r.conversation_id)?.closed {
        return Err(Error::forbidden());
    }
    // This admits an observation/communication about an already dispatched
    // effect, not another task mutation. Task continuation is still checked at
    // claim/dispatch. provider_task_reply verifies the original effect/question
    // binding and retains terminal-task answers as unassigned communication.
    let context = c
        .query_row(
            "SELECT selection_json FROM context_deliveries WHERE id=?1",
            [format!("provider-job:{id}")],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    Ok((
        ProviderJob {
            id: id.into(),
            route: r,
            body,
            previous_chat_id: None,
            context,
        },
        state,
    ))
}
impl Store {
    /// Protected read admission before asking the provider about an existing effect.
    /// A terminal task cannot authorize a new send, but does not erase an owned
    /// remote effect or the recipient's current communication authority.
    /// Does not change the job to pending/sending or create another remote effect.
    pub fn connector_reconciliation_job(&self, id: &str, connector: &str) -> Result<ProviderJob> {
        let c = self.connection()?;
        let (job, state) = authorized_job(&c, id, connector)?;
        if !["uncertain", "accepted", "completed", "delivered", "failed"].contains(&state.as_str())
        {
            return Err(conflict());
        }
        Ok(job)
    }
    /// Apply an authenticated adapter receipt, callback or authoritative status.
    /// The caller is trusted composition, never wire input. Authorization precedes
    /// duplicate handling; reply, provenance and projection share one transaction.
    pub fn apply_connector_receipt(
        &self,
        connector: &str,
        receipt: &MessageReceipt,
    ) -> Result<Apply> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (job, current) = authorized_job(&tx, &receipt.observation.mutation_id, connector)?;
        let State::Message(observed_state) = receipt.observation.state else {
            return Err(invalid());
        };
        let binding: Option<String> = tx
            .query_row(
                "SELECT provider_id FROM provider_jobs WHERE id=?1",
                [&job.id],
                |row| row.get(0),
            )
            .map_err(|_| Error::storage())?;
        if binding.is_some()
            && receipt.observation.external_id.is_some()
            && binding != receipt.observation.external_id
        {
            return Err(invalid());
        }
        if ![
            "sending",
            "uncertain",
            "accepted",
            "completed",
            "delivered",
            "failed",
        ]
        .contains(&current.as_str())
        {
            return Err(conflict());
        }
        if receipt.observation.verification == Verification::LocalAttempt
            && !matches!(
                observed_state,
                MessageState::Failed | MessageState::Uncertain
            )
        {
            return Err(invalid());
        }
        // A delivery-only observation tracks an already dispatched effect. STOP
        // or lease expiry cannot erase that evidence or authorize another send.
        // A reply can admit communication/fanout, so it still requires current
        // SMS policy before any journal effect (including replay handling).
        if self.connector(connector)?.sms && receipt.reply.is_some() {
            let source = job.route.source.as_deref().unwrap_or_default();
            let allowed = match crate::tasks::task_sms_decision(
                &tx,
                &job.id,
                &job.route.target,
                source,
                &job.route.conversation_id,
            )? {
                Some(decision) => decision,
                None => crate::sms_eligibility::eligible(
                    &tx,
                    &job.route.target,
                    source,
                    &job.route.conversation_id,
                )?,
            };
            if allowed.is_err() {
                return Err(Error::forbidden());
            }
        }
        if receipt
            .reply
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > self.limits.message_bytes)
            || (receipt.reply.is_none() && !receipt.reply_sources.is_empty())
            || (receipt.reply.is_some()
                && matches!(
                    observed_state,
                    MessageState::Failed | MessageState::Uncertain
                ))
        {
            return Err(invalid());
        }
        if !receipt.reply_sources.is_empty() {
            let credential: String = tx
                .query_row(
                    "SELECT credential_id FROM provider_jobs WHERE id=?1",
                    [&job.id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            actor_from_connection(&tx, &credential)?.require("context_view")?;
            actor_from_connection(&tx, &job.route.credential_id)?.require("context_view")?;
            let selected =
                HostContext::decode(job.context.as_deref().ok_or_else(invalid)?, 1024 * 1024)
                    .and_then(|v| v.messaging_items())
                    .map_err(|_| invalid())?;
            let mut used = std::collections::BTreeSet::new();
            if receipt
                .reply_sources
                .iter()
                .any(|id| !used.insert(id) || !selected.iter().any(|item| &item.event_id == id))
            {
                return Err(invalid());
            }
        }
        let encoded = serde_json::to_string(receipt).map_err(|_| Error::storage())?;
        let old: Option<(String,bool,String)> = tx.query_row("SELECT projection_json,reply_committed,receipt_json FROM connector_observations WHERE job_id=?1", [&job.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| Error::storage())?;
        let mut projection = match &old {
            Some((json, _, _)) => serde_json::from_str(json).map_err(|_| Error::storage())?,
            None => Projection::new(
                job.id.clone(),
                State::Message(if current == "uncertain" {
                    MessageState::Uncertain
                } else {
                    MessageState::Accepted
                }),
            )
            .map_err(|_| invalid())?,
        };
        if projection.external_id.is_none() {
            projection.external_id = binding;
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT receipt_json FROM connector_receipt_events WHERE job_id=?1 AND event_id=?2",
                params![job.id, receipt.observation.event_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if prior.as_ref().is_some_and(|json| json != &encoded) {
            return Err(conflict());
        }
        let applied = projection
            .apply(receipt.observation.clone(), true)
            .map_err(|_| conflict())?;
        if applied == Apply::Duplicate {
            return Ok(applied);
        }
        let mut reply_committed = old.as_ref().is_some_and(|(_, done, _)| *done);
        if !reply_committed
            && matches!(
                projection.state,
                State::Message(MessageState::Accepted | MessageState::Delivered)
            )
        {
            if let Some(body) = &receipt.reply {
                let actor = actor_from_connection(&tx, &job.route.credential_id)?;
                if crate::tasks::provider_task_reply(&tx, &actor, &job.id, body, self)?.is_none() {
                    self.insert_provider_message(&tx, &job.route, body)?;
                }
                reply_committed = true;
            }
        }
        let state = match projection.state {
            State::Message(MessageState::Accepted | MessageState::Delivered) if reply_committed => {
                "completed"
            }
            State::Message(MessageState::Accepted) => "accepted",
            State::Message(MessageState::Delivered) => "delivered",
            State::Message(MessageState::Failed) => "failed",
            _ => "uncertain",
        };
        if applied != Apply::Older
            && state == "completed"
            && self.connector(connector)?.conversational
        {
            let external = projection.external_id.as_deref().ok_or_else(invalid)?;
            // Receipt order is not Conversation turn order. A late observation
            // or reconciliation of an older turn may retain its own answer and
            // provenance, but cannot replace a newer completed turn's chat.
            tx.execute("INSERT INTO provider_context(route_id,chat_id) SELECT ?1,?2 WHERE NOT EXISTS(SELECT 1 FROM provider_jobs newer JOIN provider_jobs current ON current.id=?3 WHERE newer.route_id=?1 AND newer.sequence>current.sequence AND (newer.state='completed' OR newer.provider_id=(SELECT chat_id FROM provider_context WHERE route_id=?1))) ON CONFLICT(route_id) DO UPDATE SET chat_id=excluded.chat_id", params![job.route.id,external,job.id]).map_err(|_| Error::storage())?;
        }
        tx.execute(
            "UPDATE provider_jobs SET state=?2,provider_id=?3 WHERE id=?1",
            params![job.id, state, projection.external_id],
        )
        .map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE context_deliveries SET state=?2,provider_ref=?3,updated_at=?4 WHERE id=?1",
            params![
                format!("provider-job:{}", job.id),
                if ["completed", "delivered"].contains(&state) {
                    "delivered"
                } else if state == "failed" {
                    "failed"
                } else {
                    "uncertain"
                },
                projection.external_id,
                crate::now()
            ],
        )
        .map_err(|_| Error::storage())?;
        if ["completed", "delivered"].contains(&state) {
            tx.execute(
                "UPDATE provider_jobs SET state='pending' WHERE route_id=?1 AND state='blocked'",
                [&job.route.id],
            )
            .map_err(|_| Error::storage())?;
        }
        let latest = if applied == Apply::Older
            && (receipt.reply.is_none() || old.as_ref().is_some_and(|(_, done, _)| *done))
        {
            old.as_ref()
                .map(|(_, _, json)| json.as_str())
                .unwrap_or(&encoded)
        } else {
            &encoded
        };
        tx.execute("INSERT INTO connector_observations VALUES(?1,?2,?3,?4) ON CONFLICT(job_id) DO UPDATE SET projection_json=excluded.projection_json,receipt_json=excluded.receipt_json,reply_committed=excluded.reply_committed", params![job.id,serde_json::to_string(&projection).map_err(|_| Error::storage())?,latest,reply_committed]).map_err(|_| Error::storage())?;
        tx.execute(
            "INSERT INTO connector_receipt_events VALUES(?1,?2,?3)",
            params![job.id, receipt.observation.event_id, encoded],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(applied)
    }
    /// Protected ingress after adapter authentication, mapped to a configured route.
    /// SMS continues through its existing signature/STOP/START policy boundary.
    pub fn receive_connector_message(
        &self,
        connector: &str,
        route_id: &str,
        message: &IncomingMessage,
    ) -> Result<String> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let r = route(&tx, route_id)?;
        if r.connector != connector || self.connector(connector)?.sms {
            return Err(Error::forbidden());
        }
        authorize_route(&tx, &r)?;
        if !identifier(&message.mutation_id)
            || !identifier(&message.external_id)
            || !matches!(
                message.verification,
                Verification::AuthenticatedResponse | Verification::VerifiedCallback
            )
        {
            return Err(invalid());
        }
        let encoded = serde_json::to_string(message).map_err(|_| Error::storage())?;
        let prior: Option<(String,String)> = tx.query_row("SELECT message_json,message_id FROM connector_incoming WHERE route_id=?1 AND mutation_id=?2", params![route_id,message.mutation_id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| Error::storage())?;
        if let Some((old, id)) = prior {
            return if old == encoded {
                Ok(id)
            } else {
                Err(conflict())
            };
        }
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM connector_incoming WHERE route_id=?1 AND json_extract(message_json,'$.external_id')=?2)", params![route_id,message.external_id], |r| r.get(0)).map_err(|_| Error::storage())?;
        if bound {
            return Err(conflict());
        }
        let count: u64 = tx
            .query_row("SELECT COUNT(*) FROM connector_incoming", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if count >= self.limits.operation_receipts {
            return Err(Error::new(
                "capacity-exceeded",
                "Connector incoming receipt capacity exceeded",
            ));
        }
        let id = self.insert_provider_message(&tx, &r, &message.body)?;
        tx.execute(
            "INSERT INTO connector_incoming VALUES(?1,?2,?3,?4)",
            params![route_id, message.mutation_id, encoded, id],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(id)
    }
    pub fn connector_receipt(&self, id: &str, connector: &str) -> Result<MessageReceipt> {
        let c = self.connection()?;
        authorized_job(&c, id, connector)?;
        let json: String = c
            .query_row(
                "SELECT receipt_json FROM connector_observations WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        serde_json::from_str(&json).map_err(|_| Error::storage())
    }
}
