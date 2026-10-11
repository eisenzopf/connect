//! Restore the original outgoing task without reopening terminal work.
use super::{
    ledger::{execute, id},
    state::AppStore,
    workflow::{create_task, observe, record, send, settle},
    Policy,
};
use anyhow::{ensure, Result};
use connect_client::{tasks::TaskRecord, ConnectClient, Operation, Outcome};
pub(super) async fn bootstrap(
    store: &mut AppStore,
    client: &mut ConnectClient,
    policy: &Policy,
) -> Result<Option<TaskRecord>> {
    let peer = client.resolve(&policy.reviewer_address).await?;
    ensure!(
        peer.id == policy.reviewer_id,
        "Reviewer address changed identity"
    );
    let receipt = if let Some(saved) = store.accepted("outgoing-conversation")? {
        saved
    } else {
        execute(
            store,
            client,
            "outgoing-conversation",
            Operation::Create {
                request_id: id(),
                participants: vec![peer.id.clone()],
            },
        )
        .await?
    };
    let cid = match receipt {
        Outcome::Conversation(c) => c.id,
        _ => anyhow::bail!("Expected conversation receipt"),
    };
    client
        .negotiate_tasks(&policy.reviewer_address, Some(&cid))
        .await?;
    let task = if let Some(saved) = store.accepted("outgoing-task")? {
        let original = record(saved)?;
        let live = observe(store, client, &original.id).await?;
        ensure!(
            live.owner_id == original.owner_id
                && live.conversation_id == cid
                && live.scope == original.scope,
            "Outgoing task binding changed"
        );
        live
    } else {
        create_task(store, client, "outgoing-task", &cid, &peer.id, None).await?
    };
    if task.state.is_terminal() {
        return Ok(Some(task));
    }
    let body = serde_json::json!({
        "action":"review_request",
        "draft":policy.draft,
        "digest":policy.draft.digest()?,
    })
    .to_string();
    let outcome = execute(
        store,
        client,
        "outgoing-question",
        send(&task.id, &cid, body, None, true),
    )
    .await?;
    if let Some(question) = settle(store, client, "outgoing-question", outcome).await? {
        store.save("question", &question)?;
        Ok(Some(task))
    } else {
        Ok(None)
    }
}
