//! Decisions require explicit server reply binding, not text or sender inference.
use super::{
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
            super::intake::receive(store, client, event, request_id, draft, None).await
        }
        Request::SubmitArtifact {
            request_id,
            artifact,
        } => {
            super::intake::recover_pending_task(store, client, event, &request_id).await?;
            let content = super::artifacts::receive(
                client,
                &event.conversation_id,
                &message.sender,
                &artifact,
            )
            .await;
            match content {
                Ok((draft, source)) => {
                    super::intake::receive(store, client, event, request_id, draft, Some(source))
                        .await
                }
                Err(error) => store.finish_event(&event.id, super::artifacts::rejection(error)?),
            }
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
            let artifact = store
                .metadata("outgoing-artifact")?
                .map(|json| serde_json::from_str::<connect_client::Artifact>(&json))
                .transpose()?;
            if let Some(ref artifact) = artifact {
                let draft = match super::artifacts::verify_bytes(client, artifact).await {
                    Ok(draft) => draft,
                    Err(error) => {
                        return store.finish_event(&event.id, super::artifacts::rejection(error)?)
                    }
                };
                ensure!(
                    draft == policy.draft,
                    "Reviewed artifact no longer matches the configured draft"
                );
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
            let mut payload = serde_json::json!({
                "draft":policy.draft,
                "draft_id":draft_id,
                "decision":decision,
                "note":note,
            });
            if let Some(artifact) = artifact {
                payload["artifact"] = serde_json::to_value(artifact)?;
            }
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
