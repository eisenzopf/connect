//! Trusted app task creation and conversation-linked communication intents.
use super::{
    ledger::{execute, id},
    state::AppStore,
};
use anyhow::{ensure, Result};
use connect_client::{tasks::*, ConnectClient, Operation, Outcome};

pub(super) fn command(command: TaskCommand) -> Operation {
    Operation::Task {
        request: Box::new(TaskRequest::new(command)),
    }
}
pub(super) fn record(outcome: Outcome) -> Result<TaskRecord> {
    match outcome {
        Outcome::Task(result) => match *result {
            TaskOutcome::Record(task) => Ok(*task),
            _ => anyhow::bail!("Expected task record"),
        },
        _ => anyhow::bail!("Expected task outcome"),
    }
}
pub(super) async fn get(client: &mut ConnectClient, task_id: &str) -> Result<TaskRecord> {
    let task = match client
        .task(TaskCommand::Get {
            task_id: task_id.into(),
        })
        .await?
    {
        TaskOutcome::Record(task) => *task,
        _ => anyhow::bail!("Expected current task"),
    };
    Ok(task)
}
pub(super) async fn current(client: &mut ConnectClient, task_id: &str) -> Result<TaskRecord> {
    let task = get(client, task_id).await?;
    ensure!(
        !task.state.is_terminal() && task.expires_at > chrono::Utc::now().timestamp(),
        "Task ended; app work stays retained"
    );
    Ok(task)
}
pub(super) async fn observe(
    store: &mut AppStore,
    client: &mut ConnectClient,
    task_id: &str,
) -> Result<TaskRecord> {
    let task = get(client, task_id).await?;
    store.save("outgoing-state", &serde_json::to_string(&task.state)?)?;
    store.save("outgoing-task-id", &task.id)?;
    store.save("outgoing-conversation-id", &task.conversation_id)?;
    if task.state.is_terminal() {
        store.connection.execute(
            "UPDATE app_commands SET effect_state='closed' WHERE key='outgoing-question'",
            [],
        )?;
    }
    Ok(task)
}
pub(super) async fn create_task(
    store: &mut AppStore,
    client: &mut ConnectClient,
    key: &str,
    cid: &str,
    recipient: &str,
    event: Option<String>,
) -> Result<TaskRecord> {
    let task = record(
        execute(
            store,
            client,
            key,
            command(TaskCommand::Create {
                request_id: id(),
                definition: TaskDefinition {
                    conversation_id: cid.into(),
                    initiating_event_id: event,
                    expires_at: chrono::Utc::now().timestamp() + 600,
                    scope: TaskScope {
                        operations: vec![
                            TaskPermission::Send,
                            TaskPermission::History,
                            TaskPermission::Inbox,
                            TaskPermission::RequestStatus,
                            TaskPermission::ArtifactBegin,
                            TaskPermission::ArtifactAppend,
                            TaskPermission::ArtifactFinish,
                            TaskPermission::ArtifactGet,
                        ],
                        recipients: vec![recipient.into()],
                        sms_enrollments: vec![],
                    },
                },
            }),
        )
        .await?,
    )?;
    let live = current(client, &task.id).await?;
    ensure!(
        live.conversation_id == cid && live.scope.recipients == vec![recipient.to_string()],
        "Task binding changed"
    );
    Ok(live)
}
pub(super) fn send(
    task: &str,
    cid: &str,
    body: String,
    reply: Option<String>,
    wait: bool,
) -> Operation {
    let request_id = id();
    command(TaskCommand::Invoke {
        request_id: request_id.clone(),
        task_id: task.into(),
        step_id: format!("app-{request_id}"),
        operation: Box::new(Operation::Send {
            request_id,
            conversation_id: cid.into(),
            message: connect_client::MessageDraft::text_all(body),
        }),
        reply_to: reply,
        wait_for_reply: wait,
    })
}
/// Resolve an accepted effect without creating a replacement mutation.
pub(super) async fn message(
    client: &mut ConnectClient,
    outcome: Outcome,
) -> Result<Option<String>> {
    tool_outcome(client, outcome)
        .await?
        .map(accepted_message)
        .transpose()
}
/// Reconcile the original effect; pending does not imply successful execution.
pub(super) async fn tool_outcome(
    client: &mut ConnectClient,
    outcome: Outcome,
) -> Result<Option<Outcome>> {
    let Outcome::Task(result) = outcome else {
        anyhow::bail!("Expected task result")
    };
    let result = match *result {
        TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) => return Ok(Some(*outcome)),
        TaskOutcome::Tool(TaskToolResult::Pending {
            task_id,
            effect_id: Some(effect_id),
        })
        | TaskOutcome::Tool(TaskToolResult::Uncertain { task_id, effect_id }) => {
            client
                .task(TaskCommand::EffectGet { task_id, effect_id })
                .await?
        }
        TaskOutcome::Effect(effect) => TaskOutcome::Effect(effect),
        TaskOutcome::Tool(TaskToolResult::Pending { .. }) => return Ok(None),
        _ => anyhow::bail!("Unexpected tool receipt"),
    };
    let TaskOutcome::Effect(effect) = result else {
        anyhow::bail!("Expected effect status")
    };
    match effect.state {
        EffectState::Succeeded => {
            Ok(Some(*effect.outcome.ok_or_else(|| {
                anyhow::anyhow!("Succeeded effect has no outcome")
            })?))
        }
        EffectState::Failed | EffectState::Cancelled => {
            anyhow::bail!("App effect failed or cancelled; retained for inspection")
        }
        _ => Ok(None),
    }
}
pub(super) async fn settle(
    store: &mut AppStore,
    client: &mut ConnectClient,
    key: &str,
    outcome: Outcome,
) -> Result<Option<String>> {
    let waiting = key == "outgoing-question"
        && matches!(&outcome,Outcome::Task(result) if matches!(**result,TaskOutcome::Tool(TaskToolResult::Pending {..})));
    let uncertain = matches!(&outcome,Outcome::Task(result) if matches!(**result,TaskOutcome::Tool(TaskToolResult::Uncertain {..})));
    let message = message(client, outcome).await?;
    let status = if message.is_some() && waiting {
        "waiting"
    } else if message.is_some() {
        "completed"
    } else if uncertain {
        "uncertain"
    } else {
        "pending"
    };
    store.connection.execute(
        "UPDATE app_commands SET effect_state=?1 WHERE key=?2",
        [status, key],
    )?;
    Ok(message)
}
fn accepted_message(outcome: Outcome) -> Result<String> {
    match outcome {
        Outcome::Accepted { message_id, .. } => Ok(message_id),
        _ => anyhow::bail!("Expected message receipt"),
    }
}
