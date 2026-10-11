//! Decisions require explicit server reply binding, not text or sender inference.
use super::{
    domain::digest,
    ledger::{execute, id},
    state::AppStore,
    workflow::{self, command},
    Policy, Request,
};
use anyhow::{ensure, Result};
use connect_client::{tasks::*, ClientError, ConnectClient, Event, Outcome};

pub(super) async fn handle(
    store: &mut AppStore,
    client: &mut ConnectClient,
    policy: &Policy,
    outgoing: &TaskRecord,
    event: &Event,
    own_id: &str,
) -> Result<()> {
    let Some(message) = event.message.as_ref() else {
        return store.finish_event(&event.id, "ignored");
    };
    if message.sender == own_id {
        return store.finish_event(&event.id, "ignored");
    }
    if !policy.allowed_contacts.contains(&message.sender) {
        return store.finish_event(&event.id, "denied");
    }
    if message.body.len() > 8192 {
        return store.finish_event(&event.id, "invalid");
    }
    let request = match serde_json::from_str::<Request>(&message.body) {
        Ok(r) => r,
        Err(_) => return store.finish_event(&event.id, "ignored"),
    };
    if request.validate().is_err() {
        return store.finish_event(&event.id, "invalid");
    }
    ensure!(
        message.conversation_id == event.conversation_id,
        "Message journal conversation mismatch"
    );
    match request {
        Request::SubmitDraft { request_id, draft } => {
            let key = format!(
                "intake:{}",
                digest(&(&message.sender, &event.conversation_id, &request_id))?
            );
            let payload = serde_json::json!({
                "sender":message.sender,
                "request_id":request_id,
                "draft":draft,
                "status":"received",
            });
            let existing: Option<(String, String)> = store
                .connection
                .query_row(
                    "SELECT task_id,input_digest FROM app_business WHERE key=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((task_id, previous)) = existing {
                let live = workflow::get(client, &task_id).await?;
                ensure!(
                    live.conversation_id == event.conversation_id,
                    "Intake conversation changed"
                );
                let result = if previous == digest(&payload)? {
                    "duplicate"
                } else {
                    "conflict"
                };
                return store.finish_event(&event.id, result);
            }
            let task = workflow::create_task(
                store,
                client,
                &format!("task:{key}"),
                &event.conversation_id,
                &message.sender,
                Some(event.id.clone()),
            )
            .await?;
            let response = serde_json::json!({
                "action":"draft_received",
                "request_id":request_id,
                "ticket_id":key,
                "digest":draft.digest()?,
                "status":"received",
            })
            .to_string();
            let reply = workflow::send(&task.id, &event.conversation_id, response, None, false);
            store.business(&key, "intake", &task.id, event, payload, reply)
        }
        Request::ReviewDecision {
            draft_id,
            decision,
            note,
        } => {
            if message.sender != policy.reviewer_id
                || event.conversation_id != outgoing.conversation_id
                || draft_id != policy.draft.id
            {
                return store.finish_event(&event.id, "unassigned");
            }
            if workflow::get(client, &outgoing.id)
                .await?
                .state
                .is_terminal()
            {
                return store.finish_event(&event.id, "closed");
            }
            let question = store
                .metadata("question")?
                .ok_or_else(|| anyhow::anyhow!("Question binding unavailable"))?;
            if !bound(client, outgoing, event, &question).await? {
                return store.finish_event(&event.id, "unassigned");
            }
            consume(store, client, &outgoing.id, &event.id).await?;
            workflow::current(client, &outgoing.id).await?;
            let response = serde_json::json!({
                "action":"review_recorded",
                "draft_id":draft_id,
                "decision":decision,
                "digest":policy.draft.digest()?,
            })
            .to_string();
            let payload = serde_json::json!({
                "draft":policy.draft,
                "draft_id":draft_id,
                "decision":decision,
                "note":note,
            });
            let reply = workflow::send(
                &outgoing.id,
                &event.conversation_id,
                response,
                Some(event.id.clone()),
                false,
            );
            store.business(
                "outgoing-review",
                "review",
                &outgoing.id,
                event,
                payload,
                reply,
            )
        }
    }
}
async fn bound(
    client: &mut ConnectClient,
    task: &TaskRecord,
    event: &Event,
    question: &str,
) -> Result<bool> {
    let mut after = None;
    // Explicit bound metadata may be shorter than requested. Never infer a match.
    for _ in 0..10 {
        let TaskOutcome::Replies { items, next } = client
            .task(TaskCommand::Replies {
                conversation_id: event.conversation_id.clone(),
                after: after.clone(),
                limit: 100,
            })
            .await?
        else {
            anyhow::bail!("Expected replies page")
        };
        if let Some(reply) = items.iter().find(|r| r.event_id == event.id) {
            return Ok(reply.task_id.as_deref() == Some(&task.id)
                && reply.conversation_id == event.conversation_id
                && reply.reply_to.as_deref() == Some(question));
        }
        if next.is_none() {
            return Ok(false);
        }
        ensure!(next != after, "Replies cursor did not progress");
        after = next;
    }
    anyhow::bail!("Reply lookup exceeds bounded app window; retain page for inspection")
}
async fn consume(
    store: &mut AppStore,
    client: &mut ConnectClient,
    task: &str,
    event: &str,
) -> Result<()> {
    let pointer = format!("consume-pointer:{event}");
    let mut key = store
        .metadata(&pointer)?
        .unwrap_or_else(|| format!("consume:{event}"));
    store.save(&pointer, &key)?;
    for _ in 0..3 {
        let live = workflow::current(client, task).await?;
        let rejected: Option<String> = store
            .connection
            .query_row(
                "SELECT rejection FROM app_commands WHERE key=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if rejected.as_deref() == Some("task-state-conflict") {
            key = format!("consume:{event}:revision:{}", live.revision);
            store.save(&pointer, &key)?;
        }
        let operation = command(TaskCommand::ConsumeReply {
            request_id: id(),
            task_id: task.into(),
            event_id: event.into(),
            expected_revision: live.revision,
        });
        match execute(store, client, &key, operation).await {
            Ok(Outcome::Task(result)) if matches!(*result, TaskOutcome::Record(_)) => return Ok(()),
            Err(error) if matches!(error.downcast_ref::<ClientError>(),Some(ClientError::Application(e)) if e.code=="task-state-conflict") => {
                continue
            }
            Err(error) => return Err(error),
            _ => anyhow::bail!("Expected consume receipt"),
        }
    }
    anyhow::bail!("Consume CAS contention; original and rejected intents retained")
}
use rusqlite::OptionalExtension;
