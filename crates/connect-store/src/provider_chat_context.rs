//! Chat dispatch shares the durable context projection used by voice.
use crate::{identity::actor_from_connection, Route};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

/// Opaque remote memory is reusable only under the same local policy. Capture
/// this at dispatch, never on a later callback that could follow a revocation.
pub(crate) fn authority_policy(
    store: &crate::Store,
    c: &Connection,
    credential: &str,
    route: &Route,
) -> Result<String> {
    let owner = actor_from_connection(c, credential)?;
    let delegate = actor_from_connection(c, &route.credential_id)?;
    let owner_member =
        crate::audience::member(c, &owner.identity_id, &route.conversation_id, true)?;
    let peer_member =
        crate::audience::member(c, &delegate.identity_id, &route.conversation_id, true)?;
    let revision: u64 = c
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            [&route.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    // Membership revisions alone do not change when a record is redacted or a
    // source crosses retention. Bind reusable remote memory to the actual
    // authorized record projection, including its visible retention boundary.
    // Ordinary new messages do not alter this policy.
    let mut records = std::collections::BTreeMap::new();
    if owner.permits("context_view")
        && owner.permits("record_get")
        && delegate.permits("context_view")
        && delegate.permits("record_get")
    {
        let current = crate::context::view_for_scope(
            store,
            c,
            &owner,
            &route.conversation_id,
            1000,
            Some(&delegate.identity_id),
            Some(&delegate),
        )?;
        for reference in current.items.into_iter().filter_map(|item| item.record) {
            records.insert(reference.uuid.clone(), reference);
        }
    }
    let policy = serde_json::to_vec(&(
        "provider-audience/2",
        revision,
        owner.identity_id,
        owner_member.membership_epoch,
        owner.scope,
        delegate.identity_id,
        peer_member.membership_epoch,
        delegate.scope,
        records,
    ))
    .map_err(|_| Error::storage())?;
    Ok(hex::encode(Sha256::digest(policy)))
}

impl crate::Store {
    /// Acknowledge a definite conversational connector rejection without resubmitting the job.
    /// Unknown remote outcomes cannot be discarded through this operation.
    pub fn discard_failed_chat_job(&self, operator: &crate::Actor, job_id: &str) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let authority = actor_from_connection(&tx, &operator.credential_id)?;
        authority.require("admin")?;
        let (conversation, message): (String,String) = tx.query_row(
            "SELECT r.conversation_id,j.message_id FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id
             WHERE j.id=?1 AND j.state='failed' AND j.provider_id IS NULL",
            [job_id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).map_err(|_|Error::forbidden())?;
        let connector: String = tx.query_row("SELECT r.connector FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id WHERE j.id=?1", [job_id], |r| r.get(0)).map_err(|_| Error::storage())?;
        if !self.connector(&connector)?.conversational {
            return Err(Error::forbidden());
        }
        let members = {
            let mut statement = tx
                .prepare("SELECT identity_id FROM participants WHERE conversation_id=?1")
                .map_err(|_| Error::storage())?;
            let members = statement
                .query_map([&conversation], |row| row.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            members
        };
        if members.is_empty()
            || members
                .iter()
                .any(|member| !authority.permits_recipient(member))
        {
            return Err(Error::forbidden());
        }
        if !crate::conversations::capacity(&tx, members.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable event capacity exceeded",
            ));
        }
        tx.execute(
            "UPDATE provider_jobs SET state='cancelled' WHERE id=?1",
            [job_id],
        )
        .map_err(|_| Error::storage())?;
        for member in members {
            crate::conversations::insert_event(
                &tx,
                &member,
                &conversation,
                "provider_job_discarded",
                Some(&message),
            )?;
        }
        // The context-delivery row retains its failed state and exact selection.
        tx.commit().map_err(|_| Error::storage())
    }
}

pub(crate) fn begin(
    connection: &Connection,
    credential_id: &str,
    route: &Route,
    job_id: &str,
    conversational: bool,
    store: &crate::Store,
) -> Result<(Option<String>, bool)> {
    if !conversational {
        return Ok((None, false));
    }
    let delegate = actor_from_connection(connection, &route.credential_id)?;
    let context = crate::tasks::provider_task_context(connection, job_id, store)?;
    let task_origin = !matches!(context, crate::tasks::ProviderTaskContext::Ordinary);
    // No context permission is a definitive ceiling; never fall back to owner history.
    if !delegate.permits("context_view") {
        return Ok((None, task_origin));
    }
    let (serialized, watermark) = match context {
        crate::tasks::ProviderTaskContext::Restricted => return Ok((None, true)),
        crate::tasks::ProviderTaskContext::Selected {
            json,
            watermark,
            actor: owner,
        } => (
            export_task_selection(
                store,
                &owner,
                connection,
                &route.conversation_id,
                &json,
                &delegate.identity_id,
                Some(&delegate),
            )?,
            watermark,
        ),
        crate::tasks::ProviderTaskContext::Ordinary => {
            let actor = actor_from_connection(connection, credential_id)?;
            if !actor.permits("context_view") {
                return Ok((None, false));
            }
            let selection = crate::context::view_for_scope(
                store,
                connection,
                &actor,
                &route.conversation_id,
                1000,
                Some(&delegate.identity_id),
                Some(&delegate),
            )?;
            (
                serde_json::to_string(&selection).map_err(|_| Error::storage())?,
                selection.watermark,
            )
        }
    };
    let id = format!("provider-job:{job_id}");
    let timestamp = crate::now();
    connection.execute(
        "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id)
         VALUES(?1,?2,NULL,?6,?3,?4,'pending',?5,?5,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),(SELECT identity_id FROM credentials WHERE id=?7))",
        params![id,route.conversation_id,watermark,serialized,timestamp,route.connector,credential_id],
    ).map_err(|_|Error::storage())?;
    Ok((Some(serialized), task_origin))
}

pub(crate) fn finish(
    connection: &Connection,
    job_id: &str,
    state: &str,
    provider_ref: Option<&str>,
) -> Result<()> {
    let outcome = match state {
        "completed" => "delivered",
        "failed" => "failed",
        _ => "uncertain",
    };
    connection.execute(
        "UPDATE context_deliveries SET state=?2,provider_ref=?3,updated_at=?4 WHERE id=?1 AND state='pending'",
        params![format!("provider-job:{job_id}"),outcome,provider_ref,crate::now()],
    ).map_err(|_|Error::storage())?;
    Ok(())
}

/// A model receives only source rows its current participant epoch can read.
/// Cursors and counts belong to the caller's projection, so never cross this boundary.
pub(crate) fn export_task_selection(
    store: &crate::Store,
    owner: &crate::Actor,
    c: &Connection,
    conversation: &str,
    json: &str,
    identity: &str,
    actor: Option<&crate::Actor>,
) -> Result<String> {
    let selection: connect_protocol::tasks::TaskOutcome =
        serde_json::from_str(json).map_err(|_| Error::storage())?;
    let connect_protocol::tasks::TaskOutcome::Recall { sources, .. } = selection else {
        return Err(Error::storage());
    };
    let mut filtered = vec![];
    let mut budget = 32 * 1024 * 1024;
    for mut source in sources {
        let kind = crate::audience::context_kind(&source.kind);
        let visible = if let Some(actor) = actor {
            crate::audience::visible(c, actor, conversation, kind, &source.source_id)?
        } else {
            crate::audience::visible_to_identity(
                c,
                identity,
                conversation,
                kind,
                &source.source_id,
            )?
        };
        let private:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM private_content_sources WHERE kind=?1 AND source_id=?2 AND conversation_id=?3)",params![kind.as_str(),source.source_id,conversation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if visible && !private && actor.is_none_or(|a| a.permits("context_view")) {
            if kind == connect_protocol::membership::ContentKind::Record {
                if !owner.permits("record_get") || actor.is_some_and(|a| !a.permits("record_get")) {
                    continue;
                }
                let member = crate::audience::member(c, identity, conversation, true)?;
                let data = match crate::records::context::select(
                    store,
                    c,
                    owner,
                    conversation,
                    &source.source_id,
                    actor,
                    Some(&[(
                        identity.into(),
                        member.participant_id,
                        member.membership_epoch,
                    )]),
                    &mut budget,
                ) {
                    Ok(data) => data,
                    Err(error) if error.code == "forbidden" => continue,
                    Err(error) if error.code == "capacity-exceeded" => break,
                    Err(error) => return Err(error),
                };
                source.state = match data.reference.state {
                    connect_protocol::ContextRecordState::Available => "available",
                    connect_protocol::ContextRecordState::Deleted => "deleted",
                    _ => "unavailable",
                }
                .into();
                source.body = (source.state == "available").then_some(data.body);
                source.record = Some(data.reference);
                source.actor_id = data.owner;
            }
            filtered.push(source);
        }
    }
    Ok(
        serde_json::json!({"kind":"recall","data":{"sources":filtered,"next":null,"omitted":null}})
            .to_string(),
    )
}
