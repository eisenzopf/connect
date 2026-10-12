//! Independent submissions become durable app tickets in their source conversation.
use super::{domain::digest, state::AppStore, workflow, Draft};
use anyhow::{ensure, Result};
use connect_client::{Artifact, ConnectClient, Event};
use rusqlite::OptionalExtension;

pub(super) async fn receive(
    store: &mut AppStore,
    client: &mut ConnectClient,
    event: &Event,
    request_id: String,
    draft: Draft,
    artifact: Option<Artifact>,
) -> Result<()> {
    let sender = &event
        .message
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing submission message"))?
        .sender;
    let key = format!(
        "intake:{}",
        digest(&(sender, &event.conversation_id, &request_id))?
    );
    let mut payload = serde_json::json!({
        "sender":sender,
        "request_id":request_id,
        "draft":draft,
        "status":"received",
    });
    if let Some(artifact) = artifact {
        payload["artifact"] = serde_json::to_value(artifact)?;
    }
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
        sender,
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

/// A task may have been created before a crash even when its artifact later
/// disappears. Reconcile that original intent before recording an unavailable
/// source; do not strand acceptance uncertainty or create a replacement task.
pub(super) async fn recover_pending_task(
    store: &mut AppStore,
    client: &mut ConnectClient,
    event: &Event,
    request_id: &str,
) -> Result<()> {
    let sender = &event
        .message
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing submission message"))?
        .sender;
    let key = format!(
        "task:intake:{}",
        digest(&(sender, &event.conversation_id, request_id))?
    );
    let pending: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_commands WHERE key=?1 AND outcome IS NULL AND rejection IS NULL)",
        [&key], |row| row.get(0),
    )?;
    if pending {
        workflow::create_task(
            store,
            client,
            &key,
            &event.conversation_id,
            sender,
            Some(event.id.clone()),
        )
        .await?;
    }
    Ok(())
}
